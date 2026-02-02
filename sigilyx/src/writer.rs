use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use polars::prelude::*;

use crate::error::{Result, YxdbError};
use crate::field::{FieldMeta, FieldType};
use crate::header::HEADER_SIZE;
use crate::lzf;

/// Maximum uncompressed block size before flushing to disk.
/// Matches the reader's default buffer: 256 KiB.
const BLOCK_SIZE: usize = 0x40000; // 262144

// ── Public API ─────────────────────────────────────────────────────────

/// Write a Polars [`DataFrame`] to a YXDB file.
///
/// The schema is inferred from the DataFrame column types using sensible
/// defaults. Use [`write_yxdb_with_schema`] for explicit control.
pub fn write_yxdb<P: AsRef<Path>>(path: P, df: &DataFrame) -> Result<()> {
    let fields = infer_schema(df)?;
    write_yxdb_impl(path.as_ref(), df, &fields)
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
    let cursor = std::io::Cursor::new(ipc_bytes);
    let df = IpcReader::new(cursor)
        .finish()
        .map_err(|e| YxdbError::ConversionError(format!("failed to read IPC bytes: {e}")))?;
    write_yxdb(path, &df)
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
    record_count: u64,
    block_buf: Vec<u8>,
}

impl YxdbWriter<BufWriter<File>> {
    /// Create a new YXDB writer for a file path.
    ///
    /// The schema is inferred from the provided template DataFrame.
    pub fn new<P: AsRef<Path>>(path: P, template_df: &DataFrame) -> Result<Self> {
        let fields = infer_schema(template_df)?;
        Self::with_schema(path, &fields)
    }

    /// Create a new YXDB writer with an explicit schema.
    pub fn with_schema<P: AsRef<Path>>(path: P, fields: &[FieldMeta]) -> Result<Self> {
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
        let header = build_header(0, meta_info_size);
        writer.write_all(&header)?;

        // Write UTF-16LE XML metadata
        writer.write_all(&utf16_bytes)?;

        Ok(Self {
            writer,
            fields: fields.to_vec(),
            fixed_size,
            has_var,
            record_count: 0,
            block_buf: Vec::with_capacity(BLOCK_SIZE),
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

        // Build column references
        let columns: Vec<&Column> = batch.get_columns().iter().collect();

        for row in 0..num_rows {
            let record = build_record(&self.fields, &columns, self.fixed_size, self.has_var, row)?;
            
            // Check if adding this record would exceed block size
            if self.block_buf.len() + record.len() > BLOCK_SIZE && !self.block_buf.is_empty() {
                self.flush_block()?;
            }
            
            self.block_buf.extend_from_slice(&record);
        }

        self.record_count += num_rows as u64;
        Ok(())
    }

    /// Flush the current block to disk.
    fn flush_block(&mut self) -> Result<()> {
        if self.block_buf.is_empty() {
            return Ok(());
        }

        // Try to compress
        if let Some(compressed) = lzf::compress(&self.block_buf) {
            // Write compressed block length (without high bit)
            let len = compressed.len() as u32;
            self.writer.write_all(&len.to_le_bytes())?;
            self.writer.write_all(&compressed)?;
        } else {
            // Write uncompressed block length (with high bit set)
            let len = (self.block_buf.len() as u32) | 0x80000000;
            self.writer.write_all(&len.to_le_bytes())?;
            self.writer.write_all(&self.block_buf)?;
        }

        self.block_buf.clear();
        Ok(())
    }

    /// Finish writing and update the header with the final record count.
    ///
    /// This must be called to produce a valid YXDB file.
    pub fn finish(mut self) -> Result<()> {
        // Flush any remaining data
        self.flush_block()?;

        // Seek back to header and update record count
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

// ── Schema inference ───────────────────────────────────────────────────

/// Infer a YXDB field schema from a Polars DataFrame.
fn infer_schema(df: &DataFrame) -> Result<Vec<FieldMeta>> {
    let mut fields = Vec::with_capacity(df.width());
    let mut offset = 0;

    for col in df.get_columns() {
        let name = col.name().to_string();
        let dtype = col.dtype();

        let (field_type, size, scale) = match dtype {
            DataType::Boolean => (FieldType::Bool, 1, 0),
            DataType::Int8 | DataType::UInt8 => (FieldType::Byte, 1, 0),
            DataType::Int16 | DataType::UInt16 => (FieldType::Int16, 2, 0),
            DataType::Int32 | DataType::UInt32 => (FieldType::Int32, 4, 0),
            DataType::Int64 | DataType::UInt64 => (FieldType::Int64, 8, 0),
            DataType::Float32 => (FieldType::Float, 4, 0),
            DataType::Float64 => (FieldType::Double, 8, 0),
            DataType::String => {
                // Use V_WString for variable-length strings
                (FieldType::VWString, 2147483647, 0)
            }
            DataType::Date => (FieldType::Date, 10, 0),
            DataType::Datetime(_, _) => (FieldType::DateTime, 19, 0),
            DataType::Time => (FieldType::Time, 8, 0),
            DataType::Binary => (FieldType::Blob, 0, 0),
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

// ── Core writer ────────────────────────────────────────────────────────

fn write_yxdb_impl(path: &Path, df: &DataFrame, fields: &[FieldMeta]) -> Result<()> {
    let num_records = df.height() as u64;

    // Build the XML metadata string
    let xml = build_meta_xml(fields);

    // Encode XML as UTF-16LE with null terminator
    let utf16_bytes = encode_utf16_le(&xml);
    // meta_info_size includes the null terminator (1 extra u16)
    let meta_info_size = (xml.len() + 1) as u32; // count of UTF-16 code units

    // Build the 512-byte header
    let header = build_header(num_records, meta_info_size);

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

    // Serialize records into LZF-compressed blocks
    write_records(&mut writer, df, fields, fixed_size, has_var, num_records)?;

    writer.flush()?;
    Ok(())
}

// ── Header construction ────────────────────────────────────────────────

// The YXDB format uses "Alteryx Database File" as a magic identifier (like PNG or PDF magic bytes).
// This is the minimum required for format identification and interoperability.
// We intentionally do NOT include the copyright notice from Alteryx's implementation.
const MAGIC: &[u8] = b"Alteryx Database File";

fn build_header(num_records: u64, meta_info_size: u32) -> [u8; HEADER_SIZE] {
    let mut header = [0u8; HEADER_SIZE];

    // Magic string at offset 0 (required for format identification)
    // Only the 21-byte identifier, not any copyright text
    header[..MAGIC.len()].copy_from_slice(MAGIC);

    // Bytes 64-71: File version/type identifiers
    header[64] = 0x04;
    header[65] = 0x02;

    // meta_info_size at offset 80 (u32 LE)
    header[80..84].copy_from_slice(&meta_info_size.to_le_bytes());

    // Record info area (bytes 84..104 typically contain spatial index info)
    // We leave them zero for non-spatial files

    // num_records at offset 104 (u64 LE)
    header[104..112].copy_from_slice(&num_records.to_le_bytes());

    header
}

// ── XML metadata ───────────────────────────────────────────────────────

fn build_meta_xml(fields: &[FieldMeta]) -> String {
    let mut xml = String::with_capacity(256);
    // Match the exact format Alteryx uses
    xml.push_str("\n<RecordInfo>\n");

    for field in fields {
        xml.push_str("\t<Field name=\"");
        xml_escape_into(&field.name, &mut xml);
        xml.push_str("\" ");

        // source attribute (Alteryx includes this)
        xml.push_str("source=\"SigilYX\" ");

        xml.push_str("type=\"");
        xml.push_str(field_type_to_xml_str(field.field_type));
        xml.push('"');

        // Size attribute (not for Bool)
        match field.field_type {
            FieldType::Bool => {}
            FieldType::Byte | FieldType::Int16 | FieldType::Int32 | FieldType::Int64
            | FieldType::Float | FieldType::Double => {
                xml.push_str(&format!(" size=\"{}\"", field.size));
            }
            FieldType::FixedDecimal => {
                xml.push_str(&format!(" size=\"{}\" scale=\"{}\"", field.size, field.scale));
            }
            FieldType::String | FieldType::WString => {
                xml.push_str(&format!(" size=\"{}\"", field.size));
            }
            FieldType::VString | FieldType::VWString => {
                xml.push_str(&format!(" size=\"{}\"", field.size));
            }
            FieldType::Date | FieldType::Time | FieldType::DateTime => {
                xml.push_str(&format!(" size=\"{}\"", field.size));
            }
            FieldType::Blob | FieldType::SpatialObj => {
                xml.push_str(&format!(" size=\"{}\"", field.size));
            }
        }

        xml.push_str(" />\n");
    }

    xml.push_str("</RecordInfo>\n");
    xml
}

fn field_type_to_xml_str(ft: FieldType) -> &'static str {
    match ft {
        FieldType::Bool => "Bool",
        FieldType::Byte => "Byte",
        FieldType::Int16 => "Int16",
        FieldType::Int32 => "Int32",
        FieldType::Int64 => "Int64",
        FieldType::Float => "Float",
        FieldType::Double => "Double",
        FieldType::FixedDecimal => "FixedDecimal",
        FieldType::String => "String",
        FieldType::WString => "WString",
        FieldType::VString => "V_String",
        FieldType::VWString => "V_WString",
        FieldType::Date => "Date",
        FieldType::Time => "Time",
        FieldType::DateTime => "DateTime",
        FieldType::Blob => "Blob",
        FieldType::SpatialObj => "SpatialObj",
    }
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

fn write_records<W: Write>(
    writer: &mut W,
    df: &DataFrame,
    fields: &[FieldMeta],
    fixed_size: usize,
    has_var: bool,
    num_records: u64,
) -> Result<()> {
    let columns: Vec<&Column> = df.get_columns().iter().collect();

    // We accumulate raw record bytes into a block buffer, and flush
    // as LZF-compressed blocks when the buffer exceeds BLOCK_SIZE.
    let mut block_buf: Vec<u8> = Vec::with_capacity(BLOCK_SIZE + 4096);

    for row in 0..num_records as usize {
        // Build one record: fixed portion + variable data
        let record = build_record(fields, &columns, fixed_size, has_var, row)?;
        block_buf.extend_from_slice(&record);

        // Flush block if it exceeds the threshold
        if block_buf.len() >= BLOCK_SIZE {
            flush_block(writer, &block_buf)?;
            block_buf.clear();
        }
    }

    // Flush remaining data
    if !block_buf.is_empty() {
        flush_block(writer, &block_buf)?;
    }

    Ok(())
}

/// Build a single record: fixed portion + optional var_len header + var data.
fn build_record(
    fields: &[FieldMeta],
    columns: &[&Column],
    fixed_size: usize,
    has_var: bool,
    row: usize,
) -> Result<Vec<u8>> {
    let mut fixed = vec![0u8; fixed_size];
    let mut var_data: Vec<u8> = Vec::new();

    // Track which fields are variable so we can fix up offsets later
    let mut var_fixups: Vec<(usize, usize)> = Vec::new(); // (field_offset, var_data_offset)

    for (col_idx, field) in fields.iter().enumerate() {
        let col = columns[col_idx];
        serialize_field_into(
            &mut fixed,
            &mut var_data,
            &mut var_fixups,
            field,
            col,
            row,
        )?;
    }

    // Now fix up the variable-field offsets in the fixed portion.
    // For each variable field, the reader does:
    //   block_start = field_offset + (fixed_val & 0x7FFFFFFF)
    // So fixed_val = (target - field_offset) | 0x80000000
    // where target = fixed_size + 4 (var_len header) + var_data_offset_within_var_data
    if has_var {
        for (field_offset, var_data_start) in &var_fixups {
            let target = fixed_size + 4 + var_data_start;
            let offset_from_field = target - field_offset;
            let fixed_val = (offset_from_field as u32) | 0x80000000;
            fixed[*field_offset..*field_offset + 4]
                .copy_from_slice(&fixed_val.to_le_bytes());
        }
    }

    // Assemble final record
    let mut record = fixed;
    if has_var {
        let var_len = var_data.len() as u32;
        record.extend_from_slice(&var_len.to_le_bytes());
        record.extend_from_slice(&var_data);
    }

    Ok(record)
}

/// Serialize a single field value into the fixed and variable buffers.
fn serialize_field_into(
    fixed: &mut Vec<u8>,
    var_data: &mut Vec<u8>,
    var_fixups: &mut Vec<(usize, usize)>,
    field: &FieldMeta,
    col: &Column,
    row: usize,
) -> Result<()> {
    let off = field.offset;

    match field.field_type {
        FieldType::Bool => {
            let series = col.bool().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(true) => fixed[off] = 1,
                Some(false) => fixed[off] = 0,
                None => fixed[off] = 2,
            }
        }

        FieldType::Byte => {
            let series = col.i16().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(v) => {
                    fixed[off] = v as u8;
                    fixed[off + 1] = 0;
                }
                None => {
                    fixed[off] = 0;
                    fixed[off + 1] = 1;
                }
            }
        }

        FieldType::Int16 => {
            let series = col.i16().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
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
            let series = col.i32().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
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

        FieldType::Int64 => {
            let series = col.i64().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
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

        FieldType::Float => {
            let series = col.f32().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
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
            let series = col.f64().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
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
            let series = col.f64().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(v) => {
                    let s = format!("{:.*}", field.scale, v);
                    write_fixed_ascii(&mut fixed[off..off + field.size], &s);
                    fixed[off + field.size] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + field.size] { *b = 0; }
                    fixed[off + field.size] = 1;
                }
            }
        }

        FieldType::String => {
            let series = col.str().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(s) => {
                    write_fixed_ascii(&mut fixed[off..off + field.size], s);
                    fixed[off + field.size] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + field.size] { *b = 0; }
                    fixed[off + field.size] = 1;
                }
            }
        }

        FieldType::WString => {
            let series = col.str().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            let byte_len = field.size * 2;
            match series.get(row) {
                Some(s) => {
                    write_fixed_wstring(&mut fixed[off..off + byte_len], s, field.size);
                    fixed[off + byte_len] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + byte_len] { *b = 0; }
                    fixed[off + byte_len] = 1;
                }
            }
        }

        FieldType::VString => {
            let series = col.str().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(s) if s.is_empty() => {
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
            let series = col.str().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(s) if s.is_empty() => {
                    fixed[off..off + 4].copy_from_slice(&0u32.to_le_bytes());
                }
                Some(s) => {
                    let utf16: Vec<u16> = s.encode_utf16().collect();
                    let mut bytes = Vec::with_capacity(utf16.len() * 2);
                    for cu in &utf16 {
                        bytes.extend_from_slice(&cu.to_le_bytes());
                    }
                    let var_offset = var_data.len();
                    var_fixups.push((off, var_offset));
                    append_var_block(var_data, &bytes);
                    fixed[off..off + 4].copy_from_slice(&0u32.to_le_bytes());
                }
                None => {
                    fixed[off..off + 4].copy_from_slice(&1u32.to_le_bytes());
                }
            }
        }

        FieldType::Date => {
            let series = col.date().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(days) => {
                    let date_str = days_to_date_str(days);
                    write_fixed_ascii(&mut fixed[off..off + 10], &date_str);
                    fixed[off + 10] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + 10] { *b = 0; }
                    fixed[off + 10] = 1;
                }
            }
        }

        FieldType::Time => {
            let series = col.str().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(s) => {
                    write_fixed_ascii(&mut fixed[off..off + 8], s);
                    fixed[off + 8] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + 8] { *b = 0; }
                    fixed[off + 8] = 1;
                }
            }
        }

        FieldType::DateTime => {
            let series = col.datetime().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(ms) => {
                    let dt_str = ms_to_datetime_str(ms);
                    write_fixed_ascii(&mut fixed[off..off + 19], &dt_str);
                    fixed[off + 19] = 0;
                }
                None => {
                    for b in &mut fixed[off..off + 19] { *b = 0; }
                    fixed[off + 19] = 1;
                }
            }
        }

        FieldType::Blob | FieldType::SpatialObj => {
            let series = col.binary().map_err(|e| YxdbError::ConversionError(e.to_string()))?;
            match series.get(row) {
                Some(data) if data.is_empty() => {
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

/// Write a single block to the output, with LZF compression if beneficial.
fn flush_block<W: Write>(writer: &mut W, data: &[u8]) -> Result<()> {
    match lzf::compress(data) {
        Some(compressed) => {
            // Compressed block: length without high bit
            let len = compressed.len() as u32;
            writer.write_all(&len.to_le_bytes())?;
            writer.write_all(&compressed)?;
        }
        None => {
            // Uncompressed block: set high bit
            let len = data.len() as u32 | 0x80000000;
            writer.write_all(&len.to_le_bytes())?;
            writer.write_all(data)?;
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
    if data.len() <= 127 {
        var_data.push(((data.len() as u8) << 1) | 1);
        var_data.extend_from_slice(data);
    } else {
        let raw_len = (data.len() * 2) as u32;
        var_data.extend_from_slice(&raw_len.to_le_bytes());
        var_data.extend_from_slice(data);
    }
}

// ── Helper functions ───────────────────────────────────────────────────

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
    let code_units: Vec<u16> = s.encode_utf16().take(max_chars).collect();
    let mut pos = 0;
    for cu in &code_units {
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

/// Convert milliseconds since Unix epoch to "YYYY-MM-DD HH:MM:SS" string.
fn ms_to_datetime_str(ms: i64) -> String {
    let total_secs = ms / 1000;
    let days = (total_secs / 86400) as i32;
    let day_secs = (total_secs % 86400) as u32;
    let (y, m, d) = days_to_civil(days);
    let h = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, h, min, s)
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
        let ms = 20162i64 * 86_400_000 + 8 * 3_600_000 + 30 * 60_000;
        assert_eq!(ms_to_datetime_str(ms), "2025-03-15 08:30:00");
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
        use crate::read_yxdb;
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
        write_yxdb(path, &df).unwrap();

        // Read it back
        let df2 = read_yxdb(path).unwrap();

        // Compare
        assert_eq!(df2.height(), 3);
        assert_eq!(df2.width(), 2);

        let id_col = df2.column("id").unwrap();
        let id_vals: Vec<i32> = id_col.i32().unwrap().into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(id_vals, vec![1, 2, 3]);

        let name_col = df2.column("name").unwrap();
        let name_vals: Vec<&str> = name_col.str().unwrap().into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(name_vals, vec!["Alice", "Bob", "Charlie"]);
    }

    #[test]
    fn test_roundtrip_with_nulls() {
        use crate::read_yxdb;
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Create a DataFrame with nulls
        let df = df! {
            "val" => [Some(10i32), None, Some(30)],
        }
        .unwrap();

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();
        write_yxdb(path, &df).unwrap();

        let df2 = read_yxdb(path).unwrap();
        assert_eq!(df2.height(), 3);

        let val_col = df2.column("val").unwrap();
        let vals: Vec<Option<i32>> = val_col.i32().unwrap().into_iter().collect();
        assert_eq!(vals, vec![Some(10), None, Some(30)]);
    }

    #[test]
    fn test_roundtrip_multiple_types() {
        use crate::read_yxdb;
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
        write_yxdb(tmp.path(), &df).unwrap();

        let df2 = read_yxdb(tmp.path()).unwrap();
        assert_eq!(df2.height(), 3);
        assert_eq!(df2.width(), 7);

        // Check bool
        let bools: Vec<bool> = df2.column("bool_col").unwrap().bool().unwrap().into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(bools, vec![true, false, true]);

        // Check i32
        let i32s: Vec<i32> = df2.column("i32_col").unwrap().i32().unwrap().into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(i32s, vec![100, 200, 300]);

        // Check strings
        let strs: Vec<&str> = df2.column("str_col").unwrap().str().unwrap().into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(strs, vec!["a", "bb", "ccc"]);
    }

    #[test]
    fn test_streaming_writer() {
        use crate::read_yxdb;
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        // Create batches
        let batch1 = df! {
            "id" => [1i32, 2, 3],
            "name" => ["Alice", "Bob", "Charlie"],
        }.unwrap();

        let batch2 = df! {
            "id" => [4i32, 5],
            "name" => ["David", "Eve"],
        }.unwrap();

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
        let df = read_yxdb(tmp.path()).unwrap();
        assert_eq!(df.height(), 5);
        assert_eq!(df.width(), 2);

        let ids: Vec<i32> = df.column("id").unwrap().i32().unwrap().into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);

        let names: Vec<&str> = df.column("name").unwrap().str().unwrap().into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(names, vec!["Alice", "Bob", "Charlie", "David", "Eve"]);
    }

    #[test]
    fn test_streaming_writer_many_batches() {
        use crate::read_yxdb;
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

        let df = read_yxdb(tmp.path()).unwrap();
        assert_eq!(df.height(), 1000);

        let values: Vec<i64> = df.column("value").unwrap().i64().unwrap().into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(values, (0..1000).collect::<Vec<_>>());
    }
}
