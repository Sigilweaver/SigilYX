//! E2 YXDB reader - block decompression and record framing.
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

/// Maximum byte length we accept for a single block's declared size.
///
/// 512 MiB is far beyond any observed E2 block (largest seen in the wild is
/// a 1.3 MB blob). `block_size` is an untrusted u32 read straight off the
/// wire, so without this cap a corrupt/malicious file can force a
/// multi-gigabyte allocation before the block is actually read.
const MAX_BLOCK_SIZE: usize = 512 * 1024 * 1024;

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
    /// Blob data from type 0x01 blocks, keyed by file offset.
    /// For 0x11 references (Day12 style): single entry at key 0 with concatenated data.
    /// For 0x12/0x13 references: one entry per blob, keyed by the file offset
    /// at which the type 0x01 block starts.
    blob_blocks: std::collections::HashMap<usize, Vec<u8>>,
    /// Whether to allow reading unverified E2 field types.
    allow_unverified: bool,
    /// Current file position (tracked for blob block offset keying).
    file_pos: u64,
    /// Decompressed bytes of the record block currently being consumed.
    current_block: Vec<u8>,
    /// `(offset, length)` of each record within `current_block`.
    current_spans: Vec<(usize, usize)>,
    /// Index of the next unconsumed span in `current_spans`.
    current_span_idx: usize,
    /// Whether the date-flag probe has run. It uses the first record of the
    /// first record block and applies to every record in the file.
    date_flag_detected: bool,
    /// Set once the block stream has reached its end sentinel or EOF.
    exhausted: bool,
    /// Whether the verified-field-type gate has already been applied.
    types_checked: bool,
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

        let file_pos = (HEADER_SIZE + meta_size) as u64;

        Ok(Self {
            stream,
            header,
            fields,
            meta_xml,
            has_date_flag: false,
            blob_blocks: std::collections::HashMap::new(),
            allow_unverified: false,
            file_pos,
            current_block: Vec::new(),
            current_spans: Vec::new(),
            current_span_idx: 0,
            date_flag_detected: false,
            exhausted: false,
            types_checked: false,
        })
    }

    /// Set whether to allow reading unverified E2 field types.
    ///
    /// By default, E2 files containing field types that have never been
    /// verified against real corpus data (Time, WString, Blob, SpatialObj)
    /// will produce an error. Call this with `true` to attempt reading
    /// them anyway - the decoders are speculative and may produce incorrect
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

    /// Default number of records decoded per batch by [`Self::into_dataframe`].
    pub const DEFAULT_BATCH_SIZE: usize = 65_536;

    /// Read all records and return a Polars DataFrame.
    ///
    /// Records are decoded in batches and stacked, so the intermediate
    /// [`FieldValue`] form of only one batch is resident at a time.
    pub fn into_dataframe(self) -> Result<DataFrame> {
        self.into_dataframe_projected(None)
    }

    /// Read all records, materialising only the named columns.
    ///
    /// See [`Self::next_batch`] for how `columns` is interpreted. `None`
    /// returns all fields in file order.
    pub fn into_dataframe_projected(mut self, columns: Option<&[&str]>) -> Result<DataFrame> {
        let mut out: Option<DataFrame> = None;
        while let Some(batch) = self.next_batch(Self::DEFAULT_BATCH_SIZE, columns)? {
            match out.as_mut() {
                Some(df) => {
                    df.vstack_mut_owned(batch)
                        .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
                }
                None => out = Some(batch),
            }
        }

        match out {
            Some(df) => Ok(df),
            None => self.empty_dataframe(columns),
        }
    }

    /// Count the records in the file without decoding them.
    ///
    /// The E2 header carries no record count, so this walks the block stream
    /// and sums each record block's declared count. Record blocks are
    /// decompressed but their records are never framed or decoded.
    ///
    /// Consumes the reader, since it reads the block stream to the end.
    pub fn count_records(mut self) -> Result<u64> {
        let mut total = 0u64;
        while let Some(block) = self.read_block()? {
            if let Block::Record(decompressed) = block {
                if decompressed.len() < 8 {
                    return Err(YxdbError::InvalidFile(
                        "E2 decompressed block too small for record count".into(),
                    ));
                }
                total += u32::from_le_bytes(decompressed[4..8].try_into().unwrap()) as u64;
            }
        }
        Ok(total)
    }

    /// Decode up to `batch_size` records into a Polars DataFrame.
    ///
    /// Returns `None` once every record in the file has been consumed. Calls
    /// resume where the previous one stopped, including part-way through a
    /// block, so a sequence of calls yields every record exactly once.
    ///
    /// `columns` names the fields to materialise. Every field of every record
    /// is still decoded, because E2 records are row-interleaved and each
    /// field's width depends on the preceding ones, but only the named columns
    /// are collected into the result. `None` returns all fields in file order.
    pub fn next_batch(
        &mut self,
        batch_size: usize,
        columns: Option<&[&str]>,
    ) -> Result<Option<DataFrame>> {
        if !self.allow_unverified && !self.types_checked {
            self.check_verified_types()?;
        }
        self.types_checked = true;

        let projection = self.resolve_projection(columns)?;
        let mut builders: Vec<Vec<FieldValue>> = projection
            .iter()
            .map(|_| Vec::with_capacity(batch_size.min(Self::DEFAULT_BATCH_SIZE)))
            .collect();

        let mut produced = 0usize;
        while produced < batch_size {
            if self.current_span_idx >= self.current_spans.len() {
                if !self.advance_to_record_block()? {
                    break;
                }
                continue;
            }

            let (offset, length) = self.current_spans[self.current_span_idx];
            let decoded = self.decode_record(&self.current_block[offset..offset + length]);
            self.current_span_idx += 1;

            match decoded {
                Ok(mut row) => {
                    // Projection indices are unique, so each value is taken once.
                    for (slot, &field_idx) in builders.iter_mut().zip(projection.iter()) {
                        slot.push(std::mem::replace(
                            &mut row[field_idx],
                            FieldValue::Bool(None),
                        ));
                    }
                }
                Err(_) => {
                    // Records that fail to decode become all-null rows so the
                    // row count still matches the file.
                    for (slot, &field_idx) in builders.iter_mut().zip(projection.iter()) {
                        slot.push(null_field_value(self.fields[field_idx].field_type));
                    }
                }
            }
            produced += 1;
        }

        if produced == 0 {
            return Ok(None);
        }
        self.build_dataframe(&projection, builders, produced)
            .map(Some)
    }

    /// Resolve `columns` to indices into [`Self::fields`], preserving the
    /// requested order. `None` selects every field in file order.
    fn resolve_projection(&self, columns: Option<&[&str]>) -> Result<Vec<usize>> {
        let Some(names) = columns else {
            return Ok((0..self.fields.len()).collect());
        };

        let by_name: std::collections::HashMap<&str, usize> = self
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| (f.name.as_str(), i))
            .collect();

        let unknown: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| !by_name.contains_key(n))
            .collect();
        if !unknown.is_empty() {
            return Err(YxdbError::InvalidFile(format!(
                "requested columns not found in file: {unknown:?}"
            )));
        }

        Ok(names
            .iter()
            .filter_map(|n| by_name.get(n).copied())
            .collect())
    }

    /// Read blocks until one holds at least one record, making it current.
    ///
    /// Blob blocks encountered on the way are cached for reference resolution.
    /// Returns `false` when the block stream ends first.
    fn advance_to_record_block(&mut self) -> Result<bool> {
        loop {
            if self.exhausted {
                return Ok(false);
            }
            match self.read_block()? {
                None => {
                    self.exhausted = true;
                    return Ok(false);
                }
                Some(Block::Blob(offset, data)) => {
                    self.blob_blocks.insert(offset, data);
                }
                Some(Block::Record(decompressed)) => {
                    let spans = self.frame_record_spans(&decompressed)?;
                    if !self.date_flag_detected {
                        if let Some(&(offset, length)) = spans.first() {
                            self.detect_date_flag(&decompressed[offset..offset + length]);
                            self.date_flag_detected = true;
                        }
                    }
                    self.current_block = decompressed;
                    self.current_spans = spans;
                    self.current_span_idx = 0;
                    if !self.current_spans.is_empty() {
                        return Ok(true);
                    }
                }
            }
        }
    }

    /// Turn decoded column values into a DataFrame of the projected fields.
    fn build_dataframe(
        &self,
        projection: &[usize],
        builders: Vec<Vec<FieldValue>>,
        height: usize,
    ) -> Result<DataFrame> {
        let cols: Vec<Column> = projection
            .iter()
            .zip(builders)
            .map(|(&field_idx, values)| {
                let field = &self.fields[field_idx];
                field_values_to_series(&field.name, field.field_type, values)
                    .map(|s| s.into_column())
            })
            .collect::<Result<Vec<_>>>()?;

        if cols.is_empty() {
            return Ok(DataFrame::empty());
        }
        DataFrame::new(height, cols)
            .map_err(|e| YxdbError::ConversionError(format!("failed to build DataFrame: {e}")))
    }

    /// Build a zero-row DataFrame carrying the projected columns' schema.
    fn empty_dataframe(&self, columns: Option<&[&str]>) -> Result<DataFrame> {
        let projection = self.resolve_projection(columns)?;
        let builders = projection.iter().map(|_| Vec::new()).collect();
        self.build_dataframe(&projection, builders, 0)
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

        // The file offset of this block's type byte
        let block_start = self.file_pos as usize;
        self.file_pos += 1;

        match type_byte[0] {
            0x00 => Ok(None),
            0x01 => {
                // Blob block structure:
                //   [type=0x01] [block_size:u32] [uncompressed_size:u32] [hash:16] [0x0A] [snappy data]
                // block_size counts only the 0x0A marker + snappy data.
                // Total on-disk = 1(type) + 4(block_size) + 4(uncomp_size) + 16(hash) + block_size
                let mut size_buf = [0u8; 4];
                self.stream.read_exact(&mut size_buf)?;
                let block_size = u32::from_le_bytes(size_buf) as usize;
                self.file_pos += 4;
                if block_size > MAX_BLOCK_SIZE {
                    return Err(YxdbError::InvalidFile(format!(
                        "E2 type 0x01 block size {block_size} exceeds limit of {MAX_BLOCK_SIZE} (corrupt file?)",
                    )));
                }

                // Read uncompressed_size (4 bytes) + hash (16 bytes) = 20 bytes
                let mut header_buf = [0u8; 20];
                self.stream.read_exact(&mut header_buf)?;
                self.file_pos += 20;

                // Read the Snappy data (block_size bytes: 0x0A marker + actual Snappy)
                let mut snappy_with_marker = vec![0u8; block_size];
                self.stream.read_exact(&mut snappy_with_marker)?;
                self.file_pos += block_size as u64;

                if snappy_with_marker.is_empty() || snappy_with_marker[0] != 0x0A {
                    return Err(YxdbError::InvalidFile(
                        "E2 type 0x01 block missing 0x0A marker".into(),
                    ));
                }

                let snappy_data = &snappy_with_marker[1..];
                let decompressed = snap::raw::Decoder::new()
                    .decompress_vec(snappy_data)
                    .map_err(|e| {
                        YxdbError::InvalidFile(format!(
                            "E2 Snappy decompression failed (blob block): {e}"
                        ))
                    })?;

                Ok(Some(Block::Blob(block_start, decompressed)))
            }
            0x02 => {
                // Record block
                let mut size_buf = [0u8; 4];
                self.stream.read_exact(&mut size_buf)?;
                let block_size = u32::from_le_bytes(size_buf) as usize;
                self.file_pos += 4;
                if block_size > MAX_BLOCK_SIZE {
                    return Err(YxdbError::InvalidFile(format!(
                        "E2 type 0x02 block size {block_size} exceeds limit of {MAX_BLOCK_SIZE} (corrupt file?)",
                    )));
                }

                let mut block_data = vec![0u8; block_size];
                self.stream.read_exact(&mut block_data)?;
                self.file_pos += block_size as u64;

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
            0x03 | 0x04 => {
                // Spatial index blocks - skip them.
                // Structure: [type] [block_size:u32] [inner_size:u32] [0x0A] [snappy data]
                // The Snappy data extends 4 bytes past the declared block_size,
                // but inner_size (4 bytes) is already outside the block_size count.
                let mut size_buf = [0u8; 4];
                self.stream.read_exact(&mut size_buf)?;
                let block_size = u32::from_le_bytes(size_buf) as usize;
                self.file_pos += 4;
                if block_size > MAX_BLOCK_SIZE {
                    return Err(YxdbError::InvalidFile(format!(
                        "E2 spatial index block size {block_size} exceeds limit of {MAX_BLOCK_SIZE} (corrupt file?)",
                    )));
                }

                // Read inner_size (4 bytes, not counted in block_size)
                let mut inner_buf = [0u8; 4];
                self.stream.read_exact(&mut inner_buf)?;
                self.file_pos += 4;

                // Skip the block data (block_size bytes)
                let mut skip_buf = vec![0u8; block_size];
                self.stream.read_exact(&mut skip_buf)?;
                self.file_pos += block_size as u64;

                // Continue reading the next block
                self.read_block()
            }
            other => Err(YxdbError::InvalidFile(format!(
                "unknown E2 block type: 0x{other:02X}"
            ))),
        }
    }

    /// Frame records from decompressed block data.
    ///
    /// Returns the `(offset, length)` of each record within `decompressed`.
    fn frame_record_spans(&self, decompressed: &[u8]) -> Result<Vec<(usize, usize)>> {
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
        records.push((pos, first_record_size));
        pos = end;

        // Subsequent records: [u32 LE size] [record data]
        for i in 1..record_count {
            if pos + 4 > decompressed.len() {
                return Err(YxdbError::InvalidFile(format!(
                    "E2 record {i}: not enough bytes for size prefix"
                )));
            }
            let rec_size = (u32::from_le_bytes(decompressed[pos..pos + 4].try_into().unwrap())
                & 0x7FFF_FFFF) as usize;
            pos += 4;

            let end = pos + rec_size;
            if end > decompressed.len() {
                return Err(YxdbError::InvalidFile(format!(
                    "E2 record {i}: extends past block end ({end} > {})",
                    decompressed.len()
                )));
            }
            records.push((pos, rec_size));
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

        // Resolve BlobRef values against stored blob blocks.
        //
        // BlobRef(offset, len) where len != usize::MAX:
        //   Old-style "0x11" reference: offset+length into a single blob_data block (Day12 style).
        //   The corresponding blob block is stored at key 0 or at whatever file offset was used.
        //
        // BlobRef(file_offset, usize::MAX):
        //   New-style "0x12"/"0x13" reference: lookup by exact file offset key.
        //   The entire decompressed block is the blob value.
        if !self.blob_blocks.is_empty() {
            for (i, val) in values.iter_mut().enumerate() {
                if let FieldValue::BlobRef(off, len) = val {
                    let off = *off;
                    let len = *len;
                    let ft = self.fields[i].field_type;

                    let resolved = if len == usize::MAX {
                        // File-offset reference (0x12/0x13): look up by file offset
                        self.blob_blocks.get(&off).map(|data| data.as_slice())
                    } else {
                        // Offset+length reference (0x11): find any blob block and
                        // slice into it. In the Day12 style there's typically one
                        // blob block containing all concatenated data.
                        self.blob_blocks.values().next().and_then(|blob| {
                            if off + len <= blob.len() {
                                Some(&blob[off..off + len])
                            } else {
                                None
                            }
                        })
                    };

                    *val = match resolved {
                        Some(slice) => match ft {
                            FieldType::Blob | FieldType::SpatialObj => {
                                FieldValue::Blob(Some(slice.to_vec()))
                            }
                            _ => {
                                let s = String::from_utf8_lossy(slice).into_owned();
                                FieldValue::String(Some(s))
                            }
                        },
                        None => match ft {
                            FieldType::Blob | FieldType::SpatialObj => FieldValue::Blob(None),
                            _ => FieldValue::String(None),
                        },
                    };
                }
            }
        } else {
            // No blob blocks available - convert any BlobRef to null
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
    /// Blob block: (file_offset_of_block_start, decompressed_data)
    Blob(usize, Vec<u8>),
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
            // Map Byte (u8) to Int16 to match E1 behaviour and avoid
            // pyo3-polars UInt8 conversion issues.
            let ca: Int16Chunked = values
                .into_iter()
                .map(|v| match v {
                    FieldValue::Byte(b) => b.map(|v| v as i16),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal valid E2 file (header + one-field metadata), then
    /// append a type-0x02 block whose declared `block_size` is enormous.
    fn write_e2_with_oversized_block(block_size: u32) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();

        let xml = r#"<RecordInfo><Field name="x" type="Int32" size="4" /></RecordInfo>"#;

        let mut buf = vec![0u8; HEADER_SIZE];
        buf[0..header::MAGIC.len()].copy_from_slice(header::MAGIC);
        for b in &mut buf[header::MAGIC.len()..64] {
            *b = b' ';
        }
        buf[64..68].copy_from_slice(&header::FILE_ID.to_le_bytes());
        buf[68..72].copy_from_slice(&0x40000001u32.to_le_bytes());
        buf[96..100].copy_from_slice(&(xml.len() as u32).to_le_bytes());

        file.write_all(&buf).unwrap();
        file.write_all(xml.as_bytes()).unwrap();

        // Type 0x02 block with a corrupt, oversized size prefix.
        file.write_all(&[0x02]).unwrap();
        file.write_all(&block_size.to_le_bytes()).unwrap();
        file.flush().unwrap();

        file
    }

    #[test]
    fn oversized_block_size_rejected_without_allocating() {
        // Same class of value the fuzzer found: a corrupt block_size field
        // that would otherwise force a multi-gigabyte allocation before the
        // (nonexistent) block data is even read.
        let file = write_e2_with_oversized_block(u32::MAX);
        let reader = E2Reader::open(file.path()).unwrap();
        let err = reader.into_dataframe().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("exceeds limit"), "unexpected error: {msg}");
    }

    // -- Synthetic E2 files --

    /// Encode an `Int32` field value in the compact integer encoding
    /// (`base + value_byte_count`, then that many little-endian value bytes).
    fn enc_i32(v: i32) -> Vec<u8> {
        if v == 0 {
            return vec![6];
        }
        let le = v.to_le_bytes();
        let n = if v < 0 {
            4
        } else {
            4 - le.iter().rev().take_while(|b| **b == 0).count()
        };
        let mut out = vec![6 + n as u8];
        out.extend_from_slice(&le[..n]);
        out
    }

    /// Encode a short `V_String` field value (`0x80 | len`, then the bytes).
    fn enc_str(s: &str) -> Vec<u8> {
        assert!(s.len() < 128, "test helper only encodes short strings");
        let mut out = vec![0x80 | s.len() as u8];
        out.extend_from_slice(s.as_bytes());
        out
    }

    /// Frame and Snappy-compress one record block.
    fn record_block(records: &[Vec<u8>]) -> Vec<u8> {
        let mut inner = vec![0u8; 8];
        inner[4..8].copy_from_slice(&(records.len() as u32).to_le_bytes());
        // Every record carries a u32 size prefix. The first record's prefix
        // sits in the block header slot at bytes 8..12; the rest sit
        // immediately before their record.
        for rec in records {
            inner.extend_from_slice(&(rec.len() as u32).to_le_bytes());
            inner.extend_from_slice(rec);
        }
        let inner_size = inner.len() as u32;
        inner[0..4].copy_from_slice(&inner_size.to_le_bytes());

        let compressed = snap::raw::Encoder::new().compress_vec(&inner).unwrap();
        let mut block = vec![0x02];
        block.extend_from_slice(&((compressed.len() + 1) as u32).to_le_bytes());
        block.push(0x0A);
        block.extend_from_slice(&compressed);
        block
    }

    /// Write a complete E2 file: header, metadata, one block per element of
    /// `blocks`, then the end-of-stream sentinel.
    fn write_e2_file(xml: &str, blocks: &[Vec<Vec<u8>>]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();

        let mut buf = vec![0u8; HEADER_SIZE];
        buf[0..header::MAGIC.len()].copy_from_slice(header::MAGIC);
        for b in &mut buf[header::MAGIC.len()..64] {
            *b = b' ';
        }
        buf[64..68].copy_from_slice(&header::FILE_ID.to_le_bytes());
        buf[68..72].copy_from_slice(&0x40000001u32.to_le_bytes());
        buf[96..100].copy_from_slice(&(xml.len() as u32).to_le_bytes());

        file.write_all(&buf).unwrap();
        file.write_all(xml.as_bytes()).unwrap();
        for records in blocks {
            file.write_all(&record_block(records)).unwrap();
        }
        file.write_all(&[0x00]).unwrap();
        file.flush().unwrap();
        file
    }

    const ONE_INT_XML: &str =
        r#"<RecordInfo><Field name="x" type="Int32" size="4" /></RecordInfo>"#;

    const TWO_COL_XML: &str = r#"<RecordInfo><Field name="x" type="Int32" size="4" /><Field name="s" type="V_String" size="64" /></RecordInfo>"#;

    /// A file of `n` records, split into blocks of `per_block`, holding
    /// `x = 0..n`.
    fn int_file(n: i32, per_block: usize) -> tempfile::NamedTempFile {
        let blocks: Vec<Vec<Vec<u8>>> = (0..n)
            .map(enc_i32)
            .collect::<Vec<_>>()
            .chunks(per_block)
            .map(<[Vec<u8>]>::to_vec)
            .collect();
        write_e2_file(ONE_INT_XML, &blocks)
    }

    fn int_column(df: &DataFrame) -> Vec<i32> {
        df.column("x")
            .unwrap()
            .i32()
            .unwrap()
            .into_no_null_iter()
            .collect()
    }

    #[test]
    fn into_dataframe_reads_every_record_across_blocks() {
        let file = int_file(250, 40);
        let df = E2Reader::open(file.path())
            .unwrap()
            .into_dataframe()
            .unwrap();
        assert_eq!(df.height(), 250);
        assert_eq!(int_column(&df), (0..250).collect::<Vec<_>>());
    }

    #[test]
    fn batches_cover_every_record_in_order() {
        // batch_size deliberately does not divide the block size, so batches
        // start and end part-way through blocks.
        let file = int_file(250, 40);
        let mut reader = E2Reader::open(file.path()).unwrap();

        let mut seen = Vec::new();
        let mut sizes = Vec::new();
        while let Some(batch) = reader.next_batch(30, None).unwrap() {
            sizes.push(batch.height());
            seen.extend(int_column(&batch));
        }

        assert_eq!(seen, (0..250).collect::<Vec<_>>());
        assert!(
            sizes.iter().take(sizes.len() - 1).all(|&n| n == 30),
            "every batch but the last should be full: {sizes:?}"
        );
        assert_eq!(sizes.iter().sum::<usize>(), 250);
    }

    #[test]
    fn batch_size_larger_than_file_yields_one_batch() {
        let file = int_file(12, 5);
        let mut reader = E2Reader::open(file.path()).unwrap();
        let batch = reader.next_batch(1024, None).unwrap().unwrap();
        assert_eq!(batch.height(), 12);
        assert!(reader.next_batch(1024, None).unwrap().is_none());
    }

    #[test]
    fn batched_read_matches_eager_read() {
        let file = int_file(137, 16);
        let eager = E2Reader::open(file.path())
            .unwrap()
            .into_dataframe()
            .unwrap();

        let mut reader = E2Reader::open(file.path()).unwrap();
        let mut batched: Option<DataFrame> = None;
        while let Some(batch) = reader.next_batch(7, None).unwrap() {
            match batched.as_mut() {
                Some(df) => {
                    df.vstack_mut_owned(batch).unwrap();
                }
                None => batched = Some(batch),
            }
        }
        let batched = batched.unwrap();

        assert_eq!(batched.height(), eager.height());
        assert_eq!(int_column(&batched), int_column(&eager));
    }

    #[test]
    fn projection_returns_requested_columns_in_order() {
        let blocks = vec![vec![
            [enc_i32(1), enc_str("a")].concat(),
            [enc_i32(2), enc_str("bb")].concat(),
        ]];
        let file = write_e2_file(TWO_COL_XML, &blocks);

        let df = E2Reader::open(file.path())
            .unwrap()
            .into_dataframe_projected(Some(&["s", "x"]))
            .unwrap();

        assert_eq!(df.get_column_names(), vec!["s", "x"]);
        assert_eq!(df.height(), 2);
        assert_eq!(int_column(&df), vec![1, 2]);
    }

    #[test]
    fn projection_rejects_unknown_column() {
        let file = int_file(4, 4);
        let err = E2Reader::open(file.path())
            .unwrap()
            .into_dataframe_projected(Some(&["nope"]))
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not found in file"), "unexpected error: {msg}");
    }

    #[test]
    fn count_records_matches_decoded_height() {
        let file = int_file(250, 40);
        let counted = E2Reader::open(file.path())
            .unwrap()
            .count_records()
            .unwrap();
        let df = E2Reader::open(file.path())
            .unwrap()
            .into_dataframe()
            .unwrap();
        assert_eq!(counted, 250);
        assert_eq!(counted as usize, df.height());
    }

    #[test]
    fn empty_file_yields_zero_rows_with_schema() {
        let file = write_e2_file(TWO_COL_XML, &[]);
        let df = E2Reader::open(file.path())
            .unwrap()
            .into_dataframe()
            .unwrap();
        assert_eq!(df.height(), 0);
        assert_eq!(df.get_column_names(), vec!["x", "s"]);

        let mut reader = E2Reader::open(file.path()).unwrap();
        assert!(reader.next_batch(64, None).unwrap().is_none());
    }
}
