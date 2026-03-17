use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use polars::prelude::*;

use super::header::HEADER_SIZE;
use super::lzf::{self, CompressionAlgorithm};
use crate::error::{Result, YxdbError};
use crate::field::{FieldMeta, FieldType};

/// Maximum uncompressed block size before flushing to disk.
/// Matches the reader's default buffer: 256 KiB.
const BLOCK_SIZE: usize = 0x40000; // 262144

/// Number of records per record-block-index entry.
/// Matches the Alteryx OpenYXDB constant `RecordsPerBlock`.
/// The reader uses this to build a seek index for random access.
const RECORDS_PER_BLOCK: usize = 0x10000; // 65536

// ── Public API ─────────────────────────────────────────────────────────

/// Write a Polars [`DataFrame`] to a YXDB file.
///
/// `spatial_columns` names Binary columns that contain WKB geometry data.
/// These will be written as `SpatialObj` fields with automatic WKB → SHP
/// conversion. Pass an empty slice (`&[]`) when no spatial columns are
/// present.
///
/// The schema is inferred from the DataFrame column types using sensible
/// defaults. Use [`write_yxdb_with_schema`] for explicit control.
pub fn write_yxdb<P: AsRef<Path>>(path: P, df: &DataFrame, spatial_columns: &[&str]) -> Result<()> {
    use crate::spatial;

    let fields = infer_schema(df, spatial_columns)?;

    if spatial_columns.is_empty() {
        write_yxdb_impl(path.as_ref(), df, &fields)
    } else {
        let df_shp = spatial::convert_spatial_columns_to_shp(df, &fields)?;
        write_yxdb_impl(path.as_ref(), &df_shp, &fields)
    }
}

/// Write a Polars [`DataFrame`] to a YXDB file using an explicit field schema.
pub fn write_yxdb_with_schema<P: AsRef<Path>>(
    path: P,
    df: &DataFrame,
    fields: &[FieldMeta],
) -> Result<()> {
    write_yxdb_impl(path.as_ref(), df, fields)
}

/// Write Arrow IPC bytes to a YXDB file.
///
/// This is the primary cross-language entry point: Python sends Arrow IPC
/// bytes, we deserialize to a DataFrame, then write the YXDB.
pub fn write_yxdb_from_ipc<P: AsRef<Path>>(path: P, ipc_bytes: &[u8]) -> Result<()> {
    write_yxdb_from_ipc_spatial(path, ipc_bytes, &[])
}

/// Like [`write_yxdb_from_ipc`] but with support for spatial columns.
///
/// `spatial_columns` names Binary columns containing WKB geometry data
/// that should be stored as `SpatialObj` fields in the YXDB.
pub fn write_yxdb_from_ipc_spatial<P: AsRef<Path>>(
    path: P,
    ipc_bytes: &[u8],
    spatial_columns: &[&str],
) -> Result<()> {
    let cursor = std::io::Cursor::new(ipc_bytes);
    let df = IpcReader::new(cursor)
        .finish()
        .map_err(|e| YxdbError::ConversionError(format!("failed to read IPC bytes: {e}")))?;
    write_yxdb(path, &df, spatial_columns)
}

// ── Streaming Writer ───────────────────────────────────────────────────

/// A streaming YXDB writer that supports writing data in batches.
///
/// This enables `sink_yxdb` functionality where data is written incrementally
/// without holding the entire dataset in memory. The record count in the
/// header is updated at finalization using file seeking.
///
/// # Example (Rust)
/// ```no_run
/// use sigilyx::YxdbWriter;
/// use polars::prelude::*;
///
/// fn example() -> Result<(), Box<dyn std::error::Error>> {
///     let df1 = DataFrame::default(); // batch 1
///     let df2 = DataFrame::default(); // batch 2
///
///     let mut writer = YxdbWriter::new("output.yxdb", &df1)?;
///     writer.write_batch(&df1)?;
///     writer.write_batch(&df2)?;
///     writer.finish()?;
///     Ok(())
/// }
/// ```
pub struct YxdbWriter<W: Write + Seek> {
    writer: W,
    fields: Vec<FieldMeta>,
    fixed_size: usize,
    has_var: bool,
    compression: CompressionAlgorithm,
    record_count: u64,
    block_buf: Vec<u8>,
    /// Current file position (tracked manually to avoid seek calls).
    file_pos: u64,
    /// File positions at each RECORDS_PER_BLOCK boundary, for the block index.
    block_index: Vec<i64>,
    // Reusable serialization buffers (avoid per-record allocation)
    ser_fixed: Vec<u8>,
    ser_var_data: Vec<u8>,
    ser_var_fixups: Vec<(usize, usize)>,
    ser_record: Vec<u8>,
    ser_utf16_buf: Vec<u8>,
    /// Path to the output file (set for file-backed writers only).
    /// Used to clean up partial files on Drop without finish().
    output_path: Option<PathBuf>,
    /// Set to true by finish() so Drop knows not to clean up.
    finished: bool,
}

impl YxdbWriter<BufWriter<File>> {
    /// Create a new YXDB writer for a file path.
    ///
    /// The schema is inferred from the provided template DataFrame.
    pub fn new<P: AsRef<Path>>(path: P, template_df: &DataFrame) -> Result<Self> {
        let fields = infer_schema(template_df, &[])?;
        Self::with_schema(path, &fields)
    }

    /// Create a new YXDB writer with an explicit schema.
    pub fn with_schema<P: AsRef<Path>>(path: P, fields: &[FieldMeta]) -> Result<Self> {
        Self::with_schema_and_compression(path, fields, CompressionAlgorithm::Lzf)
    }

    /// Create a new YXDB writer with explicit schema and compression algorithm.
    pub fn with_schema_and_compression<P: AsRef<Path>>(
        path: P,
        fields: &[FieldMeta],
        compression: CompressionAlgorithm,
    ) -> Result<Self> {
        let output_path = path.as_ref().to_path_buf();
        let file = File::create(path.as_ref())?;
        let mut writer = BufWriter::new(file);

        // Compute sizes
        let fixed_size: usize = fields
            .last()
            .map(|f| f.offset + f.field_type.fixed_bytes(f.size))
            .unwrap_or(0);
        let has_var = fields.iter().any(|f| f.field_type.is_variable());

        // Build XML metadata
        let xml = build_meta_xml(fields);
        let utf16_bytes = encode_utf16_le(&xml);
        let meta_info_size = (utf16_bytes.len() / 2) as u32;

        // Write placeholder header (record count = 0, will be updated in finish())
        let header = build_header(0, meta_info_size, compression);
        writer.write_all(&header)?;

        // Write UTF-16LE XML metadata
        writer.write_all(&utf16_bytes)?;

        let data_start = (HEADER_SIZE + utf16_bytes.len()) as u64;

        Ok(Self {
            writer,
            fields: fields.to_vec(),
            fixed_size,
            has_var,
            compression,
            record_count: 0,
            block_buf: Vec::with_capacity(BLOCK_SIZE),
            file_pos: data_start,
            block_index: vec![data_start as i64], // block 0 starts here
            ser_fixed: vec![0u8; fixed_size],
            ser_var_data: Vec::with_capacity(if has_var { 1024 } else { 0 }),
            ser_var_fixups: Vec::with_capacity(if has_var { fields.len() } else { 0 }),
            ser_record: Vec::with_capacity(fixed_size + if has_var { 1024 } else { 0 }),
            ser_utf16_buf: Vec::with_capacity(if has_var { 256 } else { 0 }),
            output_path: Some(output_path),
            finished: false,
        })
    }
}

impl<W: Write + Seek> YxdbWriter<W> {
    /// Write a batch of records to the YXDB file.
    ///
    /// The batch DataFrame must have the same schema as the template.
    pub fn write_batch(&mut self, batch: &DataFrame) -> Result<()> {
        let num_rows = batch.height();
        if num_rows == 0 {
            return Ok(());
        }

        // Validate that the batch has the expected number of columns.
        if batch.width() != self.fields.len() {
            return Err(YxdbError::ConversionError(format!(
                "batch has {} columns but writer schema has {} fields",
                batch.width(),
                self.fields.len()
            )));
        }

        // Build column references
        let columns: Vec<&Column> = batch.columns().iter().collect();

        for row in 0..num_rows {
            // At every RECORDS_PER_BLOCK boundary, force-flush and record position
            let global_row = self.record_count as usize + row;
            if global_row > 0 && global_row % RECORDS_PER_BLOCK == 0 {
                if !self.block_buf.is_empty() {
                    self.flush_block()?;
                }
                self.block_index.push(self.file_pos as i64);
            }

            build_record_into(
                &mut self.ser_fixed,
                &mut self.ser_var_data,
                &mut self.ser_var_fixups,
                &mut self.ser_record,
                &mut self.ser_utf16_buf,
                &self.fields,
                &columns,
                self.fixed_size,
                self.has_var,
                row,
            )?;

            // Check if adding this record would exceed block size
            if self.block_buf.len() + self.ser_record.len() > BLOCK_SIZE
                && !self.block_buf.is_empty()
            {
                self.flush_block()?;
            }

            // Append record data, splitting across block boundaries if needed.
            let mut offset = 0;
            let rec_len = self.ser_record.len();
            while offset < rec_len {
                let space = BLOCK_SIZE - self.block_buf.len();
                let chunk = (rec_len - offset).min(space);
                self.block_buf
                    .extend_from_slice(&self.ser_record[offset..offset + chunk]);
                offset += chunk;
                if self.block_buf.len() >= BLOCK_SIZE && offset < rec_len {
                    self.flush_block()?;
                }
            }
        }

        self.record_count += num_rows as u64;
        Ok(())
    }

    /// Flush the current block to disk.
    fn flush_block(&mut self) -> Result<()> {
        if self.block_buf.is_empty() {
            return Ok(());
        }

        // Try to compress using the chosen algorithm
        if let Some(compressed) = lzf::compress_block(self.compression, &self.block_buf) {
            // Write compressed block length (without high bit)
            let len = compressed.len() as u32;
            self.writer.write_all(&len.to_le_bytes())?;
            self.writer.write_all(&compressed)?;
            self.file_pos += 4 + compressed.len() as u64;
        } else {
            // Write uncompressed block length (with high bit set)
            let len = (self.block_buf.len() as u32) | 0x80000000;
            self.writer.write_all(&len.to_le_bytes())?;
            self.writer.write_all(&self.block_buf)?;
            self.file_pos += 4 + self.block_buf.len() as u64;
        }

        self.block_buf.clear();
        Ok(())
    }

    /// Finish writing and update the header with the final record count.
    ///
    /// This **must** be called to produce a valid YXDB file. Dropping the
    /// writer without calling `finish()` will emit a warning and leave the
    /// file with an incorrect record count in the header.
    #[must_use = "call .finish() to write a valid YXDB — dropping the writer without finishing produces a corrupt file"]
    pub fn finish(mut self) -> Result<()> {
        self.finished = true;

        // Flush any remaining data
        self.flush_block()?;

        // Write the RecordBlockIndex at current position
        let record_block_index_pos = self.file_pos as i64;
        let num_blocks = self.block_index.len() as u32;
        self.writer.write_all(&num_blocks.to_le_bytes())?;
        for pos in &self.block_index {
            self.writer.write_all(&pos.to_le_bytes())?;
        }

        // Seek back to header and update:
        // - nRecordBlockIndexPos at offset 96 (i64 LE)
        self.writer.seek(SeekFrom::Start(96))?;
        self.writer
            .write_all(&record_block_index_pos.to_le_bytes())?;

        // - num_records at offset 104 (u64 LE)
        self.writer.seek(SeekFrom::Start(104))?;
        self.writer.write_all(&self.record_count.to_le_bytes())?;

        self.writer.flush()?;
        Ok(())
    }

    /// Get the current record count.
    pub fn record_count(&self) -> u64 {
        self.record_count
    }
}

impl<W: Write + Seek> Drop for YxdbWriter<W> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        eprintln!(
            "warning: YxdbWriter dropped without calling finish() — \
             the output file is incomplete and will be removed"
        );
        // Best-effort removal of the incomplete file.
        // On Unix this works even with the file handle still open.
        // On Windows the file handle is still held by self.writer so
        // remove_file will fail; the file is left behind but has an
        // invalid header (record count = 0) making it obviously corrupt.
        if let Some(ref path) = self.output_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

// ── Schema inference ───────────────────────────────────────────────────

/// Infer a YXDB field schema from a Polars DataFrame.
pub fn infer_schema(df: &DataFrame, spatial_columns: &[&str]) -> Result<Vec<FieldMeta>> {
    let mut fields = Vec::with_capacity(df.width());
    let mut offset = 0;

    for col in df.columns() {
        let name: String = col.name().to_string();
        let dtype = col.dtype();

        let (field_type, size, scale) = match dtype {
            DataType::Boolean => (FieldType::Bool, 1, 0),
            DataType::UInt8 => (FieldType::Byte, 1, 0),
            DataType::Int8 => (FieldType::Int16, 2, 0), // Int8 has sign — Byte is unsigned
            DataType::Int16 => (FieldType::Int16, 2, 0),
            DataType::UInt16 => (FieldType::Int32, 4, 0), // UInt16 max 65535 overflows Int16
            DataType::Int32 => (FieldType::Int32, 4, 0),
            DataType::UInt32 => (FieldType::Int64, 8, 0), // UInt32 max 4B overflows Int32
            DataType::Int64 | DataType::UInt64 => (FieldType::Int64, 8, 0),
            DataType::Float32 => (FieldType::Float, 4, 0),
            DataType::Float64 => (FieldType::Double, 8, 0),
            DataType::String => {
                // Pick V_String (1 byte/char) when all values are ASCII/Latin-1,
                // otherwise V_WString (UTF-16LE, 2 bytes/char). V_String produces
                // ~2x smaller variable data for ASCII-only columns.
                let needs_wide = column_needs_wide_string(col);
                let max_len = column_max_string_size(col, needs_wide);
                if needs_wide {
                    (FieldType::VWString, max_len, 0)
                } else {
                    (FieldType::VString, max_len, 0)
                }
            }
            DataType::Date => (FieldType::Date, 10, 0),
            DataType::Datetime(_, _) => (FieldType::DateTime, 19, 0),
            DataType::Time => (FieldType::Time, 8, 0),
            DataType::Duration(_) => (FieldType::Int64, 8, 0), // Store as microseconds
            DataType::Binary => {
                if spatial_columns.contains(&name.as_str()) {
                    (FieldType::SpatialObj, 0, 0)
                } else {
                    (FieldType::Blob, 0, 0)
                }
            }
            DataType::Decimal(precision, scale) => {
                let p = *precision;
                let s = *scale;
                // YXDB FixedDecimal `size` is the total ASCII character width,
                // which must fit: sign(1) + integer digits(p-s) + decimal point
                // (1 if s>0) + fraction digits(s) = p + 1 + (1 if s>0).
                let size = p + 1 + if s > 0 { 1 } else { 0 };
                (FieldType::FixedDecimal, size, s)
            }
            _ => {
                return Err(YxdbError::ConversionError(format!(
                    "unsupported Polars dtype for YXDB write: {dtype} (column: {name})"
                )));
            }
        };

        let current_offset = offset;
        offset += field_type.fixed_bytes(size);

        fields.push(FieldMeta {
            name,
            field_type,
            size,
            scale,
            offset: current_offset,
        });
    }

    Ok(fields)
}

/// Check whether a string column contains any non-Latin-1 characters.
///
/// Returns `true` if V_WString (UTF-16LE) is needed, `false` if V_String
/// (single-byte Latin-1) is sufficient. V_String stores each character as
/// one byte, halving the variable-data size for ASCII/Latin-1 content
/// compared to V_WString's UTF-16LE encoding.
fn column_needs_wide_string(col: &Column) -> bool {
    let Ok(ca) = col.str() else {
        return true; // fall back to wide if we can't inspect
    };
    for s in ca.iter().flatten() {
        // Latin-1 covers U+0000..U+00FF (single-byte range).
        // Any character above U+00FF requires UTF-16LE (V_WString).
        if s.chars().any(|c| c as u32 > 0xFF) {
            return true;
        }
    }
    false
}

/// Compute the `size` (declared max length) for a string column.
///
/// For V_String the size is the max byte length across all values.
/// For V_WString the size is the max UTF-16 code-unit count.
/// Returns at least 1 (for columns with only empty strings or nulls).
fn column_max_string_size(col: &Column, wide: bool) -> usize {
    let Ok(ca) = col.str() else {
        return 256; // fallback
    };
    let mut max_len: usize = 0;
    for s in ca.iter().flatten() {
        let len = if wide {
            s.encode_utf16().count()
        } else {
            s.len() // byte length for Latin-1/ASCII
        };
        if len > max_len {
            max_len = len;
        }
    }
    max_len.max(1)
}

// ── Core writer ────────────────────────────────────────────────────────

pub(crate) fn write_yxdb_impl(path: &Path, df: &DataFrame, fields: &[FieldMeta]) -> Result<()> {
    let num_records = df.height() as u64;

    // Build the XML metadata string
    let xml = build_meta_xml(fields);

    // Encode XML as UTF-16LE with null terminator
    let utf16_bytes = encode_utf16_le(&xml);
    // meta_info_size = count of UTF-16 code units (including null terminator)
    let meta_info_size = (utf16_bytes.len() / 2) as u32;

    // Build the 512-byte header (nRecordBlockIndexPos will be patched later)
    let header = build_header(num_records, meta_info_size, CompressionAlgorithm::Lzf);

    // Open output file
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    // Write header
    writer.write_all(&header)?;

    // Write UTF-16LE XML metadata (including null terminator)
    writer.write_all(&utf16_bytes)?;

    // Compute fixed record size
    let fixed_size: usize = fields
        .last()
        .map(|f| f.offset + f.field_type.fixed_bytes(f.size))
        .unwrap_or(0);
    let has_var = fields.iter().any(|f| f.field_type.is_variable());

    let data_start = (HEADER_SIZE + utf16_bytes.len()) as u64;

    // Serialize records into LZF-compressed blocks, collecting block index
    let (block_index, end_pos) = write_records(
        &mut writer,
        df,
        fields,
        fixed_size,
        has_var,
        num_records,
        data_start,
        CompressionAlgorithm::Lzf,
    )?;

    // Write RecordBlockIndex at current position
    let record_block_index_pos = end_pos as i64;
    let num_blocks = block_index.len() as u32;
    writer.write_all(&num_blocks.to_le_bytes())?;
    for pos in &block_index {
        writer.write_all(&pos.to_le_bytes())?;
    }

    // Seek back to header and write nRecordBlockIndexPos at offset 96
    writer.seek(SeekFrom::Start(96))?;
    writer.write_all(&record_block_index_pos.to_le_bytes())?;

    writer.flush()?;
    Ok(())
}

// ── Header construction ────────────────────────────────────────────────

// The YXDB format uses "Alteryx Database File" as a magic identifier (like PNG or PDF magic bytes).
// This is the minimum required for format identification and interoperability.
// We intentionally do NOT include the copyright notice from Alteryx's implementation.
const MAGIC: &[u8] = b"Alteryx Database File";

fn build_header(
    num_records: u64,
    meta_info_size: u32,
    compression: CompressionAlgorithm,
) -> [u8; HEADER_SIZE] {
    let mut header = [0u8; HEADER_SIZE];

    // Magic string at offset 0 (required for format identification)
    // Only the 21-byte identifier, not any copyright text
    header[..MAGIC.len()].copy_from_slice(MAGIC);

    // Bytes 64-67: File version/type identifier (int32 LE)
    // Must be 0x00440204 (ID_WRIGLEYDB_NoSpatialIndex) for compatibility
    // with the official Alteryx OpenYXDB library, which checks byte 66 == 0x44.
    let file_id: u32 = 0x00440204;
    header[64..68].copy_from_slice(&file_id.to_le_bytes());

    // meta_info_size at offset 80 (u32 LE)
    header[80..84].copy_from_slice(&meta_info_size.to_le_bytes());

    // Record info area (bytes 84..104 typically contain spatial index info)
    // We leave them zero for non-spatial files

    // num_records at offset 104 (u64 LE)
    header[104..112].copy_from_slice(&num_records.to_le_bytes());

    // nCompressionVersion at offset 112 (i32 LE)
    // 1 = LZF (standard Alteryx compression)
    let compression_version: i32 = compression.version_id();
    header[112..116].copy_from_slice(&compression_version.to_le_bytes());

    header
}

// ── XML metadata ───────────────────────────────────────────────────────

fn build_meta_xml(fields: &[FieldMeta]) -> String {
    use std::fmt::Write;
    let mut xml = String::with_capacity(256);
    // Match the reference Alteryx OpenYXDB format (no leading newline,
    // alphabetical attribute ordering, no size for fixed-size types).
    xml.push_str("<RecordInfo>\n");

    for field in fields {
        xml.push_str("\t<Field name=\"");
        xml_escape_into(&field.name, &mut xml);
        xml.push('"');

        // Attributes in alphabetical order (matching Alteryx convention):
        //   name, scale (FixedDecimal only), size (variable/string types),
        //   source, type

        match field.field_type {
            FieldType::FixedDecimal => {
                let _ = write!(xml, " scale=\"{}\" size=\"{}\"", field.scale, field.size);
            }
            FieldType::String | FieldType::WString | FieldType::VString | FieldType::VWString => {
                let _ = write!(xml, " size=\"{}\"", field.size);
            }
            FieldType::Blob | FieldType::SpatialObj => {
                if field.size > 0 {
                    let _ = write!(xml, " size=\"{}\"", field.size);
                }
            }
            // Fixed-size types: Bool, Byte, Int16, Int32, Int64, Float,
            // Double, Date, Time, DateTime — Alteryx omits size for these.
            _ => {}
        }

        xml.push_str(" source=\"SigilYX\" type=\"");
        xml.push_str(field.field_type.as_xml_str());
        xml.push_str("\"/>\n");
    }

    xml.push_str("</RecordInfo>\n");
    xml
}

fn xml_escape_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
}

// ── UTF-16LE encoding ──────────────────────────────────────────────────

fn encode_utf16_le(s: &str) -> Vec<u8> {
    let code_units: Vec<u16> = s.encode_utf16().collect();
    let mut bytes = Vec::with_capacity((code_units.len() + 1) * 2);
    for cu in &code_units {
        bytes.extend_from_slice(&cu.to_le_bytes());
    }
    // Null terminator
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

// ── Record serialization ───────────────────────────────────────────────

/// A compressed block ready to be written to disk.
struct CompressedBlock {
    /// The serialized bytes (4-byte length header + payload).
    data: Vec<u8>,
}

/// Compress a raw block into a serialized compressed block.
///
/// This runs on the background compression thread. It produces the
/// exact bytes that should be written to the output file.
fn compress_block(raw: Vec<u8>, algo: CompressionAlgorithm) -> CompressedBlock {
    match lzf::compress_block(algo, &raw) {
        Some(compressed) => {
            let len = compressed.len() as u32;
            let mut data = Vec::with_capacity(4 + compressed.len());
            data.extend_from_slice(&len.to_le_bytes());
            data.extend_from_slice(&compressed);
            CompressedBlock { data }
        }
        None => {
            let len = raw.len() as u32 | 0x80000000;
            let mut data = Vec::with_capacity(4 + raw.len());
            data.extend_from_slice(&len.to_le_bytes());
            data.extend_from_slice(&raw);
            CompressedBlock { data }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_records<W: Write>(
    writer: &mut W,
    df: &DataFrame,
    fields: &[FieldMeta],
    fixed_size: usize,
    has_var: bool,
    num_records: u64,
    data_start: u64,
    compression: CompressionAlgorithm,
) -> Result<(Vec<i64>, u64)> {
    let columns: Vec<&Column> = df.columns().iter().collect();

    // ── Pipelined compression ──────────────────────────────────────
    // Main thread: serialize records into block buffers
    // Background thread: compress blocks while main thread builds the next one
    //
    // Flow: main → raw_tx → [compress thread] → done_rx → main (write)

    let (raw_tx, raw_rx) = mpsc::sync_channel::<Vec<u8>>(2);
    let (done_tx, done_rx) = mpsc::sync_channel::<CompressedBlock>(2);

    let compress_handle = thread::spawn(move || {
        for raw_block in raw_rx {
            let compressed = compress_block(raw_block, compression);
            if done_tx.send(compressed).is_err() {
                break;
            }
        }
    });

    let mut block_buf: Vec<u8> = Vec::with_capacity(BLOCK_SIZE + 4096);
    let mut file_pos = data_start;
    let mut block_index: Vec<i64> = vec![data_start as i64];
    let mut pending_blocks: usize = 0;

    // Reusable serialization buffers (avoid per-record allocation)
    let mut ser_fixed: Vec<u8> = vec![0u8; fixed_size];
    let mut ser_var_data: Vec<u8> = Vec::with_capacity(if has_var { 1024 } else { 0 });
    let mut ser_var_fixups: Vec<(usize, usize)> =
        Vec::with_capacity(if has_var { fields.len() } else { 0 });
    let mut ser_record: Vec<u8> = Vec::with_capacity(fixed_size + if has_var { 1024 } else { 0 });
    let mut ser_utf16_buf: Vec<u8> = Vec::with_capacity(if has_var { 256 } else { 0 });

    // Helper closure: drain all completed compressed blocks from the channel
    // and write them to the output.
    let drain_completed = |writer: &mut W,
                           done_rx: &mpsc::Receiver<CompressedBlock>,
                           file_pos: &mut u64,
                           pending: &mut usize|
     -> Result<()> {
        while *pending > 0 {
            match done_rx.try_recv() {
                Ok(block) => {
                    writer.write_all(&block.data)?;
                    *file_pos += block.data.len() as u64;
                    *pending -= 1;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
        Ok(())
    };

    // Helper: send the block buffer off for compression and swap in a fresh one.
    let send_block = |block_buf: &mut Vec<u8>,
                      raw_tx: &mpsc::SyncSender<Vec<u8>>,
                      pending: &mut usize|
     -> Result<()> {
        let block = std::mem::replace(block_buf, Vec::with_capacity(BLOCK_SIZE + 4096));
        raw_tx
            .send(block)
            .map_err(|_| YxdbError::LzfError("compression thread disconnected".into()))?;
        *pending += 1;
        Ok(())
    };

    for row in 0..num_records as usize {
        // At every RECORDS_PER_BLOCK boundary, force-flush and record position
        if row > 0 && row % RECORDS_PER_BLOCK == 0 {
            if !block_buf.is_empty() {
                // Drain any completed blocks before recording position
                drain_completed(writer, &done_rx, &mut file_pos, &mut pending_blocks)?;
                // Wait for all pending to flush so file_pos is accurate for block index
                while pending_blocks > 0 {
                    let block = done_rx
                        .recv()
                        .map_err(|_| YxdbError::LzfError("compression thread died".into()))?;
                    writer.write_all(&block.data)?;
                    file_pos += block.data.len() as u64;
                    pending_blocks -= 1;
                }
                send_block(&mut block_buf, &raw_tx, &mut pending_blocks)?;
                // Drain the block we just sent (it's guaranteed to finish soon)
                while pending_blocks > 0 {
                    let block = done_rx
                        .recv()
                        .map_err(|_| YxdbError::LzfError("compression thread died".into()))?;
                    writer.write_all(&block.data)?;
                    file_pos += block.data.len() as u64;
                    pending_blocks -= 1;
                }
            }
            block_index.push(file_pos as i64);
        }

        // Build one record using reusable buffers
        build_record_into(
            &mut ser_fixed,
            &mut ser_var_data,
            &mut ser_var_fixups,
            &mut ser_record,
            &mut ser_utf16_buf,
            fields,
            &columns,
            fixed_size,
            has_var,
            row,
        )?;

        // Flush block BEFORE adding if it would exceed BLOCK_SIZE
        if block_buf.len() + ser_record.len() > BLOCK_SIZE && !block_buf.is_empty() {
            // Drain completed blocks opportunistically
            drain_completed(writer, &done_rx, &mut file_pos, &mut pending_blocks)?;
            // Send current block for background compression
            send_block(&mut block_buf, &raw_tx, &mut pending_blocks)?;
        }

        // Append record data, splitting across block boundaries if needed.
        // Alteryx writes records contiguously across blocks — a record may
        // start in one block and finish in the next. We replicate this
        // behaviour so that large records (e.g. big blobs) get compressed
        // in standard-sized blocks rather than stored uncompressed.
        let mut remaining = ser_record.as_slice();
        while !remaining.is_empty() {
            let space = BLOCK_SIZE - block_buf.len();
            if remaining.len() <= space {
                block_buf.extend_from_slice(remaining);
                break;
            }
            // Fill current block to capacity, then flush
            block_buf.extend_from_slice(&remaining[..space]);
            remaining = &remaining[space..];
            drain_completed(writer, &done_rx, &mut file_pos, &mut pending_blocks)?;
            send_block(&mut block_buf, &raw_tx, &mut pending_blocks)?;
        }
    }

    // Flush remaining block
    if !block_buf.is_empty() {
        send_block(&mut block_buf, &raw_tx, &mut pending_blocks)?;
    }

    // Signal the compression thread that no more blocks are coming
    drop(raw_tx);

    // Drain all remaining compressed blocks
    while pending_blocks > 0 {
        let block = done_rx
            .recv()
            .map_err(|_| YxdbError::LzfError("compression thread died".into()))?;
        writer.write_all(&block.data)?;
        file_pos += block.data.len() as u64;
        pending_blocks -= 1;
    }

    compress_handle
        .join()
        .map_err(|_| YxdbError::LzfError("compression thread panicked".into()))?;

    Ok((block_index, file_pos))
}

/// Build a single record into reusable buffers, avoiding per-record allocation.
#[allow(clippy::too_many_arguments)]
fn build_record_into(
    fixed: &mut Vec<u8>,
    var_data: &mut Vec<u8>,
    var_fixups: &mut Vec<(usize, usize)>,
    record_out: &mut Vec<u8>,
    utf16_buf: &mut Vec<u8>,
    fields: &[FieldMeta],
    columns: &[&Column],
    fixed_size: usize,
    has_var: bool,
    row: usize,
) -> Result<()> {
    // Reset buffers (reusing allocations)
    fixed.clear();
    fixed.resize(fixed_size, 0);
    var_data.clear();
    var_fixups.clear();

    for (col_idx, field) in fields.iter().enumerate() {
        let col = columns[col_idx];
        serialize_field_into(fixed, var_data, var_fixups, utf16_buf, field, col, row)?;
    }

    // Fix up the variable-field offsets in the fixed portion.
    if has_var {
        for (field_offset, var_data_start) in var_fixups.iter() {
            let target = fixed_size + 4 + var_data_start;
            let offset_from_field = target - field_offset;
            let fixed_val = (offset_from_field as u32) | 0x80000000;
            fixed[*field_offset..*field_offset + 4].copy_from_slice(&fixed_val.to_le_bytes());
        }
    }

    // Assemble final record into record_out
    record_out.clear();
    record_out.extend_from_slice(fixed);
    if has_var {
        let var_len = var_data.len() as u32;
        record_out.extend_from_slice(&var_len.to_le_bytes());
        record_out.extend_from_slice(var_data);
    }

    Ok(())
}

/// Serialize a single field value into the fixed and variable buffers.
fn serialize_field_into(
    fixed: &mut [u8],
    var_data: &mut Vec<u8>,
    var_fixups: &mut Vec<(usize, usize)>,
    utf16_buf: &mut Vec<u8>,
    field: &FieldMeta,
    col: &Column,
    row: usize,
) -> Result<()> {
    let off = field.offset;

    match field.field_type {
        FieldType::Bool => {
            let series = col
                .bool()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(true) => fixed[off] = 1,
                Some(false) => fixed[off] = 0,
                None => fixed[off] = 2,
            }
        }

        FieldType::Byte => {
            let value: Option<u8> = match col.dtype() {
                DataType::UInt8 => col
                    .u8()
                    .map_err(|e| YxdbError::ConversionError(e.to_string()))?
                    .get(row),
                _ => col
                    .i16()
                    .map_err(|e| YxdbError::ConversionError(e.to_string()))?
                    .get(row)
                    .map(|v| v as u8),
            };
            match value {
                Some(v) => {
                    fixed[off] = v;
                    fixed[off + 1] = 0;
                }
                None => {
                    fixed[off] = 0;
                    fixed[off + 1] = 1;
                }
            }
        }

        FieldType::Int16 => {
            let value: Option<i16> = match col.dtype() {
                DataType::Int8 => col
                    .i8()
                    .map_err(|e| YxdbError::ConversionError(e.to_string()))?
                    .get(row)
                    .map(|v| v as i16),
                _ => col
                    .i16()
                    .map_err(|e| YxdbError::ConversionError(e.to_string()))?
                    .get(row),
            };
            match value {
                Some(v) => {
                    fixed[off..off + 2].copy_from_slice(&v.to_le_bytes());
                    fixed[off + 2] = 0;
                }
                None => {
                    fixed[off + 2] = 1;
                }
            }
        }

        FieldType::Int32 => {
            let value: Option<i32> = match col.dtype() {
                DataType::UInt16 => col
                    .u16()
                    .map_err(|e| YxdbError::ConversionError(e.to_string()))?
                    .get(row)
                    .map(|v| v as i32),
                _ => col
                    .i32()
                    .map_err(|e| YxdbError::ConversionError(e.to_string()))?
                    .get(row),
            };
            match value {
                Some(v) => {
                    fixed[off..off + 4].copy_from_slice(&v.to_le_bytes());
                    fixed[off + 4] = 0;
                }
                None => {
                    fixed[off + 4] = 1;
                }
            }
        }

        FieldType::Int64 => {
            let value: Option<i64> = match col.dtype() {
                DataType::UInt32 => col
                    .u32()
                    .map_err(|e| YxdbError::ConversionError(e.to_string()))?
                    .get(row)
                    .map(|v| v as i64),
                DataType::UInt64 => {
                    let v = col
                        .u64()
                        .map_err(|e| YxdbError::ConversionError(e.to_string()))?
                        .get(row);
                    match v {
                        Some(val) if val > i64::MAX as u64 => {
                            return Err(YxdbError::ConversionError(format!(
                                "column '{}' at row {}: UInt64 value {} exceeds the YXDB Int64 range (max {}). \
                                 Cast to Decimal or String before writing.",
                                col.name(), row, val, i64::MAX
                            )));
                        }
                        Some(val) => Some(val as i64),
                        None => None,
                    }
                }
                DataType::Duration(tu) => {
                    // Duration stored as physical i64 — normalize to microseconds
                    let phys = col.to_physical_repr();
                    let v = phys
                        .i64()
                        .map_err(|e| YxdbError::ConversionError(e.to_string()))?
                        .get(row);
                    v.map(|val| match tu {
                        TimeUnit::Nanoseconds => val / 1_000,
                        TimeUnit::Microseconds => val,
                        TimeUnit::Milliseconds => val * 1_000,
                    })
                }
                _ => col
                    .i64()
                    .map_err(|e| YxdbError::ConversionError(e.to_string()))?
                    .get(row),
            };
            match value {
                Some(v) => {
                    fixed[off..off + 8].copy_from_slice(&v.to_le_bytes());
                    fixed[off + 8] = 0;
                }
                None => {
                    fixed[off + 8] = 1;
                }
            }
        }

        FieldType::Float => {
            let series = col
                .f32()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(v) => {
                    fixed[off..off + 4].copy_from_slice(&v.to_le_bytes());
                    fixed[off + 4] = 0;
                }
                None => {
                    fixed[off + 4] = 1;
                }
            }
        }

        FieldType::Double => {
            let series = col
                .f64()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(v) => {
                    fixed[off..off + 8].copy_from_slice(&v.to_le_bytes());
                    fixed[off + 8] = 0;
                }
                None => {
                    fixed[off + 8] = 1;
                }
            }
        }

        FieldType::FixedDecimal => {
            // Support both Decimal (i128) and Float64 input columns
            match col.dtype() {
                DataType::Decimal(_, _) => {
                    let decimal_ca = col
                        .decimal()
                        .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
                    match decimal_ca.phys.get(row) {
                        Some(val) => {
                            let s = format_decimal_i128(val, field.scale);
                            write_fixed_ascii(&mut fixed[off..off + field.size], &s);
                            fixed[off + field.size] = 0;
                        }
                        None => {
                            for b in &mut fixed[off..off + field.size] {
                                *b = 0;
                            }
                            fixed[off + field.size] = 1;
                        }
                    }
                }
                _ => {
                    let series = col
                        .f64()
                        .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
                    match series.get(row) {
                        Some(v) => {
                            let s = format!("{:.*}", field.scale, v);
                            write_fixed_ascii(&mut fixed[off..off + field.size], &s);
                            fixed[off + field.size] = 0;
                        }
                        None => {
                            for b in &mut fixed[off..off + field.size] {
                                *b = 0;
                            }
                            fixed[off + field.size] = 1;
                        }
                    }
                }
            }
        }

        FieldType::String => {
            let series = col
                .str()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(s) => {
                    write_fixed_ascii(&mut fixed[off..off + field.size], s);
                    fixed[off + field.size] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + field.size] {
                        *b = 0;
                    }
                    fixed[off + field.size] = 1;
                }
            }
        }

        FieldType::WString => {
            let series = col
                .str()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            let byte_len = field.size * 2;
            match series.get(row) {
                Some(s) => {
                    write_fixed_wstring(&mut fixed[off..off + byte_len], s, field.size);
                    fixed[off + byte_len] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + byte_len] {
                        *b = 0;
                    }
                    fixed[off + byte_len] = 1;
                }
            }
        }

        FieldType::VString => {
            let series = col
                .str()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some("") => {
                    // Empty string: fixed portion = 0
                    fixed[off..off + 4].copy_from_slice(&0u32.to_le_bytes());
                }
                Some(s) => {
                    let data = s.as_bytes();
                    let var_offset = var_data.len();
                    var_fixups.push((off, var_offset));
                    append_var_block(var_data, data);
                    // Fixed portion will be patched by fix-up pass
                    fixed[off..off + 4].copy_from_slice(&0u32.to_le_bytes());
                }
                None => {
                    fixed[off..off + 4].copy_from_slice(&1u32.to_le_bytes());
                }
            }
        }

        FieldType::VWString => {
            let series = col
                .str()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some("") => {
                    fixed[off..off + 4].copy_from_slice(&0u32.to_le_bytes());
                }
                Some(s) => {
                    let var_offset = var_data.len();
                    var_fixups.push((off, var_offset));
                    // Reuse utf16_buf across records to avoid per-row allocation
                    utf16_buf.clear();
                    for cu in s.encode_utf16() {
                        utf16_buf.extend_from_slice(&cu.to_le_bytes());
                    }
                    append_var_block_header(var_data, utf16_buf.len());
                    var_data.extend_from_slice(utf16_buf);
                    fixed[off..off + 4].copy_from_slice(&0u32.to_le_bytes());
                }
                None => {
                    fixed[off..off + 4].copy_from_slice(&1u32.to_le_bytes());
                }
            }
        }

        FieldType::Date => {
            let series = col
                .date()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.phys.get(row) {
                Some(days) => {
                    let date_str = days_to_date_str(days);
                    write_fixed_ascii(&mut fixed[off..off + 10], &date_str);
                    fixed[off + 10] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + 10] {
                        *b = 0;
                    }
                    fixed[off + 10] = 1;
                }
            }
        }

        FieldType::Time => {
            // Polars Time is stored as i64 nanoseconds since midnight.
            // Extract the physical i64 representation.
            let phys = col.to_physical_repr();
            let series = phys
                .i64()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(ns) => {
                    let time_str = ns_to_time_str(ns);
                    write_fixed_ascii(&mut fixed[off..off + 8], &time_str);
                    fixed[off + 8] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + 8] {
                        *b = 0;
                    }
                    fixed[off + 8] = 1;
                }
            }
        }

        FieldType::DateTime => {
            let series = col
                .datetime()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            // Determine the time unit from the column's dtype
            let time_unit = match col.dtype() {
                DataType::Datetime(tu, _) => *tu,
                _ => TimeUnit::Microseconds,
            };
            match series.phys.get(row) {
                Some(val) => {
                    // Normalize to microseconds
                    let us = match time_unit {
                        TimeUnit::Nanoseconds => val / 1_000,
                        TimeUnit::Microseconds => val,
                        TimeUnit::Milliseconds => val * 1_000,
                    };
                    let dt_str = us_to_datetime_str(us);
                    write_fixed_ascii(&mut fixed[off..off + 19], &dt_str);
                    fixed[off + 19] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + 19] {
                        *b = 0;
                    }
                    fixed[off + 19] = 1;
                }
            }
        }

        FieldType::Blob | FieldType::SpatialObj => {
            let series = col
                .binary()
                .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some([]) => {
                    fixed[off..off + 4].copy_from_slice(&0u32.to_le_bytes());
                }
                Some(data) => {
                    let var_offset = var_data.len();
                    var_fixups.push((off, var_offset));
                    append_var_block(var_data, data);
                    fixed[off..off + 4].copy_from_slice(&0u32.to_le_bytes());
                }
                None => {
                    fixed[off..off + 4].copy_from_slice(&1u32.to_le_bytes());
                }
            }
        }
    }

    Ok(())
}

// ── Field serialization ────────────────────────────────────────────────

/// Append variable-length data as a var block to var_data.
///
/// Small block (≤ 127 bytes): 1-byte header (len << 1 | 1), then data.
/// Normal block (> 127 bytes): 4-byte header (len * 2), then data.
fn append_var_block(var_data: &mut Vec<u8>, data: &[u8]) {
    append_var_block_header(var_data, data.len());
    var_data.extend_from_slice(data);
}

/// Append just the variable-length block header (without data).
/// Used when data will be written separately (e.g. streaming UTF-16 encoding).
fn append_var_block_header(var_data: &mut Vec<u8>, byte_len: usize) {
    if byte_len <= 127 {
        var_data.push(((byte_len as u8) << 1) | 1);
    } else {
        let raw_len = (byte_len * 2) as u32;
        var_data.extend_from_slice(&raw_len.to_le_bytes());
    }
}

// ── Helper functions ───────────────────────────────────────────────────

/// Format an i128 unscaled decimal value as an ASCII decimal string.
/// E.g., `format_decimal_i128(12345678, 4)` → `"1234.5678"`.
fn format_decimal_i128(value: i128, scale: usize) -> String {
    if scale == 0 {
        return value.to_string();
    }
    let neg = value < 0;
    let abs = value.unsigned_abs();
    let divisor = 10u128.pow(scale as u32);
    let int_part = abs / divisor;
    let frac_part = abs % divisor;
    if neg {
        format!("-{}.{:0>width$}", int_part, frac_part, width = scale)
    } else {
        format!("{}.{:0>width$}", int_part, frac_part, width = scale)
    }
}

/// Write a fixed-length ASCII string into the buffer, null-padding.
fn write_fixed_ascii(buf: &mut [u8], s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(buf.len());
    buf[..len].copy_from_slice(&bytes[..len]);
    // Null-pad the rest
    for b in &mut buf[len..] {
        *b = 0;
    }
}

/// Write a fixed-length UTF-16LE string into the buffer, null-padding.
fn write_fixed_wstring(buf: &mut [u8], s: &str, max_chars: usize) {
    let mut pos = 0;
    for cu in s.encode_utf16().take(max_chars) {
        if pos + 2 > buf.len() {
            break;
        }
        buf[pos..pos + 2].copy_from_slice(&cu.to_le_bytes());
        pos += 2;
    }
    // Null-pad the rest
    for b in &mut buf[pos..] {
        *b = 0;
    }
}

/// Convert days since Unix epoch to "YYYY-MM-DD" string.
fn days_to_date_str(days: i32) -> String {
    let (y, m, d) = days_to_civil(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Convert microseconds since Unix epoch to "YYYY-MM-DD HH:MM:SS" string.
fn us_to_datetime_str(us: i64) -> String {
    let total_secs = us.div_euclid(1_000_000);
    let days = total_secs.div_euclid(86400) as i32;
    let day_secs = total_secs.rem_euclid(86400) as u32;
    let (y, m, d) = days_to_civil(days);
    let h = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, h, min, s)
}

/// Convert nanoseconds since midnight to "HH:MM:SS" string.
fn ns_to_time_str(ns: i64) -> String {
    let total_secs = ns / 1_000_000_000;
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// Convert days since Unix epoch to (year, month, day).
/// Inverse of Hinnant's algorithm from the reader.
fn days_to_civil(days: i32) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: create a DataFrame from columns, inferring height.
    fn test_df(columns: Vec<Column>) -> DataFrame {
        let h = columns.first().map_or(0, |c| c.len());
        DataFrame::new(h, columns).unwrap()
    }

    #[test]
    fn test_days_to_civil_epoch() {
        assert_eq!(days_to_civil(0), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_civil_known_date() {
        // 2025-03-15
        let days = 20162; // known from reader test
        let (y, m, d) = days_to_civil(days);
        assert_eq!((y, m, d), (2025, 3, 15));
    }

    #[test]
    fn test_days_to_date_str() {
        assert_eq!(days_to_date_str(0), "1970-01-01");
        assert_eq!(days_to_date_str(20162), "2025-03-15");
    }

    #[test]
    fn test_ms_to_datetime_str() {
        // 2025-03-15 08:30:00
        let us = 20162i64 * 86_400_000_000 + 8 * 3_600_000_000 + 30 * 60_000_000;
        assert_eq!(us_to_datetime_str(us), "2025-03-15 08:30:00");
    }

    #[test]
    fn test_datetime_str_pre_epoch() {
        // 1969-12-31 23:59:59 → -1_000_000 us
        assert_eq!(us_to_datetime_str(-1_000_000), "1969-12-31 23:59:59");
        // 1960-01-01 12:00:00
        // days from 1970-01-01 to 1960-01-01 = -3653
        let us = -3653i64 * 86_400_000_000 + 12 * 3_600_000_000;
        assert_eq!(us_to_datetime_str(us), "1960-01-01 12:00:00");
    }

    #[test]
    fn test_build_meta_xml() {
        let fields = vec![
            FieldMeta {
                name: "ID".to_string(),
                field_type: FieldType::Int32,
                size: 4,
                scale: 0,
                offset: 0,
            },
            FieldMeta {
                name: "Name".to_string(),
                field_type: FieldType::VWString,
                size: 256,
                scale: 0,
                offset: 5,
            },
        ];
        let xml = build_meta_xml(&fields);
        assert!(xml.contains("<RecordInfo>"));
        assert!(xml.contains("name=\"ID\""));
        assert!(xml.contains("type=\"Int32\""));
        assert!(xml.contains("name=\"Name\""));
        assert!(xml.contains("type=\"V_WString\""));
        assert!(xml.contains("</RecordInfo>"));
    }

    #[test]
    fn test_encode_utf16_le() {
        let s = "AB";
        let bytes = encode_utf16_le(s);
        // 'A' = 0x41, 'B' = 0x42, null = 0x00
        assert_eq!(bytes, vec![0x41, 0x00, 0x42, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_roundtrip_simple() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Create a simple DataFrame
        let df = df! {
            "id" => [1i32, 2, 3],
            "name" => ["Alice", "Bob", "Charlie"],
        }
        .unwrap();

        // Write to a temp file
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        write_yxdb(path, &df, &[]).unwrap();

        // Read it back
        let df2 = read_yxdb(path, SpatialMode::Raw, false).unwrap();

        // Compare
        assert_eq!(df2.height(), 3);
        assert_eq!(df2.width(), 2);

        let id_col = df2.column("id").unwrap();
        let id_vals: Vec<i32> = id_col
            .i32()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(id_vals, vec![1, 2, 3]);

        let name_col = df2.column("name").unwrap();
        let name_vals: Vec<&str> = name_col
            .str()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(name_vals, vec!["Alice", "Bob", "Charlie"]);
    }

    #[test]
    fn test_roundtrip_with_nulls() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Create a DataFrame with nulls
        let df = df! {
            "val" => [Some(10i32), None, Some(30)],
        }
        .unwrap();

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        write_yxdb(path, &df, &[]).unwrap();

        let df2 = read_yxdb(path, SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.height(), 3);

        let val_col = df2.column("val").unwrap();
        let vals: Vec<Option<i32>> = val_col.i32().unwrap().into_iter().collect();
        assert_eq!(vals, vec![Some(10), None, Some(30)]);
    }

    #[test]
    fn test_roundtrip_multiple_types() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = df! {
            "bool_col" => [true, false, true],
            "i16_col" => [1i16, 2, 3],
            "i32_col" => [100i32, 200, 300],
            "i64_col" => [1000i64, 2000, 3000],
            "f32_col" => [1.5f32, 2.5, 3.5],
            "f64_col" => [10.5f64, 20.5, 30.5],
            "str_col" => ["a", "bb", "ccc"],
        }
        .unwrap();

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.height(), 3);
        assert_eq!(df2.width(), 7);

        // Check bool
        let bools: Vec<bool> = df2
            .column("bool_col")
            .unwrap()
            .bool()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(bools, vec![true, false, true]);

        // Check i32
        let i32s: Vec<i32> = df2
            .column("i32_col")
            .unwrap()
            .i32()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(i32s, vec![100, 200, 300]);

        // Check strings
        let strs: Vec<&str> = df2
            .column("str_col")
            .unwrap()
            .str()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(strs, vec!["a", "bb", "ccc"]);
    }

    #[test]
    fn test_streaming_writer() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Create batches
        let batch1 = df! {
            "id" => [1i32, 2, 3],
            "name" => ["Alice", "Bob", "Charlie"],
        }
        .unwrap();

        let batch2 = df! {
            "id" => [4i32, 5],
            "name" => ["David", "Eve"],
        }
        .unwrap();

        // Write using streaming writer
        let tmp = NamedTempFile::new().unwrap();
        {
            let mut writer = YxdbWriter::new(tmp.path(), &batch1).unwrap();
            writer.write_batch(&batch1).unwrap();
            writer.write_batch(&batch2).unwrap();
            assert_eq!(writer.record_count(), 5);
            writer.finish().unwrap();
        }

        // Read back and verify
        let df = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df.height(), 5);
        assert_eq!(df.width(), 2);

        let ids: Vec<i32> = df
            .column("id")
            .unwrap()
            .i32()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);

        let names: Vec<&str> = df
            .column("name")
            .unwrap()
            .str()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(names, vec!["Alice", "Bob", "Charlie", "David", "Eve"]);
    }

    #[test]
    fn test_streaming_writer_many_batches() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Create many small batches to test block flushing
        let tmp = NamedTempFile::new().unwrap();
        let template = df! { "value" => [0i64] }.unwrap();

        {
            let mut writer = YxdbWriter::new(tmp.path(), &template).unwrap();

            for i in 0..1000 {
                let batch = df! { "value" => [i as i64] }.unwrap();
                writer.write_batch(&batch).unwrap();
            }

            assert_eq!(writer.record_count(), 1000);
            writer.finish().unwrap();
        }

        let df = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df.height(), 1000);

        let values: Vec<i64> = df
            .column("value")
            .unwrap()
            .i64()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(values, (0..1000).collect::<Vec<_>>());
    }

    #[test]
    fn test_build_header_compression_version() {
        let header = build_header(42, 100, CompressionAlgorithm::Lzf);

        // Magic at offset 0
        assert_eq!(&header[0..21], MAGIC);

        // FileID at offset 64-68: should be 0x00440204
        let file_id = u32::from_le_bytes(header[64..68].try_into().unwrap());
        assert_eq!(file_id, 0x00440204, "FileID mismatch");

        // meta_info_size at offset 80
        let meta = u32::from_le_bytes(header[80..84].try_into().unwrap());
        assert_eq!(meta, 100);

        // num_records at offset 104
        let num_rec = u64::from_le_bytes(header[104..112].try_into().unwrap());
        assert_eq!(num_rec, 42);

        // nCompressionVersion at offset 112: MUST be 1
        let comp_ver = i32::from_le_bytes(header[112..116].try_into().unwrap());
        assert_eq!(comp_ver, 1, "nCompressionVersion must be 1 for LZF");
    }

    #[test]
    fn test_roundtrip_compression_version_in_file() {
        use std::fs;
        use tempfile::NamedTempFile;

        let df = df! {
            "id" => [1i32, 2, 3],
        }
        .unwrap();

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        // Read raw bytes and check header offset 112
        let bytes = fs::read(tmp.path()).unwrap();
        let comp_ver = i32::from_le_bytes(bytes[112..116].try_into().unwrap());
        assert_eq!(comp_ver, 1, "nCompressionVersion in written file must be 1");

        // Also verify FileID
        let file_id = u32::from_le_bytes(bytes[64..68].try_into().unwrap());
        assert_eq!(file_id, 0x00440204, "FileID in written file");
    }

    // ── Edge-case / stress tests ─────────────────────────────────────

    #[test]
    fn test_roundtrip_empty_dataframe() {
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = test_df(vec![Column::new("x".into(), Vec::<i32>::new())]);
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        // Verify the file header declares 0 records
        let bytes = std::fs::read(tmp.path()).unwrap();
        assert!(
            bytes.len() >= 512,
            "file should at least contain the 512-byte header"
        );
        let num_records = u64::from_le_bytes(bytes[104..112].try_into().unwrap());
        assert_eq!(num_records, 0, "header should declare 0 records");
        // Magic string should be present
        assert_eq!(&bytes[..21], b"Alteryx Database File");
    }

    #[test]
    fn test_roundtrip_single_row() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = df! {
            "a" => [42i32],
            "b" => ["only"],
        }
        .unwrap();
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.height(), 1);
        assert_eq!(df2.column("a").unwrap().i32().unwrap().get(0), Some(42));
        assert_eq!(df2.column("b").unwrap().str().unwrap().get(0), Some("only"));
    }

    #[test]
    fn test_roundtrip_all_null_column() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let s = Column::new("n".into(), &[None::<i64>, None, None]);
        let df = test_df(vec![s]);
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.column("n").unwrap().null_count(), 3);
    }

    #[test]
    fn test_roundtrip_empty_strings() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = df! {
            "s" => ["", "", ""],
        }
        .unwrap();
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let vals: Vec<&str> = df2
            .column("s")
            .unwrap()
            .str()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(vals, vec!["", "", ""]);
        assert_eq!(df2.column("s").unwrap().null_count(), 0);
    }

    #[test]
    fn test_roundtrip_mixed_null_and_empty_strings() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let s = Column::new("s".into(), &[Some("hi"), Some(""), None, Some(""), None]);
        let df = test_df(vec![s]);
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("s").unwrap().str().unwrap();
        assert_eq!(col.get(0), Some("hi"));
        assert_eq!(col.get(1), Some(""));
        assert_eq!(col.get(2), None);
        assert_eq!(col.get(3), Some(""));
        assert_eq!(col.get(4), None);
    }

    #[test]
    fn test_roundtrip_int_boundary_values() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = df! {
            "i16" => [i16::MIN, i16::MAX, 0i16],
            "i32" => [i32::MIN, i32::MAX, 0i32],
            "i64" => [i64::MIN, i64::MAX, 0i64],
        }
        .unwrap();
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(
            df2.column("i16").unwrap().i16().unwrap().get(0),
            Some(i16::MIN)
        );
        assert_eq!(
            df2.column("i16").unwrap().i16().unwrap().get(1),
            Some(i16::MAX)
        );
        assert_eq!(
            df2.column("i32").unwrap().i32().unwrap().get(0),
            Some(i32::MIN)
        );
        assert_eq!(
            df2.column("i32").unwrap().i32().unwrap().get(1),
            Some(i32::MAX)
        );
        assert_eq!(
            df2.column("i64").unwrap().i64().unwrap().get(0),
            Some(i64::MIN)
        );
        assert_eq!(
            df2.column("i64").unwrap().i64().unwrap().get(1),
            Some(i64::MAX)
        );
    }

    #[test]
    fn test_roundtrip_float_special_values() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = df! {
            "f64" => [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 0.0f64, -0.0f64],
        }
        .unwrap();
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("f64").unwrap().f64().unwrap();
        assert_eq!(col.get(0), Some(f64::INFINITY));
        assert_eq!(col.get(1), Some(f64::NEG_INFINITY));
        assert!(col.get(2).unwrap().is_nan());
        assert_eq!(col.get(3), Some(0.0));
        // -0.0 and 0.0 compare equal but bit pattern differs
        assert_eq!(col.get(4), Some(0.0));
    }

    #[test]
    fn test_roundtrip_bool_all_states() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let s = Column::new(
            "b".into(),
            &[Some(true), Some(false), None, Some(true), None],
        );
        let df = test_df(vec![s]);
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("b").unwrap().bool().unwrap();
        assert_eq!(col.get(0), Some(true));
        assert_eq!(col.get(1), Some(false));
        assert_eq!(col.get(2), None);
        assert_eq!(col.get(3), Some(true));
        assert_eq!(col.get(4), None);
    }

    #[test]
    fn test_roundtrip_unicode_strings() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = df! {
            "text" => [
                "Hello",             // ASCII
                "café",              // Latin-1 supplement
                "日本語",             // CJK
                "ĀĂĄĆĈĊČ",          // Latin Extended-A (the bug range)
                "αβγδ",              // Greek
                "Привет",            // Cyrillic
            ],
        }
        .unwrap();
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("text").unwrap().str().unwrap();
        assert_eq!(col.get(0), Some("Hello"));
        assert_eq!(col.get(1), Some("café"));
        assert_eq!(col.get(2), Some("日本語"));
        assert_eq!(col.get(3), Some("ĀĂĄĆĈĊČ"));
        assert_eq!(col.get(4), Some("αβγδ"));
        assert_eq!(col.get(5), Some("Привет"));
    }

    #[test]
    fn test_roundtrip_long_string() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let long_str = "X".repeat(50_000);
        let df = df! {
            "s" => [long_str.as_str()],
        }
        .unwrap();
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(
            df2.column("s").unwrap().str().unwrap().get(0),
            Some(long_str.as_str())
        );
    }

    #[test]
    fn test_roundtrip_wide_dataframe() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let cols: Vec<Column> = (0..50)
            .map(|i| Column::new(format!("c{i:03}").into(), &[i]))
            .collect();
        let df = test_df(cols);
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.width(), 50);
        assert_eq!(df2.height(), 1);
        for i in 0..50 {
            assert_eq!(
                df2.column(&format!("c{i:03}"))
                    .unwrap()
                    .i32()
                    .unwrap()
                    .get(0),
                Some(i)
            );
        }
    }

    #[test]
    fn test_roundtrip_many_rows_stress() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let n = 20_000;
        let ids: Vec<i64> = (0..n).collect();
        let texts: Vec<String> = (0..n).map(|i| format!("row_{i:06}")).collect();
        let df = test_df(vec![
            Column::new("id".into(), &ids),
            Column::new("text".into(), texts),
        ]);
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.height(), n as usize);
        assert_eq!(df2.column("id").unwrap().i64().unwrap().get(0), Some(0));
        assert_eq!(
            df2.column("id").unwrap().i64().unwrap().get(n as usize - 1),
            Some(n - 1)
        );
    }

    #[test]
    fn test_roundtrip_alternating_nulls() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let vals: Vec<Option<i32>> = (0..100)
            .map(|i| if i % 2 == 0 { Some(i) } else { None })
            .collect();
        let s = Column::new("v".into(), &vals);
        let df = test_df(vec![s]);
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("v").unwrap().i32().unwrap();
        for i in 0..100 {
            assert_eq!(col.get(i as usize), vals[i as usize]);
        }
    }

    #[test]
    fn test_streaming_writer_empty_batch() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let batch = df! {
            "x" => [1i32, 2],
        }
        .unwrap();
        let empty_batch = test_df(vec![Column::new("x".into(), Vec::<i32>::new())]);

        let tmp = NamedTempFile::new().unwrap();
        {
            let mut writer = YxdbWriter::new(tmp.path(), &batch).unwrap();
            writer.write_batch(&empty_batch).unwrap();
            writer.write_batch(&batch).unwrap();
            writer.write_batch(&empty_batch).unwrap();
            assert_eq!(writer.record_count(), 2);
            writer.finish().unwrap();
        }
        let df = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df.height(), 2);
    }

    #[test]
    fn test_write_read_cycle_stability() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let mut df = df! {
            "id" => [1i32, 2, 3],
            "text" => ["a", "b", "c"],
        }
        .unwrap();

        for _ in 0..5 {
            let tmp = NamedTempFile::new().unwrap();
            write_yxdb(tmp.path(), &df, &[]).unwrap();
            df = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        }
        let ids: Vec<i32> = df
            .column("id")
            .unwrap()
            .i32()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(ids, vec![1, 2, 3]);
        let texts: Vec<&str> = df
            .column("text")
            .unwrap()
            .str()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(texts, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_xml_escape_special_chars_in_column_name() {
        let fields = vec![FieldMeta {
            name: "A&B<C>D\"E'F".to_string(),
            field_type: FieldType::Int32,
            size: 4,
            scale: 0,
            offset: 0,
        }];
        let xml = build_meta_xml(&fields);
        // Must NOT contain raw & < > " ' in attribute values
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("&lt;"));
        assert!(xml.contains("&gt;"));
        assert!(xml.contains("&quot;"));
        assert!(xml.contains("&apos;"));
        assert!(!xml.contains("A&B"));
    }

    #[test]
    fn test_format_decimal_i128() {
        assert_eq!(format_decimal_i128(12345678, 4), "1234.5678");
        assert_eq!(format_decimal_i128(-12345678, 4), "-1234.5678");
        assert_eq!(format_decimal_i128(0, 4), "0.0000");
        assert_eq!(format_decimal_i128(42, 0), "42");
        assert_eq!(format_decimal_i128(-1, 0), "-1");
        assert_eq!(format_decimal_i128(1, 10), "0.0000000001");
        assert_eq!(format_decimal_i128(-50, 2), "-0.50");
    }

    #[test]
    fn test_days_to_civil_leap_years() {
        // 2000-02-29 is a leap day
        // 2000-01-01 = day 10957 from epoch
        let day_2000_01_01 = 10957;
        assert_eq!(days_to_civil(day_2000_01_01), (2000, 1, 1));
        assert_eq!(days_to_civil(day_2000_01_01 + 59), (2000, 2, 29));
        assert_eq!(days_to_civil(day_2000_01_01 + 60), (2000, 3, 1));
    }

    #[test]
    fn test_days_to_civil_negative_days() {
        // 1969-12-31 = day -1
        assert_eq!(days_to_civil(-1), (1969, 12, 31));
        // 1960-01-01
        assert_eq!(days_to_civil(-3653), (1960, 1, 1));
    }

    #[test]
    fn test_ns_to_time_str_boundaries() {
        assert_eq!(ns_to_time_str(0), "00:00:00");
        // 23:59:59
        let max_ns = (23 * 3600 + 59 * 60 + 59) * 1_000_000_000i64;
        assert_eq!(ns_to_time_str(max_ns), "23:59:59");
        // 12:00:00
        let noon_ns = 12 * 3600 * 1_000_000_000i64;
        assert_eq!(ns_to_time_str(noon_ns), "12:00:00");
    }

    // ── Additional edge-case / stress tests ──────────────────────────

    #[test]
    fn test_roundtrip_time_column() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Build Time series from nanoseconds since midnight
        let time_series = Series::from_any_values_and_dtype(
            "t".into(),
            &[
                AnyValue::Time(0),
                AnyValue::Time(12 * 3_600_000_000_000),
                AnyValue::Time((23 * 3600 + 59 * 60 + 59) * 1_000_000_000i64),
                AnyValue::Time((8 * 3600 + 30 * 60) * 1_000_000_000i64),
            ],
            &DataType::Time,
            false,
        )
        .unwrap();
        let df = test_df(vec![time_series.into()]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.height(), 4);
        assert_eq!(df2.column("t").unwrap().dtype(), &DataType::Time);
        // Extract physical i64 values to verify Time roundtrip
        let phys = df2.column("t").unwrap().to_physical_repr();
        let col = phys.i64().unwrap();
        assert_eq!(col.get(0), Some(0)); // midnight
        assert_eq!(col.get(1), Some(12 * 3_600_000_000_000)); // noon
    }

    #[test]
    fn test_roundtrip_date_column() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let date_series = Series::from_any_values_and_dtype(
            "d".into(),
            &[
                AnyValue::Date(0),          // 1970-01-01
                AnyValue::Date(-1),         // 1969-12-31
                AnyValue::Date(10957),      // 2000-01-01
                AnyValue::Date(10957 + 59), // 2000-02-29 (leap day)
                AnyValue::Date(20162),      // 2025-03-15
            ],
            &DataType::Date,
            false,
        )
        .unwrap();
        let df = test_df(vec![date_series.into()]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.height(), 5);
        let col = df2.column("d").unwrap().date().unwrap();
        assert_eq!(col.phys.get(0), Some(0));
        assert_eq!(col.phys.get(1), Some(-1));
        assert_eq!(col.phys.get(3), Some(10957 + 59));
    }

    #[test]
    fn test_roundtrip_binary_blob() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Various blob sizes including at the 127-byte threshold
        let blobs: Vec<Option<&[u8]>> = vec![
            Some(b""),           // empty
            Some(b"\x42"),       // single byte
            Some(&[0xAA; 127]),  // exactly 127 (small block max)
            Some(&[0xBB; 128]),  // 128 (needs 4-byte header)
            Some(&[0xFF; 1000]), // larger blob
            None,                // null
        ];
        let series = Column::new(
            "b".into(),
            blobs
                .iter()
                .map(|b| b.map(|v| v.to_vec()))
                .collect::<Vec<_>>(),
        );
        let df = test_df(vec![series]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("b").unwrap().binary().unwrap();
        assert_eq!(col.get(0).unwrap().len(), 0);
        assert_eq!(col.get(1).unwrap(), &[0x42]);
        assert_eq!(col.get(2).unwrap().len(), 127);
        assert_eq!(col.get(3).unwrap().len(), 128);
        assert_eq!(col.get(4).unwrap().len(), 1000);
        assert!(col.get(5).is_none());
    }

    #[test]
    fn test_roundtrip_variable_string_at_127_threshold() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Test strings at and around the 127-byte small/normal block threshold
        let owned: Vec<String> = vec![
            "A".to_string(),
            "B".repeat(126),
            "C".repeat(127),
            "D".repeat(128),
            "E".repeat(129),
            "F".repeat(1000),
        ];
        let df = df! { "s" => owned.iter().map(|s| s.as_str()).collect::<Vec<_>>() }.unwrap();

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("s").unwrap().str().unwrap();
        for (i, expected) in owned.iter().enumerate() {
            assert_eq!(
                col.get(i).unwrap(),
                expected.as_str(),
                "mismatch at index {i}"
            );
        }
    }

    #[test]
    fn test_roundtrip_emoji_surrogate_pairs() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = df! {
            "s" => ["😀", "🎉🚀💻", "Hello 🌍!", "A😀B😀C"],
        }
        .unwrap();
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("s").unwrap().str().unwrap();
        assert_eq!(col.get(0), Some("😀"));
        assert_eq!(col.get(1), Some("🎉🚀💻"));
        assert_eq!(col.get(2), Some("Hello 🌍!"));
        assert_eq!(col.get(3), Some("A😀B😀C"));
    }

    #[test]
    fn test_roundtrip_200_columns() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let cols: Vec<Column> = (0..200)
            .map(|i| Column::new(format!("c{i:04}").into(), &[i, i * 2]))
            .collect();
        let df = test_df(cols);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.width(), 200);
        assert_eq!(df2.height(), 2);
        // Verify first and last columns
        assert_eq!(df2.column("c0000").unwrap().i32().unwrap().get(0), Some(0));
        assert_eq!(
            df2.column("c0199").unwrap().i32().unwrap().get(0),
            Some(199)
        );
    }

    #[test]
    fn test_roundtrip_100k_rows_multiple_blocks() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // 100k rows × 9 bytes/record (Int64) ≈ 900kB → spans ~3.4 blocks
        let n = 100_000;
        let ids: Vec<i64> = (0..n).collect();
        let df = test_df(vec![Column::new("id".into(), &ids)]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.height(), n as usize);
        let col = df2.column("id").unwrap().i64().unwrap();
        assert_eq!(col.get(0), Some(0));
        assert_eq!(col.get(n as usize - 1), Some(n - 1));
        // Verify sum to catch any corruption
        let sum: i64 = col.into_iter().map(|v| v.unwrap()).sum();
        assert_eq!(sum, n * (n - 1) / 2);
    }

    #[test]
    fn test_roundtrip_all_null_multiple_types() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = test_df(vec![
            Column::new("i32".into(), &[None::<i32>, None, None]),
            Column::new("i64".into(), &[None::<i64>, None, None]),
            Column::new("f64".into(), &[None::<f64>, None, None]),
            Column::new("bool".into(), &[None::<bool>, None, None]),
            Column::new("str".into(), &[None::<&str>, None, None]),
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.height(), 3);
        for col_name in df2.get_column_names().into_iter() {
            assert_eq!(
                df2.column(col_name).unwrap().null_count(),
                3,
                "column {col_name} should be all null"
            );
        }
    }

    #[test]
    fn test_xml_escape_all_xml_special_chars() {
        // Test that all 5 XML special characters are properly escaped
        let fields = vec![
            FieldMeta {
                name: "a&b".to_string(),
                field_type: FieldType::Int32,
                size: 4,
                scale: 0,
                offset: 0,
            },
            FieldMeta {
                name: "c<d".to_string(),
                field_type: FieldType::Int32,
                size: 4,
                scale: 0,
                offset: 5,
            },
            FieldMeta {
                name: "e>f".to_string(),
                field_type: FieldType::Int32,
                size: 4,
                scale: 0,
                offset: 10,
            },
            FieldMeta {
                name: "g\"h".to_string(),
                field_type: FieldType::Int32,
                size: 4,
                scale: 0,
                offset: 15,
            },
            FieldMeta {
                name: "i'j".to_string(),
                field_type: FieldType::Int32,
                size: 4,
                scale: 0,
                offset: 20,
            },
        ];
        let xml = build_meta_xml(&fields);
        assert!(xml.contains("a&amp;b"), "& not escaped");
        assert!(xml.contains("c&lt;d"), "< not escaped");
        assert!(xml.contains("e&gt;f"), "> not escaped");
        assert!(xml.contains("g&quot;h"), "\" not escaped");
        assert!(xml.contains("i&apos;j"), "' not escaped");
    }

    #[test]
    fn test_roundtrip_unicode_column_names() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = test_df(vec![
            Column::new("Ñame".into(), &[1i32]),
            Column::new("日付".into(), &[2i32]),
            Column::new("Größe".into(), &[3i32]),
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();
        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let names: Vec<&str> = df2
            .get_column_names()
            .into_iter()
            .map(|n| n.as_str())
            .collect();
        assert_eq!(names, vec!["Ñame", "日付", "Größe"]);
    }

    #[test]
    fn test_days_to_civil_extreme_dates() {
        // Year 1 CE
        // 0001-01-01 is day -719162 from epoch
        assert_eq!(days_to_civil(-719162), (1, 1, 1));
        // Year 9999
        // 9999-12-31 is day 2932896 from epoch
        assert_eq!(days_to_civil(2932896), (9999, 12, 31));
    }

    #[test]
    fn test_days_to_civil_all_months_end() {
        // Verify last day of each month in 2023 (non-leap year)
        let jan1_2023 = 19358; // 2023-01-01 days from epoch
        let expected = [
            (31, 1, 31), // Jan has 31
            (59, 2, 28), // Feb has 28
            (90, 3, 31),
            (120, 4, 30),
            (151, 5, 31),
            (181, 6, 30),
            (212, 7, 31),
            (243, 8, 31),
            (273, 9, 30),
            (304, 10, 31),
            (334, 11, 30),
            (365, 12, 31),
        ];
        for (day_of_year, expected_month, expected_day) in expected {
            let (y, m, d) = days_to_civil(jan1_2023 + day_of_year - 1);
            assert_eq!(
                (y, m, d),
                (2023, expected_month, expected_day),
                "day of year {day_of_year}"
            );
        }
    }

    #[test]
    fn test_streaming_writer_many_empty_batches() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let template = df! { "x" => [0i32] }.unwrap();
        let empty = test_df(vec![Column::new("x".into(), Vec::<i32>::new())]);
        let data = df! { "x" => [1i32, 2, 3] }.unwrap();

        let tmp = NamedTempFile::new().unwrap();
        {
            let mut writer = YxdbWriter::new(tmp.path(), &template).unwrap();
            // Interleave many empty batches
            for _ in 0..100 {
                writer.write_batch(&empty).unwrap();
            }
            writer.write_batch(&data).unwrap();
            for _ in 0..100 {
                writer.write_batch(&empty).unwrap();
            }
            assert_eq!(writer.record_count(), 3);
            writer.finish().unwrap();
        }
        let df = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df.height(), 3);
        let vals: Vec<i32> = df
            .column("x")
            .unwrap()
            .i32()
            .unwrap()
            .into_iter()
            .map(|x| x.unwrap())
            .collect();
        assert_eq!(vals, vec![1, 2, 3]);
    }

    // ── New feature tests: spatial roundtrip, GeoArrow, header file_id ──

    #[test]
    fn test_written_file_has_no_spatial_index_file_id() {
        // Files we write should have file_id = 0x00440204 (no spatial index)
        use tempfile::NamedTempFile;
        let df = df! { "x" => [1i32] }.unwrap();
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        // Read raw bytes and check header offset 64..68
        let bytes = std::fs::read(tmp.path()).unwrap();
        let file_id = u32::from_le_bytes(bytes[64..68].try_into().unwrap());
        assert_eq!(
            file_id, 0x00440204,
            "written file should use NoSpatialIndex file_id"
        );
    }

    #[test]
    fn test_written_file_header_spatial_index_pos_zero() {
        // spatial_index_pos (offset 88..96) should be 0 for files we write
        use tempfile::NamedTempFile;
        let df = df! { "x" => [1i32] }.unwrap();
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        let bytes = std::fs::read(tmp.path()).unwrap();
        let spatial_idx_pos = i64::from_le_bytes(bytes[88..96].try_into().unwrap());
        assert_eq!(spatial_idx_pos, 0, "spatial_index_pos should be 0");
    }

    #[test]
    fn test_read_written_file_has_spatial_index_false() {
        // After writing and re-reading, has_spatial_index() should be false
        use crate::YxdbReader;
        use tempfile::NamedTempFile;
        let df = df! { "x" => [1i32] }.unwrap();
        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        let reader = YxdbReader::open(tmp.path()).unwrap();
        assert!(!reader.header.has_spatial_index());
        assert_eq!(reader.header.file_id, crate::ID_WRIGLEYDB_NO_SPATIAL_INDEX);
    }

    #[test]
    fn test_spatial_roundtrip_point_wkb_mode() {
        // Write a DataFrame with a spatial column (WKB Point), read it back in Wkb mode
        use crate::{read_yxdb, SpatialMode};
        use tempfile::NamedTempFile;

        // Build WKB Point: byte_order(1) + type(1=Point) + x + y
        let mut wkb_point = Vec::new();
        wkb_point.push(1u8); // little-endian
        wkb_point.extend_from_slice(&1u32.to_le_bytes()); // WKB_POINT
        wkb_point.extend_from_slice(&(-73.9857f64).to_le_bytes()); // x
        wkb_point.extend_from_slice(&40.7484f64.to_le_bytes()); // y

        let df = test_df(vec![
            Series::new("id".into(), &[1i32, 2]).into(),
            Series::new(
                "geom".into(),
                vec![Some(wkb_point.as_slice()), Some(wkb_point.as_slice())],
            )
            .into(),
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["geom"]).unwrap();

        // Read back in Wkb mode — should get WKB bytes back
        let df2 = read_yxdb(tmp.path(), SpatialMode::Wkb, false).unwrap();
        assert_eq!(df2.height(), 2);
        assert_eq!(df2.width(), 2);

        let geom_col = df2.column("geom").unwrap().binary().unwrap();
        let wkb0 = geom_col.get(0).unwrap();
        // Should be a valid WKB Point
        assert_eq!(wkb0[0], 1); // LE
        let wkb_type = u32::from_le_bytes(wkb0[1..5].try_into().unwrap());
        assert_eq!(wkb_type, 1); // Point
        let x = f64::from_le_bytes(wkb0[5..13].try_into().unwrap());
        let y = f64::from_le_bytes(wkb0[13..21].try_into().unwrap());
        assert!((x - (-73.9857)).abs() < 1e-10);
        assert!((y - 40.7484).abs() < 1e-10);
    }

    #[test]
    fn test_spatial_roundtrip_geoarrow_mode() {
        // GeoArrow mode at Rust level should produce same WKB as Wkb mode
        use crate::{read_yxdb, SpatialMode};
        use tempfile::NamedTempFile;

        let mut wkb_point = Vec::new();
        wkb_point.push(1u8);
        wkb_point.extend_from_slice(&1u32.to_le_bytes());
        wkb_point.extend_from_slice(&1.0f64.to_le_bytes());
        wkb_point.extend_from_slice(&2.0f64.to_le_bytes());

        let df = test_df(vec![
            Series::new("id".into(), &[1i32]).into(),
            Series::new("geom".into(), vec![Some(wkb_point.as_slice())]).into(),
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["geom"]).unwrap();

        let df_wkb = read_yxdb(tmp.path(), SpatialMode::Wkb, false).unwrap();
        let df_geo = read_yxdb(tmp.path(), SpatialMode::GeoArrow, false).unwrap();

        // Both should produce identical WKB output
        let wkb_col = df_wkb.column("geom").unwrap().binary().unwrap();
        let geo_col = df_geo.column("geom").unwrap().binary().unwrap();
        assert_eq!(wkb_col.get(0), geo_col.get(0));
    }

    #[test]
    fn test_spatial_roundtrip_raw_mode_returns_shp() {
        // Raw mode should return raw SHP bytes, not WKB
        use crate::{read_yxdb, SpatialMode};
        use tempfile::NamedTempFile;

        let mut wkb_point = Vec::new();
        wkb_point.push(1u8);
        wkb_point.extend_from_slice(&1u32.to_le_bytes());
        wkb_point.extend_from_slice(&5.0f64.to_le_bytes());
        wkb_point.extend_from_slice(&10.0f64.to_le_bytes());

        let df = test_df(vec![Series::new(
            "geom".into(),
            vec![Some(wkb_point.as_slice())],
        )
        .into()]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["geom"]).unwrap();

        let df_raw = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let raw_col = df_raw.column("geom").unwrap().binary().unwrap();
        let raw0 = raw_col.get(0).unwrap();
        // In raw mode, should be SHP format (starts with shape type i32 = 1 for Point)
        let shape_type = i32::from_le_bytes(raw0[0..4].try_into().unwrap());
        assert_eq!(shape_type, 1); // SHP_POINT
    }

    #[test]
    fn test_spatial_roundtrip_with_null_geometry() {
        // Test that null geometry values survive the roundtrip
        use crate::{read_yxdb, SpatialMode};
        use tempfile::NamedTempFile;

        let mut wkb_point = Vec::new();
        wkb_point.push(1u8);
        wkb_point.extend_from_slice(&1u32.to_le_bytes());
        wkb_point.extend_from_slice(&1.0f64.to_le_bytes());
        wkb_point.extend_from_slice(&2.0f64.to_le_bytes());

        let df = test_df(vec![
            Series::new("id".into(), &[1i32, 2, 3]).into(),
            Series::new(
                "geom".into(),
                vec![Some(wkb_point.as_slice()), None, Some(wkb_point.as_slice())],
            )
            .into(),
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["geom"]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Wkb, false).unwrap();
        assert_eq!(df2.height(), 3);

        let geom_col = df2.column("geom").unwrap().binary().unwrap();
        assert!(geom_col.get(0).is_some());
        assert!(geom_col.get(1).is_none());
        assert!(geom_col.get(2).is_some());
    }

    #[test]
    fn test_spatial_roundtrip_linestring() {
        // Test WKB LineString → SHP Polyline → WKB MultiLineString roundtrip
        use crate::{read_yxdb, SpatialMode};
        use tempfile::NamedTempFile;

        // WKB LineString with 3 points
        let mut wkb = Vec::new();
        wkb.push(1u8); // LE
        wkb.extend_from_slice(&2u32.to_le_bytes()); // WKB_LINESTRING
        wkb.extend_from_slice(&3u32.to_le_bytes()); // 3 points
        for &(x, y) in &[(0.0f64, 0.0f64), (1.0, 1.0), (2.0, 0.0)] {
            wkb.extend_from_slice(&x.to_le_bytes());
            wkb.extend_from_slice(&y.to_le_bytes());
        }

        let df = test_df(vec![
            Series::new("line".into(), vec![Some(wkb.as_slice())]).into()
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["line"]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Wkb, false).unwrap();
        let col = df2.column("line").unwrap().binary().unwrap();
        let wkb2 = col.get(0).unwrap();

        // SHP Polyline converts to WKB MultiLineString (type 5)
        let wkb_type = u32::from_le_bytes(wkb2[1..5].try_into().unwrap());
        assert_eq!(wkb_type, 5); // WKB_MULTILINESTRING
    }

    #[test]
    fn test_spatial_roundtrip_polygon() {
        // Test WKB Polygon → SHP Polygon → WKB MultiPolygon roundtrip
        use crate::{read_yxdb, SpatialMode};
        use tempfile::NamedTempFile;

        // WKB Polygon with 1 ring, 4 points (closed ring)
        let mut wkb = Vec::new();
        wkb.push(1u8);
        wkb.extend_from_slice(&3u32.to_le_bytes()); // WKB_POLYGON
        wkb.extend_from_slice(&1u32.to_le_bytes()); // 1 ring
        wkb.extend_from_slice(&4u32.to_le_bytes()); // 4 points
        for &(x, y) in &[(0.0f64, 0.0f64), (4.0, 0.0), (2.0, 3.0), (0.0, 0.0)] {
            wkb.extend_from_slice(&x.to_le_bytes());
            wkb.extend_from_slice(&y.to_le_bytes());
        }

        let df = test_df(vec![
            Series::new("poly".into(), vec![Some(wkb.as_slice())]).into()
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["poly"]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Wkb, false).unwrap();
        let col = df2.column("poly").unwrap().binary().unwrap();
        let wkb2 = col.get(0).unwrap();

        // SHP Polygon converts to WKB MultiPolygon (type 6)
        let wkb_type = u32::from_le_bytes(wkb2[1..5].try_into().unwrap());
        assert_eq!(wkb_type, 6); // WKB_MULTIPOLYGON
    }

    #[test]
    fn test_spatial_roundtrip_multipoint() {
        // Test WKB MultiPoint → SHP MultiPoint → WKB MultiPoint roundtrip
        use crate::{read_yxdb, SpatialMode};
        use tempfile::NamedTempFile;

        // WKB MultiPoint with 2 points
        let mut wkb = Vec::new();
        wkb.push(1u8);
        wkb.extend_from_slice(&4u32.to_le_bytes()); // WKB_MULTIPOINT
        wkb.extend_from_slice(&2u32.to_le_bytes()); // 2 points
                                                    // Each sub-point is a full WKB Point
        for &(x, y) in &[(1.0f64, 2.0f64), (3.0, 4.0)] {
            wkb.push(1u8); // LE
            wkb.extend_from_slice(&1u32.to_le_bytes()); // WKB_POINT
            wkb.extend_from_slice(&x.to_le_bytes());
            wkb.extend_from_slice(&y.to_le_bytes());
        }

        let df = test_df(vec![
            Series::new("pts".into(), vec![Some(wkb.as_slice())]).into()
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["pts"]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Wkb, false).unwrap();
        let col = df2.column("pts").unwrap().binary().unwrap();
        let wkb2 = col.get(0).unwrap();

        let wkb_type = u32::from_le_bytes(wkb2[1..5].try_into().unwrap());
        assert_eq!(wkb_type, 4); // WKB_MULTIPOINT
    }

    #[test]
    fn test_spatial_column_names_from_written_file() {
        // Verify spatial_column_names works on files we write
        use crate::{spatial_column_names, YxdbReader};
        use tempfile::NamedTempFile;

        let mut wkb_point = Vec::new();
        wkb_point.push(1u8);
        wkb_point.extend_from_slice(&1u32.to_le_bytes());
        wkb_point.extend_from_slice(&1.0f64.to_le_bytes());
        wkb_point.extend_from_slice(&2.0f64.to_le_bytes());

        let df = test_df(vec![
            Series::new("id".into(), &[1i32]).into(),
            Series::new("location".into(), vec![Some(wkb_point.as_slice())]).into(),
            Series::new("name".into(), &["test"]).into(),
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["location"]).unwrap();

        let reader = YxdbReader::open(tmp.path()).unwrap();
        let names = spatial_column_names(&reader.fields);
        assert_eq!(names, vec!["location"]);
    }

    #[test]
    fn test_spatial_roundtrip_multiple_spatial_columns() {
        // Test file with two spatial columns
        use crate::{read_yxdb, spatial_column_names, SpatialMode, YxdbReader};
        use tempfile::NamedTempFile;

        let mut wkb_pt1 = Vec::new();
        wkb_pt1.push(1u8);
        wkb_pt1.extend_from_slice(&1u32.to_le_bytes());
        wkb_pt1.extend_from_slice(&1.0f64.to_le_bytes());
        wkb_pt1.extend_from_slice(&2.0f64.to_le_bytes());

        let mut wkb_pt2 = Vec::new();
        wkb_pt2.push(1u8);
        wkb_pt2.extend_from_slice(&1u32.to_le_bytes());
        wkb_pt2.extend_from_slice(&10.0f64.to_le_bytes());
        wkb_pt2.extend_from_slice(&20.0f64.to_le_bytes());

        let df = test_df(vec![
            Series::new("id".into(), &[1i32]).into(),
            Series::new("origin".into(), vec![Some(wkb_pt1.as_slice())]).into(),
            Series::new("dest".into(), vec![Some(wkb_pt2.as_slice())]).into(),
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["origin", "dest"]).unwrap();

        // Verify field metadata
        let reader = YxdbReader::open(tmp.path()).unwrap();
        let names = spatial_column_names(&reader.fields);
        assert_eq!(names, vec!["origin", "dest"]);

        // Read back and verify both columns are decoded
        let df2 = read_yxdb(tmp.path(), SpatialMode::Wkb, false).unwrap();
        assert_eq!(df2.width(), 3);

        for col_name in &["origin", "dest"] {
            let col = df2.column(col_name).unwrap().binary().unwrap();
            let wkb = col.get(0).unwrap();
            assert_eq!(wkb[0], 1); // LE
            let t = u32::from_le_bytes(wkb[1..5].try_into().unwrap());
            assert_eq!(t, 1); // WKB_POINT
        }

        // Verify coordinates
        let origin = df2
            .column("origin")
            .unwrap()
            .binary()
            .unwrap()
            .get(0)
            .unwrap();
        let ox = f64::from_le_bytes(origin[5..13].try_into().unwrap());
        assert!((ox - 1.0).abs() < 1e-10);

        let dest = df2
            .column("dest")
            .unwrap()
            .binary()
            .unwrap()
            .get(0)
            .unwrap();
        let dx = f64::from_le_bytes(dest[5..13].try_into().unwrap());
        assert!((dx - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_read_existing_files_have_correct_file_id() {
        // All existing test files should have valid file_id
        use crate::YxdbReader;
        let test_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test_files");
        for entry in std::fs::read_dir(&test_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().map(|e| e == "yxdb").unwrap_or(false) {
                let reader = YxdbReader::open(&path).unwrap();
                // file_id should be in the Wrigley family (byte 66 == 0x44)
                assert_eq!(
                    reader.header.file_id & 0x00ff0000,
                    0x00440000,
                    "file {:?} has unexpected file_id: 0x{:08X}",
                    path.file_name().unwrap(),
                    reader.header.file_id
                );
            }
        }
    }

    #[test]
    fn test_spatial_mode_all_real_files() {
        // All three spatial modes should work on all existing test files
        use crate::{read_yxdb, SpatialMode};
        let test_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test_files");
        for entry in std::fs::read_dir(&test_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().map(|e| e == "yxdb").unwrap_or(false) {
                let df_raw = read_yxdb(&path, SpatialMode::Raw, false).unwrap();
                let df_wkb = read_yxdb(&path, SpatialMode::Wkb, false).unwrap();
                let df_geo = read_yxdb(&path, SpatialMode::GeoArrow, false).unwrap();
                // All modes should produce the same shape
                assert_eq!(
                    df_raw.shape(),
                    df_wkb.shape(),
                    "Shape mismatch for {:?}",
                    path.file_name().unwrap()
                );
                assert_eq!(
                    df_wkb.shape(),
                    df_geo.shape(),
                    "Shape mismatch for {:?}",
                    path.file_name().unwrap()
                );
            }
        }
    }

    #[test]
    fn test_spatial_point_coordinates_preserved() {
        // Detailed coordinate check: write known coords, read them back
        use crate::{read_yxdb, SpatialMode};
        use tempfile::NamedTempFile;

        let coords: Vec<(f64, f64)> = vec![
            (0.0, 0.0),
            (-180.0, -90.0),
            (180.0, 90.0),
            (-73.985428, 40.748817), // Empire State Building
            (139.6917, 35.6895),     // Tokyo
        ];

        let wkb_points: Vec<Vec<u8>> = coords
            .iter()
            .map(|&(x, y)| {
                let mut wkb = Vec::new();
                wkb.push(1u8);
                wkb.extend_from_slice(&1u32.to_le_bytes());
                wkb.extend_from_slice(&x.to_le_bytes());
                wkb.extend_from_slice(&y.to_le_bytes());
                wkb
            })
            .collect();

        let wkb_refs: Vec<Option<&[u8]>> = wkb_points.iter().map(|v| Some(v.as_slice())).collect();
        let ids: Vec<i32> = (0..coords.len() as i32).collect();

        let df = test_df(vec![
            Series::new("id".into(), ids).into(),
            Series::new("geom".into(), wkb_refs).into(),
        ]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["geom"]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Wkb, false).unwrap();
        let geom_col = df2.column("geom").unwrap().binary().unwrap();

        for (i, &(expected_x, expected_y)) in coords.iter().enumerate() {
            let wkb = geom_col.get(i).unwrap();
            let x = f64::from_le_bytes(wkb[5..13].try_into().unwrap());
            let y = f64::from_le_bytes(wkb[13..21].try_into().unwrap());
            assert!(
                (x - expected_x).abs() < 1e-10,
                "x mismatch at row {i}: expected {expected_x}, got {x}"
            );
            assert!(
                (y - expected_y).abs() < 1e-10,
                "y mismatch at row {i}: expected {expected_y}, got {y}"
            );
        }
    }

    #[test]
    fn test_spatial_ipc_roundtrip_with_all_modes() {
        // Test read_yxdb_to_ipc with all spatial modes
        use crate::{read_yxdb_to_ipc, SpatialMode};
        use tempfile::NamedTempFile;

        let mut wkb_point = Vec::new();
        wkb_point.push(1u8);
        wkb_point.extend_from_slice(&1u32.to_le_bytes());
        wkb_point.extend_from_slice(&42.0f64.to_le_bytes());
        wkb_point.extend_from_slice(&24.0f64.to_le_bytes());

        let df = test_df(vec![Series::new(
            "geom".into(),
            vec![Some(wkb_point.as_slice())],
        )
        .into()]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &["geom"]).unwrap();

        // All three modes should produce valid IPC bytes
        let ipc_raw = read_yxdb_to_ipc(tmp.path(), SpatialMode::Raw, false).unwrap();
        let ipc_wkb = read_yxdb_to_ipc(tmp.path(), SpatialMode::Wkb, false).unwrap();
        let ipc_geo = read_yxdb_to_ipc(tmp.path(), SpatialMode::GeoArrow, false).unwrap();

        assert!(!ipc_raw.is_empty());
        assert!(!ipc_wkb.is_empty());
        assert!(!ipc_geo.is_empty());

        // WKB and GeoArrow should produce the same IPC bytes
        assert_eq!(ipc_wkb, ipc_geo);
        // Raw should differ (SHP vs WKB encoding)
        assert_ne!(ipc_raw, ipc_wkb);
    }

    // ══════════════════════════════════════════════════════════════════
    // Regression tests — audit findings (v0.1.1)
    // ══════════════════════════════════════════════════════════════════

    /// Audit #1 — Int8 sign bit must survive a roundtrip.
    /// Previously Int8 mapped to YXDB Byte (unsigned 0-255) causing
    /// negative values to corrupt.  Now maps to Int16.
    #[test]
    fn regression_int8_sign_preserved() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = df! {
            "vals" => &[-128i8, -1i8, 0i8, 1i8, 127i8]
        }
        .unwrap();

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        // Int8 → Int16 in YXDB → reads back as i16
        let col = df2.column("vals").unwrap().i16().unwrap();
        assert_eq!(col.get(0), Some(-128));
        assert_eq!(col.get(1), Some(-1));
        assert_eq!(col.get(2), Some(0));
        assert_eq!(col.get(3), Some(1));
        assert_eq!(col.get(4), Some(127));
    }

    /// Audit #1 (companion) — Int8 maps to Int16 (signed), NOT Byte (unsigned).
    /// This verifies the schema inference at the type level.
    #[test]
    fn regression_int8_maps_to_int16_not_byte() {
        use polars::prelude::*;

        let int8_df = df! { "s" => &[-1i8, 0i8, 1i8] }.unwrap();
        let fields = super::infer_schema(&int8_df, &[]).unwrap();

        // Int8 → Int16 (2 bytes), NOT Byte (1 byte)
        assert_eq!(fields[0].field_type, crate::field::FieldType::Int16);
        assert_eq!(fields[0].size, 2);
    }

    /// Audit #2 — UInt64 values exceeding i64::MAX must error, not wrap.
    #[test]
    fn regression_uint64_overflow_rejected() {
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = df! {
            "big" => &[u64::MAX]
        }
        .unwrap();

        let tmp = NamedTempFile::new().unwrap();
        let err = write_yxdb(tmp.path(), &df, &[]).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("exceeds"),
            "expected overflow message, got: {msg}"
        );
    }

    /// Audit #2 (companion) — UInt64 values within i64 range succeed.
    #[test]
    fn regression_uint64_within_range_ok() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let max_safe = i64::MAX as u64;
        let df = df! {
            "val" => &[0u64, 42u64, max_safe]
        }
        .unwrap();

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("val").unwrap().i64().unwrap();
        assert_eq!(col.get(0), Some(0));
        assert_eq!(col.get(1), Some(42));
        assert_eq!(col.get(2), Some(i64::MAX));
    }

    /// Audit #5 — Large binary blob (> BLOCK_SIZE) roundtrips correctly.
    /// Previously oversized records caused LZF buffer overflows.
    #[test]
    fn regression_large_blob_roundtrip() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Create a blob larger than BLOCK_SIZE (262144)
        let big_blob: Vec<u8> = (0..300_000u32).map(|i| (i % 256) as u8).collect();
        let df = test_df(vec![Column::new("data".into(), vec![big_blob.as_slice()])]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("data").unwrap().binary().unwrap();
        let read_blob = col.get(0).unwrap();
        assert_eq!(read_blob.len(), 300_000);
        assert_eq!(read_blob, big_blob.as_slice());
    }

    /// Audit #7 — Empty DataFrame (0 rows) roundtrips preserving schema.
    #[test]
    fn regression_empty_dataframe_roundtrip() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let empty_i32: Vec<i32> = vec![];
        let empty_str: Vec<&str> = vec![];
        let df = df! {
            "id" => empty_i32,
            "name" => empty_str
        }
        .unwrap();
        assert_eq!(df.height(), 0);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.height(), 0);
        assert_eq!(df2.width(), 2);
        assert_eq!(df2.get_column_names()[0].as_str(), "id");
        assert_eq!(df2.get_column_names()[1].as_str(), "name");
    }

    /// Audit #12 — Duration columns roundtrip as Int64 microseconds.
    #[test]
    fn regression_duration_roundtrip() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Duration in microseconds
        let values: Vec<Option<i64>> = vec![Some(1_000_000), Some(-500_000), None, Some(0)];
        let series = Series::new("dur".into(), &values)
            .cast(&DataType::Duration(TimeUnit::Microseconds))
            .unwrap();
        let df = test_df(vec![series.into()]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("dur").unwrap().i64().unwrap();
        assert_eq!(col.get(0), Some(1_000_000));
        assert_eq!(col.get(1), Some(-500_000));
        assert_eq!(col.get(2), None);
        assert_eq!(col.get(3), Some(0));
    }

    /// Audit #12 (companion) — Duration nanoseconds are normalized to microseconds.
    #[test]
    fn regression_duration_nanoseconds_normalized() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // 5_000_000 ns = 5_000 µs
        let values: Vec<Option<i64>> = vec![Some(5_000_000)];
        let series = Series::new("dur_ns".into(), &values)
            .cast(&DataType::Duration(TimeUnit::Nanoseconds))
            .unwrap();
        let df = test_df(vec![series.into()]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("dur_ns").unwrap().i64().unwrap();
        assert_eq!(col.get(0), Some(5_000)); // normalized from ns → µs
    }

    /// Audit #12 (companion) — Duration milliseconds are normalized to microseconds.
    #[test]
    fn regression_duration_milliseconds_normalized() {
        use crate::{read_yxdb, SpatialMode};
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // 3 ms = 3_000 µs
        let values: Vec<Option<i64>> = vec![Some(3)];
        let series = Series::new("dur_ms".into(), &values)
            .cast(&DataType::Duration(TimeUnit::Milliseconds))
            .unwrap();
        let df = test_df(vec![series.into()]);

        let tmp = NamedTempFile::new().unwrap();
        write_yxdb(tmp.path(), &df, &[]).unwrap();

        let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
        let col = df2.column("dur_ms").unwrap().i64().unwrap();
        assert_eq!(col.get(0), Some(3_000)); // normalized from ms → µs
    }

    /// Audit #18 — Drop no longer prints to stderr.
    /// (Structural test: a writer that goes out of scope without finish()
    /// should not panic and should not produce output on stderr.)
    #[test]
    fn regression_drop_does_not_panic() {
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let df = df! { "x" => [1i32] }.unwrap();
        let tmp = NamedTempFile::new().unwrap();

        {
            let mut writer = YxdbWriter::new(tmp.path(), &df).unwrap();
            writer.write_batch(&df).unwrap();
            // Drop without calling finish() — should not panic
        }
        // If we reach here, the drop didn't panic
    }
}
