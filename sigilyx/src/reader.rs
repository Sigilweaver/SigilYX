use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use polars::prelude::*;

use crate::error::{YxdbError, Result};
use crate::field::{FieldMeta, FieldType};
use crate::header::{self, YxdbHeader, HEADER_SIZE};
use crate::lzf;
use crate::record;

/// A streaming YXDB file reader.
///
/// Opens and validates the file, then provides methods to iterate records
/// or materialize the entire file as a Polars [`DataFrame`].
pub struct YxdbReader {
    stream: BufReader<File>,
    pub header: YxdbHeader,
    pub fields: Vec<FieldMeta>,
    pub meta_xml: String,
    fixed_size: usize,
    has_var: bool,
    // LZF block state
    lzf_out: Vec<u8>,
    lzf_out_idx: usize,
    lzf_out_size: usize,
    lzf_in: Vec<u8>,         // reusable compressed-input buffer
    current_record: u64,
}

impl YxdbReader {
    /// Open a YXDB file for reading.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        let mut stream = BufReader::new(file);

        // Read 512-byte header
        let mut header_buf = [0u8; HEADER_SIZE];
        stream.read_exact(&mut header_buf)?;
        let header = YxdbHeader::parse(&header_buf)?;

        // Read UTF-16LE metadata
        let meta_byte_len = header.meta_info_size as usize * 2;
        let mut meta_bytes = vec![0u8; meta_byte_len];
        stream.read_exact(&mut meta_bytes)?;

        // Strip trailing null terminator (2 bytes)
        let xml_bytes = if meta_byte_len >= 2 {
            &meta_bytes[..meta_byte_len - 2]
        } else {
            &meta_bytes
        };
        let meta_xml = header::decode_utf16_le(xml_bytes);

        // Parse fields
        let fields = header::parse_meta_xml(&meta_xml)?;

        // Compute fixed record size
        let fixed_size: usize = fields
            .last()
            .map(|f| f.offset + f.field_type.fixed_bytes(f.size))
            .unwrap_or(0);
        let has_var = fields.iter().any(|f| f.field_type.is_variable());

        Ok(YxdbReader {
            stream,
            header,
            fields,
            meta_xml,
            fixed_size,
            has_var,
            lzf_out: vec![0u8; 262144],
            lzf_out_idx: 0,
            lzf_out_size: 0,
            lzf_in: Vec::with_capacity(262144),
            current_record: 0,
        })
    }

    /// Read the next record into the provided buffer.
    ///
    /// Returns `true` if a record was read, `false` if all records have been
    /// consumed. The buffer is resized as needed.
    pub fn next_record(&mut self, buf: &mut Vec<u8>) -> Result<bool> {
        self.current_record += 1;
        if self.current_record > self.header.num_records {
            return Ok(false);
        }

        if self.has_var {
            // Read fixed portion + 4-byte var-length header
            let needed = self.fixed_size + 4;
            buf.resize(needed, 0);
            self.read_bytes(&mut buf[..needed])?;

            // Read the variable portion length from the last 4 bytes
            let var_len = u32::from_le_bytes(
                buf[self.fixed_size..self.fixed_size + 4]
                    .try_into()
                    .unwrap(),
            ) as usize;

            // Extend buffer for variable data
            let total = needed + var_len;
            buf.resize(total, 0);
            self.read_bytes(&mut buf[needed..total])?;
        } else {
            buf.resize(self.fixed_size, 0);
            self.read_bytes(&mut buf[..self.fixed_size])?;
        }

        Ok(true)
    }

    /// Consume the reader and produce a Polars [`DataFrame`].
    ///
    /// This reads all records and builds columnar arrays directly from the
    /// record buffer — no intermediate allocations per field.
    pub fn into_dataframe(mut self) -> Result<DataFrame> {
        let num_records = self.header.num_records as usize;
        let fields = self.fields.clone();

        // Pre-allocate column builders
        let mut builders: Vec<ColumnBuilder> = fields
            .iter()
            .map(|f| ColumnBuilder::new(f, num_records))
            .collect();

        // Read all records — push directly from buffer into builders
        let mut record_buf = Vec::with_capacity(self.fixed_size + 1024);
        while self.next_record(&mut record_buf)? {
            for (i, field) in fields.iter().enumerate() {
                builders[i].push_from_record(&record_buf, field)?;
            }
        }

        // Build the DataFrame
        let columns: Vec<Column> = builders
            .into_iter()
            .zip(fields.iter())
            .map(|(b, f)| b.into_series(&f.name))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .map(Column::from)
            .collect();

        DataFrame::new(columns).map_err(|e| YxdbError::ConversionError(e.to_string()))
    }

    /// Read the next batch of up to `batch_size` records as a [`DataFrame`].
    ///
    /// Returns `None` when all records have been consumed. This enables
    /// streaming/memory-efficient processing of large YXDB files.
    ///
    /// ```no_run
    /// use sigilyx::YxdbReader;
    ///
    /// let mut reader = YxdbReader::open("large_file.yxdb").unwrap();
    /// while let Some(batch) = reader.next_batch(65_536).unwrap() {
    ///     println!("batch: {} rows", batch.height());
    /// }
    /// ```
    pub fn next_batch(&mut self, batch_size: usize) -> Result<Option<DataFrame>> {
        if self.current_record >= self.header.num_records {
            return Ok(None);
        }

        let remaining = (self.header.num_records - self.current_record) as usize;
        let this_batch = remaining.min(batch_size);

        let fields = self.fields.clone();
        let mut builders: Vec<ColumnBuilder> = fields
            .iter()
            .map(|f| ColumnBuilder::new(f, this_batch))
            .collect();

        let mut record_buf = Vec::with_capacity(self.fixed_size + 1024);
        let mut count = 0;
        while count < this_batch {
            if !self.next_record(&mut record_buf)? {
                break;
            }
            for (i, field) in fields.iter().enumerate() {
                builders[i].push_from_record(&record_buf, field)?;
            }
            count += 1;
        }

        if count == 0 {
            return Ok(None);
        }

        let columns: Vec<Column> = builders
            .into_iter()
            .zip(fields.iter())
            .map(|(b, f)| b.into_series(&f.name))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .map(Column::from)
            .collect();

        let df = DataFrame::new(columns)
            .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
        Ok(Some(df))
    }

    // ── LZF block reading ──────────────────────────────────────────────

    /// Read exactly `size` bytes from the LZF-compressed stream into `dest`.
    fn read_bytes(&mut self, dest: &mut [u8]) -> Result<()> {
        let mut remaining = dest.len();
        let mut dest_idx = 0;

        while remaining > 0 {
            // If we've exhausted the current decompressed block, read a new one
            if self.lzf_out_idx >= self.lzf_out_size {
                self.read_next_lzf_block()?;
            }

            let available = self.lzf_out_size - self.lzf_out_idx;
            let to_copy = remaining.min(available);
            dest[dest_idx..dest_idx + to_copy]
                .copy_from_slice(&self.lzf_out[self.lzf_out_idx..self.lzf_out_idx + to_copy]);
            self.lzf_out_idx += to_copy;
            dest_idx += to_copy;
            remaining -= to_copy;
        }

        Ok(())
    }

    /// Read and decompress the next LZF block from the stream.
    fn read_next_lzf_block(&mut self) -> Result<()> {
        // Read 4-byte block length
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf)?;
        let raw_len = u32::from_le_bytes(len_buf) as usize;

        let is_uncompressed = raw_len & 0x80000000 != 0;
        let block_len = raw_len & 0x7FFFFFFF;

        if is_uncompressed {
            // Store directly
            if self.lzf_out.len() < block_len {
                self.lzf_out.resize(block_len, 0);
            }
            self.stream.read_exact(&mut self.lzf_out[..block_len])?;
            self.lzf_out_size = block_len;
        } else {
            // Reuse the lzf_in buffer to avoid per-block heap allocation
            self.lzf_in.resize(block_len, 0);
            self.stream.read_exact(&mut self.lzf_in[..block_len])?;
            self.lzf_out_size = lzf::decompress(&self.lzf_in[..block_len], &mut self.lzf_out)?;
        }

        self.lzf_out_idx = 0;
        Ok(())
    }
}

// ── Column builders ────────────────────────────────────────────────────

/// Accumulates values for a single column and converts to a Polars [`Series`].
///
/// The `push_from_record` method reads directly from the record buffer,
/// avoiding intermediate `FieldValue` allocations on the hot path.
enum ColumnBuilder {
    Bool(Vec<Option<bool>>),
    Byte(Vec<Option<u8>>),
    Int16(Vec<Option<i16>>),
    Int32(Vec<Option<i32>>),
    Int64(Vec<Option<i64>>),
    Float(Vec<Option<f32>>),
    Double(Vec<Option<f64>>),
    Str(Vec<Option<String>>),
    /// Date stored as days-since-epoch (i32) for direct Polars Date construction.
    DateDays(Vec<Option<i32>>),
    Time(Vec<Option<String>>),
    /// DateTime stored as ms-since-epoch (i64) for direct Polars Datetime construction.
    DateTimeMs(Vec<Option<i64>>),
    Blob(Vec<Option<Vec<u8>>>),
}

/// Hinnant's algorithm raw-day value for 1970-01-01 (Unix epoch).
const UNIX_EPOCH_DAYS: i32 = 719_468;

/// Parse "YYYY-MM-DD" ASCII bytes directly to days-since-Unix-epoch.
#[inline]
fn parse_date_to_days(buf: &[u8]) -> Option<i32> {
    // buf must be at least 10 bytes: "YYYY-MM-DD"
    if buf.len() < 10 { return None; }
    let y = parse_4_digits(buf)? as i32;
    let m = parse_2_digits(&buf[5..])? as u32;
    let d = parse_2_digits(&buf[8..])? as u32;
    Some(civil_to_days(y, m, d))
}

/// Parse "YYYY-MM-DD HH:MM:SS" ASCII bytes directly to ms-since-Unix-epoch.
#[inline]
fn parse_datetime_to_ms(buf: &[u8]) -> Option<i64> {
    if buf.len() < 19 { return None; }
    let days = parse_date_to_days(buf)? as i64;
    let h = parse_2_digits(&buf[11..])? as i64;
    let min = parse_2_digits(&buf[14..])? as i64;
    let s = parse_2_digits(&buf[17..])? as i64;
    Some(days * 86_400_000 + h * 3_600_000 + min * 60_000 + s * 1_000)
}

#[inline]
fn parse_2_digits(b: &[u8]) -> Option<u16> {
    let d0 = b[0].wrapping_sub(b'0');
    let d1 = b[1].wrapping_sub(b'0');
    if d0 > 9 || d1 > 9 { return None; }
    Some(d0 as u16 * 10 + d1 as u16)
}

#[inline]
fn parse_4_digits(b: &[u8]) -> Option<u16> {
    let d0 = b[0].wrapping_sub(b'0') as u16;
    let d1 = b[1].wrapping_sub(b'0') as u16;
    let d2 = b[2].wrapping_sub(b'0') as u16;
    let d3 = b[3].wrapping_sub(b'0') as u16;
    if d0 > 9 || d1 > 9 || d2 > 9 || d3 > 9 { return None; }
    Some(d0 * 1000 + d1 * 100 + d2 * 10 + d3)
}

/// Convert a civil date (year, month 1-12, day 1-31) to days since Unix epoch.
/// Algorithm from Howard Hinnant's date algorithms (public domain).
#[inline]
fn civil_to_days(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe as i32) - UNIX_EPOCH_DAYS
}

impl ColumnBuilder {
    fn new(field: &FieldMeta, capacity: usize) -> Self {
        match field.field_type {
            FieldType::Bool => ColumnBuilder::Bool(Vec::with_capacity(capacity)),
            FieldType::Byte => ColumnBuilder::Byte(Vec::with_capacity(capacity)),
            FieldType::Int16 => ColumnBuilder::Int16(Vec::with_capacity(capacity)),
            FieldType::Int32 => ColumnBuilder::Int32(Vec::with_capacity(capacity)),
            FieldType::Int64 => ColumnBuilder::Int64(Vec::with_capacity(capacity)),
            FieldType::Float => ColumnBuilder::Float(Vec::with_capacity(capacity)),
            FieldType::Double | FieldType::FixedDecimal => {
                ColumnBuilder::Double(Vec::with_capacity(capacity))
            }
            FieldType::String
            | FieldType::WString
            | FieldType::VString
            | FieldType::VWString => ColumnBuilder::Str(Vec::with_capacity(capacity)),
            FieldType::Date => ColumnBuilder::DateDays(Vec::with_capacity(capacity)),
            FieldType::Time => ColumnBuilder::Time(Vec::with_capacity(capacity)),
            FieldType::DateTime => ColumnBuilder::DateTimeMs(Vec::with_capacity(capacity)),
            FieldType::Blob | FieldType::SpatialObj => {
                ColumnBuilder::Blob(Vec::with_capacity(capacity))
            }
        }
    }

    /// Push a value directly from the record buffer into this builder.
    ///
    /// This is the hot-path method — it avoids creating any intermediate
    /// `FieldValue` enum and parses dates/datetimes to native integers.
    #[inline]
    fn push_from_record(&mut self, record: &[u8], field: &FieldMeta) -> Result<()> {
        let off = field.offset;
        match self {
            ColumnBuilder::Bool(v) => {
                let b = record[off];
                v.push(if b == 2 { None } else { Some(b == 1) });
            }
            ColumnBuilder::Byte(v) => {
                if record[off + 1] == 1 { v.push(None); }
                else { v.push(Some(record[off])); }
            }
            ColumnBuilder::Int16(v) => {
                if record[off + 2] == 1 { v.push(None); }
                else {
                    v.push(Some(i16::from_le_bytes(
                        record[off..off + 2].try_into().unwrap(),
                    )));
                }
            }
            ColumnBuilder::Int32(v) => {
                if record[off + 4] == 1 { v.push(None); }
                else {
                    v.push(Some(i32::from_le_bytes(
                        record[off..off + 4].try_into().unwrap(),
                    )));
                }
            }
            ColumnBuilder::Int64(v) => {
                if record[off + 8] == 1 { v.push(None); }
                else {
                    v.push(Some(i64::from_le_bytes(
                        record[off..off + 8].try_into().unwrap(),
                    )));
                }
            }
            ColumnBuilder::Float(v) => {
                if record[off + 4] == 1 { v.push(None); }
                else {
                    v.push(Some(f32::from_le_bytes(
                        record[off..off + 4].try_into().unwrap(),
                    )));
                }
            }
            ColumnBuilder::Double(v) => {
                // Handles both Double and FixedDecimal
                match field.field_type {
                    FieldType::Double => {
                        if record[off + 8] == 1 { v.push(None); }
                        else {
                            v.push(Some(f64::from_le_bytes(
                                record[off..off + 8].try_into().unwrap(),
                            )));
                        }
                    }
                    FieldType::FixedDecimal => {
                        if record[off + field.size] == 1 { v.push(None); }
                        else {
                            // Parse ASCII decimal in-place
                            let s = record::extract_fixed_string(record, off, field.size);
                            v.push(Some(s.parse::<f64>().unwrap_or(0.0)));
                        }
                    }
                    _ => unreachable!(),
                }
            }
            ColumnBuilder::Str(v) => {
                match field.field_type {
                    FieldType::String => {
                        if record[off + field.size] == 1 { v.push(None); }
                        else {
                            v.push(Some(record::extract_fixed_string(record, off, field.size)));
                        }
                    }
                    FieldType::WString => {
                        let null_byte_off = off + field.size * 2;
                        if record[null_byte_off] == 1 { v.push(None); }
                        else {
                            v.push(Some(record::extract_fixed_wstring(record, off, field.size)));
                        }
                    }
                    FieldType::VString => {
                        match record::parse_var_data(record, off) {
                            None => v.push(None),
                            Some(bytes) => v.push(Some(String::from_utf8_lossy(&bytes).into_owned())),
                        }
                    }
                    FieldType::VWString => {
                        match record::parse_var_data(record, off) {
                            None => v.push(None),
                            Some(bytes) if bytes.is_empty() => v.push(Some(String::new())),
                            Some(bytes) => {
                                let code_units: Vec<u16> = bytes
                                    .chunks_exact(2)
                                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                                    .collect();
                                v.push(Some(String::from_utf16_lossy(&code_units)));
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
            ColumnBuilder::DateDays(v) => {
                if record[off + 10] == 1 {
                    v.push(None);
                } else {
                    v.push(parse_date_to_days(&record[off..off + 10]));
                }
            }
            ColumnBuilder::Time(v) => {
                if record[off + 8] == 1 { v.push(None); }
                else {
                    v.push(Some(record::extract_fixed_string(record, off, 8)));
                }
            }
            ColumnBuilder::DateTimeMs(v) => {
                if record[off + 19] == 1 {
                    v.push(None);
                } else {
                    v.push(parse_datetime_to_ms(&record[off..off + 19]));
                }
            }
            ColumnBuilder::Blob(v) => {
                v.push(record::parse_var_data(record, off));
            }
        }
        Ok(())
    }

    fn into_series(self, name: &str) -> Result<Series> {
        let s = match self {
            ColumnBuilder::Bool(v) => Series::new(name.into(), v),
            ColumnBuilder::Byte(v) => {
                // Store Byte as Int16 (UInt8 needs extra Polars features)
                let vals: Vec<Option<i16>> = v.into_iter().map(|o| o.map(|b| b as i16)).collect();
                Series::new(name.into(), vals)
            }
            ColumnBuilder::Int16(v) => Series::new(name.into(), v),
            ColumnBuilder::Int32(v) => Series::new(name.into(), v),
            ColumnBuilder::Int64(v) => Series::new(name.into(), v),
            ColumnBuilder::Float(v) => Series::new(name.into(), v),
            ColumnBuilder::Double(v) => Series::new(name.into(), v),
            ColumnBuilder::Str(v) => Series::new(name.into(), v),
            ColumnBuilder::DateDays(v) => {
                // Build Date series directly from i32 days-since-epoch
                let ca = Int32Chunked::new(name.into(), &v);
                ca.into_date().into_series()
            }
            ColumnBuilder::Time(v) => {
                // Keep as string — Polars Time type needs duration
                Series::new(name.into(), v)
            }
            ColumnBuilder::DateTimeMs(v) => {
                // Build Datetime series directly from i64 ms-since-epoch
                let ca = Int64Chunked::new(name.into(), &v);
                ca.into_datetime(TimeUnit::Milliseconds, None).into_series()
            }
            ColumnBuilder::Blob(v) => {
                // Store as Binary
                let values: Vec<Option<&[u8]>> = v
                    .iter()
                    .map(|opt| opt.as_deref())
                    .collect();
                Series::new(name.into(), values)
            }
        };
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> String {
        format!("{}/test_files/{}", env!("CARGO_MANIFEST_DIR"), name)
    }

    // ── AllTypes.yxdb: 2 rows × 16 columns covering every field type ──

    #[test]
    fn all_types_shape() {
        let df = crate::read_yxdb(&test_path("AllTypes.yxdb")).unwrap();
        assert_eq!(df.height(), 2);
        assert_eq!(df.width(), 16);
    }

    #[test]
    fn all_types_integer_values() {
        let df = crate::read_yxdb(&test_path("AllTypes.yxdb")).unwrap();
        // Row 0 values (known from generation script)
        let byte_col = df.column("ByteCol").unwrap().i16().unwrap();
        assert_eq!(byte_col.get(0), Some(7));
        assert_eq!(byte_col.get(1), Some(255));

        let i16_col = df.column("Int16Col").unwrap().i16().unwrap();
        assert_eq!(i16_col.get(0), Some(-1234));
        assert_eq!(i16_col.get(1), Some(32767));

        let i32_col = df.column("Int32Col").unwrap().i32().unwrap();
        assert_eq!(i32_col.get(0), Some(42000));
        assert_eq!(i32_col.get(1), Some(-1));

        let i64_col = df.column("Int64Col").unwrap().i64().unwrap();
        assert_eq!(i64_col.get(0), Some(9_000_000_000));
        assert_eq!(i64_col.get(1), Some(-9_000_000_000));
    }

    #[test]
    fn all_types_bool_values() {
        let df = crate::read_yxdb(&test_path("AllTypes.yxdb")).unwrap();
        let col = df.column("BoolCol").unwrap().bool().unwrap();
        assert_eq!(col.get(0), Some(true));
        assert_eq!(col.get(1), Some(false));
    }

    #[test]
    fn all_types_float_values() {
        let df = crate::read_yxdb(&test_path("AllTypes.yxdb")).unwrap();
        let f32_col = df.column("FloatCol").unwrap().f32().unwrap();
        assert!((f32_col.get(0).unwrap() - 2.5).abs() < 0.01);

        let f64_col = df.column("DoubleCol").unwrap().f64().unwrap();
        assert!((f64_col.get(0).unwrap() - std::f64::consts::PI).abs() < 1e-10);
        assert!((f64_col.get(1).unwrap() - 0.0).abs() < 1e-10);

        let dec_col = df.column("DecimalCol").unwrap().f64().unwrap();
        assert!((dec_col.get(0).unwrap() - 1234.5678).abs() < 0.001);
    }

    #[test]
    fn all_types_string_values() {
        let df = crate::read_yxdb(&test_path("AllTypes.yxdb")).unwrap();
        let str_col = df.column("StringCol").unwrap().str().unwrap();
        assert_eq!(str_col.get(0), Some("Alteryx"));

        let wstr_col = df.column("WStringCol").unwrap().str().unwrap();
        assert_eq!(wstr_col.get(0), Some("Ünïcödé"));

        let vstr_col = df.column("VStringCol").unwrap().str().unwrap();
        assert_eq!(vstr_col.get(0), Some("short var"));

        // Row 1 long string
        let vwstr_col = df.column("VWStringCol").unwrap().str().unwrap();
        let row0 = vwstr_col.get(0).unwrap();
        assert_eq!(row0.len(), 600);
        assert!(row0.chars().all(|c| c == 'x'));
    }

    #[test]
    fn all_types_date_time_values() {
        let df = crate::read_yxdb(&test_path("AllTypes.yxdb")).unwrap();

        // DateCol → Polars Date (days since epoch)
        let date_col = df.column("DateCol").unwrap().date().unwrap();
        // 2025-03-15 → days since 1970-01-01
        let expected_date = chrono_date_to_days(2025, 3, 15);
        assert_eq!(date_col.get(0), Some(expected_date));

        // DateTimeCol → Polars Datetime (ms since epoch)
        let dt_col = df.column("DateTimeCol").unwrap().datetime().unwrap();
        // 2025-03-15 08:30:00 → ms since 1970-01-01
        let expected_dt = chrono_date_to_days(2025, 3, 15) as i64 * 86_400_000
            + 8 * 3_600_000 + 30 * 60_000;
        assert_eq!(dt_col.get(0), Some(expected_dt));
    }

    /// Helper: compute days since Unix epoch for a given civil date.
    fn chrono_date_to_days(y: i32, m: u32, d: u32) -> i32 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = (y - era * 400) as u32;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        (era * 146097 + doe as i32) - 719_468
    }

    #[test]
    fn all_types_blob_values() {
        let df = crate::read_yxdb(&test_path("AllTypes.yxdb")).unwrap();
        let col = df.column("BlobCol").unwrap().binary().unwrap();

        // Row 0: 1024 bytes (bytes(range(256)) * 4)
        let blob0 = col.get(0).unwrap();
        assert_eq!(blob0.len(), 1024);
        assert_eq!(blob0[0], 0x00);
        assert_eq!(blob0[1], 0x01);
        assert_eq!(blob0[255], 0xFF);
        assert_eq!(blob0[256], 0x00); // repeats

        // Row 1: 512 bytes of 0xFF
        let blob1 = col.get(1).unwrap();
        assert_eq!(blob1.len(), 512);
        assert!(blob1.iter().all(|&b| b == 0xFF));
    }

    // ── NullValues.yxdb: 3 rows with null patterns ──

    #[test]
    fn null_values_populated_row() {
        let df = crate::read_yxdb(&test_path("NullValues.yxdb")).unwrap();
        assert_eq!(df.height(), 3);

        // Row 0 is fully populated
        let id_col = df.column("Id").unwrap().i32().unwrap();
        assert_eq!(id_col.get(0), Some(1));

        let str_col = df.column("NullStr").unwrap().str().unwrap();
        assert_eq!(str_col.get(0), Some("hello"));
    }

    #[test]
    fn null_values_all_null_row() {
        let df = crate::read_yxdb(&test_path("NullValues.yxdb")).unwrap();

        // Row 1: all null except Id
        let id_col = df.column("Id").unwrap().i32().unwrap();
        assert_eq!(id_col.get(1), Some(2));

        // Check nulls via the typed chunked arrays
        assert!(df.column("NullByte").unwrap().i16().unwrap().get(1).is_none());
        assert!(df.column("NullInt16").unwrap().i16().unwrap().get(1).is_none());
        assert!(df.column("NullInt32").unwrap().i32().unwrap().get(1).is_none());
        assert!(df.column("NullInt64").unwrap().i64().unwrap().get(1).is_none());
        assert!(df.column("NullFloat").unwrap().f32().unwrap().get(1).is_none());
        assert!(df.column("NullDouble").unwrap().f64().unwrap().get(1).is_none());
        assert!(df.column("NullStr").unwrap().str().unwrap().get(1).is_none());
        assert!(df.column("NullBlob").unwrap().binary().unwrap().get(1).is_none());
    }

    #[test]
    fn null_values_mixed_row() {
        let df = crate::read_yxdb(&test_path("NullValues.yxdb")).unwrap();

        // Row 2: mixed — NullByte is null, NullInt16 is 50
        assert!(df.column("NullByte").unwrap().i16().unwrap().get(2).is_none());
        let i16_col = df.column("NullInt16").unwrap().i16().unwrap();
        assert_eq!(i16_col.get(2), Some(50));
        assert!(df.column("NullInt32").unwrap().i32().unwrap().get(2).is_none());
    }

    // ── ManyRecords.yxdb: 50,000 rows for LZF block stress test ──

    #[test]
    fn many_records_shape() {
        let df = crate::read_yxdb(&test_path("ManyRecords.yxdb")).unwrap();
        assert_eq!(df.height(), 50_000);
        assert_eq!(df.width(), 3);
    }

    #[test]
    fn many_records_id_sum() {
        let df = crate::read_yxdb(&test_path("ManyRecords.yxdb")).unwrap();
        let id_col = df.column("Id").unwrap().i32().unwrap();
        let id_sum: i64 = id_col.into_iter().map(|v| v.unwrap_or(0) as i64).sum();
        // sum(1..=50000) = 50000 * 50001 / 2 = 1_250_025_000
        assert_eq!(id_sum, 1_250_025_000);
    }

    #[test]
    fn many_records_label_check() {
        let df = crate::read_yxdb(&test_path("ManyRecords.yxdb")).unwrap();
        let label_col = df.column("Label").unwrap().str().unwrap();
        assert_eq!(label_col.get(0), Some("row_00001"));
        assert_eq!(label_col.get(49_999), Some("row_50000"));
    }

    // ── LargeBlob.yxdb: large binary data ──

    #[test]
    fn large_blob_sizes() {
        let df = crate::read_yxdb(&test_path("LargeBlob.yxdb")).unwrap();
        assert_eq!(df.height(), 4);
        let col = df.column("Data").unwrap().binary().unwrap();

        // Row 0: 512,000 bytes
        assert_eq!(col.get(0).unwrap().len(), 512_000);
        // Row 1: null
        assert!(col.get(1).is_none());
        // Row 2: 4 bytes ("tiny")
        assert_eq!(col.get(2).unwrap(), b"tiny");
        // Row 3: 500,000 bytes
        assert_eq!(col.get(3).unwrap().len(), 500_000);
    }

    // ── People.yxdb: 200 rows of realistic mixed data ──

    #[test]
    fn people_shape_and_columns() {
        let df = crate::read_yxdb(&test_path("People.yxdb")).unwrap();
        assert_eq!(df.height(), 200);
        assert_eq!(df.width(), 8);
        assert!(df.get_column_names().iter().any(|n| n.as_str() == "FirstName"));
        assert!(df.get_column_names().iter().any(|n| n.as_str() == "Salary"));
    }

    #[test]
    fn people_no_null_ids() {
        let df = crate::read_yxdb(&test_path("People.yxdb")).unwrap();
        assert_eq!(df.column("PersonId").unwrap().null_count(), 0);
    }

    // ── Strings.yxdb: string edge cases ──

    #[test]
    fn strings_edge_cases() {
        let df = crate::read_yxdb(&test_path("Strings.yxdb")).unwrap();
        assert_eq!(df.height(), 6);

        let vstr = df.column("VarStr").unwrap().str().unwrap();
        // Row 0: normal
        assert_eq!(vstr.get(0), Some("variable"));
        // Row 1: empty
        assert_eq!(vstr.get(1), Some(""));
        // Row 3: long (2000 chars of 'M')
        let long_str = vstr.get(3).unwrap();
        assert_eq!(long_str.len(), 2000);
        assert!(long_str.chars().all(|c| c == 'M'));
        // Row 5: null
        assert!(vstr.get(5).is_none());
    }

    #[test]
    fn strings_unicode() {
        let df = crate::read_yxdb(&test_path("Strings.yxdb")).unwrap();
        let vwstr = df.column("VarWStr").unwrap().str().unwrap();
        // Row 0: "wïdé" (unicode in wide string)
        assert_eq!(vwstr.get(0), Some("wïdé"));
        // Row 4: Japanese characters
        assert_eq!(vwstr.get(4), Some("日本語テスト"));
    }

    // ── SingleColumn.yxdb: simplest valid file ──

    #[test]
    fn single_column_values() {
        let df = crate::read_yxdb(&test_path("SingleColumn.yxdb")).unwrap();
        assert_eq!(df.height(), 5);
        let col = df.column("Value").unwrap().i32().unwrap();
        assert_eq!(col.get(0), Some(10));
        assert_eq!(col.get(4), Some(50));
    }

    // ── Error handling ──

    #[test]
    fn reject_invalid_text_file() {
        let result = YxdbReader::open(&test_path("not_a_yxdb.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn reject_too_small_file() {
        let result = YxdbReader::open(&test_path("too_small.bin"));
        assert!(result.is_err());
    }

    #[test]
    fn reject_nonexistent_file() {
        let result = YxdbReader::open(&test_path("does_not_exist.yxdb"));
        assert!(result.is_err());
    }
}
