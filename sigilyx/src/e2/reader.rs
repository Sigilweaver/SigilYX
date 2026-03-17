//! E2 YXDB reader — block decompression and record framing.
//!
//! The E2 format uses Snappy-compressed blocks with compact variable-length
//! record encoding. This module handles:
//! - Reading blocks (type 0x02 Snappy, type 0x01 blob, type 0x00 sentinel)
//! - Snappy decompression
//! - Record framing (inter-record u32 LE size prefixes)
//! - DataFrame construction via Polars

use std::io::{BufReader, Read};
use std::path::Path;

use polars::prelude::*;

use super::header::{self, E2Header, HEADER_SIZE};
use super::record::{self, is_e2_verified_type, FieldValue};
use crate::error::{Result, YxdbError};
use crate::field::{FieldMeta, FieldType};

/// An E2 YXDB reader.
///
/// Reads E2-format YXDB files (magic "Alteryx e2 Database file"),
/// decompresses Snappy blocks, and decodes compact-encoded records.
pub struct E2Reader {
    stream: BufReader<std::fs::File>,
    pub header: E2Header,
    pub fields: Vec<FieldMeta>,
    pub meta_xml: String,
    /// Whether the first Date field in each record has a preceding 0x00 flag byte.
    has_date_flag: bool,
    /// Blob data from type 0x01 blocks (if any).
    blob_data: Option<Vec<u8>>,
    /// Whether to allow reading unverified E2 field types.
    allow_unverified: bool,
}

impl E2Reader {
    /// Open an E2 YXDB file for reading.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = std::fs::File::open(path.as_ref())?;
        let mut stream = BufReader::new(file);

        // Read 100-byte header
        let mut header_buf = [0u8; HEADER_SIZE];
        match stream.read_exact(&mut header_buf) {
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(YxdbError::InvalidFile(
                    "file too small to be a valid E2 YXDB (< 100 bytes)".into(),
                ));
            }
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
        let header = E2Header::parse(&header_buf)?;

        // Read UTF-8 metadata (size is in bytes)
        let meta_size = header.metadata_size as usize;
        let mut meta_bytes = vec![0u8; meta_size];
        stream.read_exact(&mut meta_bytes)?;

        let meta_xml = String::from_utf8(meta_bytes)
            .map_err(|e| YxdbError::InvalidFile(format!("E2 metadata is not valid UTF-8: {e}")))?;

        let fields = header::parse_meta_xml(&meta_xml)?;

        Ok(Self {
            stream,
            header,
            fields,
            meta_xml,
            has_date_flag: false,
            blob_data: None,
            allow_unverified: false,
        })
    }

    /// Set whether to allow reading unverified E2 field types.
    ///
    /// By default, E2 files containing field types that have never been
    /// verified against real corpus data (Time, WString, Blob, SpatialObj)
    /// will produce an error. Call this with `true` to attempt reading
    /// them anyway — the decoders are speculative and may produce incorrect
    /// results.
    pub fn set_allow_unverified(&mut self, allow: bool) {
        self.allow_unverified = allow;
    }

    /// Check that all field types in this file have been verified against
    /// real E2 corpus data. Returns an error listing any unverified types.
    fn check_verified_types(&self) -> Result<()> {
        let unverified: Vec<&str> = self
            .fields
            .iter()
            .filter(|f| !is_e2_verified_type(f.field_type))
            .map(|f| f.field_type.as_xml_str())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        if unverified.is_empty() {
            return Ok(());
        }

        Err(YxdbError::InvalidFile(format!(
            "this E2 file contains field types whose decoders have never been \
             verified against real data: {}. Reading may produce incorrect \
             results. To attempt reading anyway, set allow_unverified_e2_types=True.",
            unverified.join(", ")
        )))
    }

    /// Read all records and return a Polars DataFrame.
    pub fn into_dataframe(mut self) -> Result<DataFrame> {
        if !self.allow_unverified {
            self.check_verified_types()?;
        }

        let fields = self.fields.clone();
        let n_fields = fields.len();

        // Column builders: one Vec<FieldValue> per column
        let mut columns: Vec<Vec<FieldValue>> = vec![Vec::new(); n_fields];

        // Read all blocks
        let mut first_block = true;
        while let Some(block) = self.read_block()? {
            match block {
                Block::Record(decompressed) => {
                    let records = self.frame_records(&decompressed)?;

                    // Auto-detect date flag on first block
                    if first_block && !records.is_empty() {
                        self.detect_date_flag(records[0]);
                        first_block = false;
                    }

                    for rec_data in &records {
                        match self.decode_record(rec_data) {
                            Ok(row) => {
                                for (col_idx, val) in row.into_iter().enumerate() {
                                    columns[col_idx].push(val);
                                }
                            }
                            Err(_) => {
                                // Skip corrupted records by inserting nulls
                                // (spec documents 1 anomalous record in Task1)
                                for (col_idx, field) in fields.iter().enumerate() {
                                    columns[col_idx].push(null_field_value(field.field_type));
                                }
                            }
                        }
                    }
                }
                Block::Blob(data) => {
                    self.blob_data = Some(data);
                }
            }
        }

        // Build Polars Series from column vectors
        let height = if n_fields > 0 { columns[0].len() } else { 0 };
        let cols: Vec<Column> = fields
            .iter()
            .zip(columns.into_iter())
            .map(|(field, vals)| {
                field_values_to_series(&field.name, field.field_type, vals).map(|s| s.into_column())
            })
            .collect::<Result<Vec<_>>>()?;

        if cols.is_empty() {
            return Ok(DataFrame::empty());
        }

        DataFrame::new(height, cols)
            .map_err(|e| YxdbError::ConversionError(format!("failed to build DataFrame: {e}")))
    }

    /// Read a single block from the stream.
    ///
    /// Returns `None` for 0x00 sentinel or EOF.
    fn read_block(&mut self) -> Result<Option<Block>> {
        let mut type_byte = [0u8; 1];
        match self.stream.read_exact(&mut type_byte) {
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }

        match type_byte[0] {
            0x00 => Ok(None),
            0x01 => {
                // Blob block: Snappy data may extend past declared block_size (per spec).
                let mut size_buf = [0u8; 4];
                self.stream.read_exact(&mut size_buf)?;
                let block_size = u32::from_le_bytes(size_buf) as usize;

                // Read declared size + generous extra for Snappy overshoot
                let read_size = block_size + 256;
                let mut block_data = vec![0u8; read_size];
                let mut total_read = 0;
                while total_read < read_size {
                    match self.stream.read(&mut block_data[total_read..]) {
                        Ok(0) => break,
                        Ok(n) => total_read += n,
                        Err(e) => return Err(e.into()),
                    }
                }
                block_data.truncate(total_read);

                if block_data.len() < 21 {
                    return Err(YxdbError::InvalidFile(
                        "E2 type 0x01 block too small".into(),
                    ));
                }

                // Skip uncompressed_size(4) + hash(16) + 0x0A marker(1) = 21 bytes
                // Try decompression with increasing input sizes because the Snappy
                // stream may extend past the declared block_size, but trailing bytes
                // (from the next block/footer) corrupt the decoder.
                let snappy_offset = 21;
                let base_len = block_size.saturating_sub(snappy_offset);
                let max_len = total_read.saturating_sub(snappy_offset);

                let mut decompressed = None;
                let mut actual_snappy_len = 0;
                for try_len in base_len..=max_len {
                    let snappy_data = &block_data[snappy_offset..snappy_offset + try_len];
                    match snap::raw::Decoder::new().decompress_vec(snappy_data) {
                        Ok(data) => {
                            decompressed = Some(data);
                            actual_snappy_len = try_len;
                            break;
                        }
                        Err(_) => continue,
                    }
                }

                let decompressed = decompressed.ok_or_else(|| {
                    YxdbError::InvalidFile(
                        "E2 Snappy decompression failed (blob block): all input sizes failed"
                            .into(),
                    )
                })?;

                // Seek to the correct position: header(21) + actual snappy data
                use std::io::{Seek, SeekFrom};
                let consumed = snappy_offset + actual_snappy_len;
                let overshoot = total_read as i64 - consumed as i64;
                if overshoot > 0 {
                    self.stream.seek(SeekFrom::Current(-overshoot))?;
                }

                Ok(Some(Block::Blob(decompressed)))
            }
            0x02 => {
                // Record block
                let mut size_buf = [0u8; 4];
                self.stream.read_exact(&mut size_buf)?;
                let block_size = u32::from_le_bytes(size_buf) as usize;

                let mut block_data = vec![0u8; block_size];
                self.stream.read_exact(&mut block_data)?;

                if block_data.is_empty() || block_data[0] != 0x0A {
                    return Err(YxdbError::InvalidFile(
                        "E2 type 0x02 block missing 0x0A marker".into(),
                    ));
                }

                let snappy_data = &block_data[1..];
                let decompressed = snap::raw::Decoder::new()
                    .decompress_vec(snappy_data)
                    .map_err(|e| {
                        YxdbError::InvalidFile(format!("E2 Snappy decompression failed: {e}"))
                    })?;

                Ok(Some(Block::Record(decompressed)))
            }
            other => Err(YxdbError::InvalidFile(format!(
                "unknown E2 block type: 0x{other:02X}"
            ))),
        }
    }

    /// Frame records from decompressed block data.
    fn frame_records<'a>(&self, decompressed: &'a [u8]) -> Result<Vec<&'a [u8]>> {
        if decompressed.len() < 12 {
            return Err(YxdbError::InvalidFile(
                "E2 decompressed block too small for header".into(),
            ));
        }

        let _inner_size = u32::from_le_bytes(decompressed[0..4].try_into().unwrap()) & 0x7FFF_FFFF;
        let record_count = u32::from_le_bytes(decompressed[4..8].try_into().unwrap()) as usize;
        let first_record_size =
            (u32::from_le_bytes(decompressed[8..12].try_into().unwrap()) & 0x7FFF_FFFF) as usize;

        if record_count == 0 {
            return Ok(Vec::new());
        }

        let mut records = Vec::with_capacity(record_count);
        let mut pos = 12;

        // First record
        let end = pos + first_record_size;
        if end > decompressed.len() {
            return Err(YxdbError::InvalidFile(format!(
                "E2 first record extends past block end ({end} > {})",
                decompressed.len()
            )));
        }
        records.push(&decompressed[pos..end]);
        pos = end;

        // Subsequent records: [u32 LE size] [record data]
        for i in 1..record_count {
            if pos + 4 > decompressed.len() {
                return Err(YxdbError::InvalidFile(format!(
                    "E2 record {i}: not enough bytes for size prefix"
                )));
            }
            let rec_size =
                u32::from_le_bytes(decompressed[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;

            let end = pos + rec_size;
            if end > decompressed.len() {
                return Err(YxdbError::InvalidFile(format!(
                    "E2 record {i}: extends past block end ({end} > {})",
                    decompressed.len()
                )));
            }
            records.push(&decompressed[pos..end]);
            pos = end;
        }

        Ok(records)
    }

    /// Auto-detect the date flag byte by trying both interpretations.
    ///
    /// Compares how many bytes each interpretation consumes before encountering
    /// an error. The interpretation that decodes further is chosen. This is
    /// robust even when files have undocumented extra fields (e.g., Task1's
    /// extra Int64) that prevent either interpretation from consuming the
    /// entire record.
    fn detect_date_flag(&mut self, record_data: &[u8]) {
        let has_date = self.fields.iter().any(|f| f.field_type == FieldType::Date);
        if !has_date {
            self.has_date_flag = false;
            return;
        }

        let without = self.try_decode_consumed(record_data, false);
        let with = self.try_decode_consumed(record_data, true);

        self.has_date_flag = with > without;
    }

    /// Try decoding a record, returning the total bytes consumed.
    ///
    /// On error, returns the offset reached before the error (partial decode).
    fn try_decode_consumed(&self, record_data: &[u8], has_date_flag: bool) -> usize {
        let mut offset = 0;
        let mut is_first_date = true;

        for field in &self.fields {
            let is_date = field.field_type == FieldType::Date;
            match record::decode_field(
                record_data,
                offset,
                field.field_type,
                is_date && is_first_date,
                has_date_flag,
            ) {
                Ok((_, consumed)) => {
                    offset += consumed;
                    if is_date {
                        is_first_date = false;
                    }
                }
                Err(_) => break,
            }
        }
        offset
    }

    /// Decode all fields from a single record.
    ///
    /// Uses adaptive recovery for undocumented extra Int64 fields that appear
    /// before string fields in some files (see spec finding #10, Task1 anomaly).
    fn decode_record(&self, record_data: &[u8]) -> Result<Vec<FieldValue>> {
        let mut offset = 0;
        let mut values = Vec::with_capacity(self.fields.len());
        let mut is_first_date = true;

        for field in &self.fields {
            let is_date = field.field_type == FieldType::Date;
            let result = record::decode_field(
                record_data,
                offset,
                field.field_type,
                is_date && is_first_date,
                self.has_date_flag,
            );

            match result {
                Ok((val, consumed)) => {
                    offset += consumed;
                    values.push(val);
                }
                Err(_) if matches!(field.field_type, FieldType::VString | FieldType::VWString) => {
                    // Adaptive extra Int64 recovery: some files have an
                    // undocumented Int64 field not in the XML metadata.
                    // Skip it and retry the string field.
                    if let Some(skip) = try_skip_extra_int64(record_data, offset) {
                        offset += skip;
                        let (val, consumed) = record::decode_field(
                            record_data,
                            offset,
                            field.field_type,
                            false,
                            self.has_date_flag,
                        )
                        .map_err(|e| {
                            YxdbError::ConversionError(format!(
                                "E2 decode error in field '{}' (offset {offset}) \
                                 after skipping extra Int64: {e}",
                                field.name
                            ))
                        })?;
                        offset += consumed;
                        values.push(val);
                    } else {
                        return Err(YxdbError::ConversionError(format!(
                            "E2 decode error in field '{}' (offset {offset}): \
                             invalid prefix and no Int64 recovery possible",
                            field.name
                        )));
                    }
                }
                Err(e) => {
                    return Err(YxdbError::ConversionError(format!(
                        "E2 decode error in field '{}' (offset {offset}): {e}",
                        field.name
                    )));
                }
            }

            if is_date {
                is_first_date = false;
            }
        }

        // Resolve BlobRef values against stored blob_data.
        // BlobRef(offset, len) references a slice of the decompressed type 0x01
        // block data. For V_String/V_WString fields the slice is UTF-8 text;
        // for Blob/SpatialObj it's raw bytes.
        if let Some(blob) = &self.blob_data {
            for (i, val) in values.iter_mut().enumerate() {
                if let FieldValue::BlobRef(off, len) = val {
                    let off = *off;
                    let len = *len;
                    let ft = self.fields[i].field_type;
                    if off + len <= blob.len() {
                        let slice = &blob[off..off + len];
                        *val = match ft {
                            FieldType::Blob | FieldType::SpatialObj => {
                                FieldValue::Blob(Some(slice.to_vec()))
                            }
                            _ => {
                                let s = String::from_utf8_lossy(slice).into_owned();
                                FieldValue::String(Some(s))
                            }
                        };
                    } else {
                        // Reference out of bounds — return null
                        *val = match ft {
                            FieldType::Blob | FieldType::SpatialObj => FieldValue::Blob(None),
                            _ => FieldValue::String(None),
                        };
                    }
                }
            }
        } else {
            // No blob_data available — convert any BlobRef to null
            for (i, val) in values.iter_mut().enumerate() {
                if matches!(val, FieldValue::BlobRef(_, _)) {
                    let ft = self.fields[i].field_type;
                    *val = match ft {
                        FieldType::Blob | FieldType::SpatialObj => FieldValue::Blob(None),
                        _ => FieldValue::String(None),
                    };
                }
            }
        }

        Ok(values)
    }
}

/// Internal block types.
enum Block {
    Record(Vec<u8>),
    Blob(Vec<u8>),
}

/// Try to skip an extra Int64 value at the given offset.
///
/// Some files contain undocumented Int64 fields not declared in the XML
/// metadata (see spec finding #10). Returns the number of bytes consumed
/// if the prefix is a valid compact Int64 encoding (base 6, null 0x4A).
fn try_skip_extra_int64(data: &[u8], offset: usize) -> Option<usize> {
    if offset >= data.len() {
        return None;
    }
    let prefix = data[offset];
    // Int64 compact: base=6, null=0x4A
    // 0x00-0x05: below-base null (1 byte)
    // 0x06: zero value (1 byte)
    // 0x07-0x0E: 1-8 data bytes
    // 0x4A: type-specific null (1 byte)
    if prefix == 0x4A || prefix <= 0x06 {
        return Some(1);
    }
    if (0x07..=0x0E).contains(&prefix) {
        let n_bytes = (prefix - 0x06) as usize;
        let end = offset + 1 + n_bytes;
        if end <= data.len() {
            return Some(1 + n_bytes);
        }
    }
    None
}

/// Return a null FieldValue appropriate for the given field type.
fn null_field_value(ft: FieldType) -> FieldValue {
    match ft {
        FieldType::Bool => FieldValue::Bool(None),
        FieldType::Byte => FieldValue::Byte(None),
        FieldType::Int16 => FieldValue::Int16(None),
        FieldType::Int32 => FieldValue::Int32(None),
        FieldType::Int64 => FieldValue::Int64(None),
        FieldType::Float => FieldValue::Float(None),
        FieldType::Double => FieldValue::Double(None),
        FieldType::Date => FieldValue::Date(None),
        FieldType::DateTime => FieldValue::DateTime(None),
        FieldType::Time => FieldValue::Time(None),
        _ => FieldValue::String(None),
    }
}

/// Convert a column of FieldValues to a Polars Series.
fn field_values_to_series(
    name: &str,
    field_type: FieldType,
    values: Vec<FieldValue>,
) -> Result<Series> {
    match field_type {
        FieldType::Bool => {
            let ca: BooleanChunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Bool(b) => b,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::Byte => {
            let ca: UInt8Chunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Byte(b) => b,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::Int16 => {
            let ca: Int16Chunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Int16(i) => i,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::Int32 => {
            let ca: Int32Chunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Int32(i) => i,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::Int64 => {
            let ca: Int64Chunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Int64(i) => i,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::Float => {
            let ca: Float32Chunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Float(f) => f,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::Double => {
            let ca: Float64Chunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Double(f) => f,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::VString | FieldType::VWString | FieldType::String | FieldType::WString => {
            let ca: StringChunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::String(s) => s,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::Date => {
            let ca: StringChunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Date(s) => s,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::DateTime => {
            let ca: StringChunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::DateTime(s) => s,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::Time => {
            let ca: StringChunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Time(s) => s,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::FixedDecimal => {
            let ca: StringChunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Decimal(s) => s,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
        FieldType::Blob | FieldType::SpatialObj => {
            let ca: BinaryChunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Blob(b) => b,
                    _ => None,
                })
                .collect_ca(PlSmallStr::from(name));
            Ok(ca.into_series())
        }
    }
}
