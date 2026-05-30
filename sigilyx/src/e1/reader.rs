use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use arrow::bitmap::MutableBitmap;
use memmap2::Mmap;
use polars::prelude::*;
use rayon::prelude::*;

use super::header::{self, YxdbHeader, HEADER_SIZE};
use super::lzf::{self, CompressionAlgorithm};
use super::record;
use super::record::FieldValue;
use crate::error::{Result, YxdbError};
use crate::field::{FieldMeta, FieldType};

/// A streaming YXDB file reader.
///
/// Opens and validates the file, then provides methods to iterate records
/// or materialize the entire file as a Polars [`DataFrame`].
pub struct YxdbReader {
    stream: BufReader<File>,
    pub header: YxdbHeader,
    pub fields: Vec<FieldMeta>,
    pub meta_xml: String,
    pub(crate) fixed_size: usize,
    has_var: bool,
    compression: Option<CompressionAlgorithm>,
    // LZF block state
    lzf_out: Vec<u8>,
    lzf_out_idx: usize,
    lzf_out_size: usize,
    lzf_in: Vec<u8>, // reusable compressed-input buffer
    current_record: u64,
}

impl YxdbReader {
    /// Open a YXDB file for reading.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        let mut stream = BufReader::new(file);

        // Read 512-byte header
        let mut header_buf = [0u8; HEADER_SIZE];
        match stream.read_exact(&mut header_buf) {
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(YxdbError::InvalidFile(
                    "file too small to be a valid YXDB (< 512 bytes)".into(),
                ));
            }
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
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

        // Compute fixed record size and validate field offsets
        let fixed_size: usize = fields
            .last()
            .map(|f| f.offset + f.field_type.fixed_bytes(f.size))
            .unwrap_or(0);

        // Sanity-check: reject schemas where computed fixed_size is unreasonably
        // large.  This guards the `unsafe` code paths that index into record
        // buffers using field offsets parsed from the XML metadata.
        const MAX_REASONABLE_RECORD_SIZE: usize = 64 * 1024 * 1024; // 64 MiB
        if fixed_size > MAX_REASONABLE_RECORD_SIZE {
            return Err(YxdbError::InvalidFile(format!(
                "computed fixed record size ({fixed_size} bytes) exceeds sanity limit — \
                 the XML metadata is likely corrupt"
            )));
        }

        // Validate that every field's offset + size fits within the fixed portion
        for f in &fields {
            let end = f.offset + f.field_type.fixed_bytes(f.size);
            if end > fixed_size {
                return Err(YxdbError::InvalidFile(format!(
                    "field '{}' (offset {} + {} bytes) exceeds computed fixed record size ({})",
                    f.name,
                    f.offset,
                    f.field_type.fixed_bytes(f.size),
                    fixed_size
                )));
            }
        }
        let has_var = fields.iter().any(|f| f.field_type.is_variable());

        let compression = CompressionAlgorithm::from_version_id(header.compression_version)?;

        Ok(YxdbReader {
            stream,
            header,
            fields,
            meta_xml,
            fixed_size,
            has_var,
            compression,
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
    /// Pipelined approach for maximum throughput:
    /// 1. Read all remaining file data into memory (single I/O operation)
    /// 2. Parse LZF block boundaries, then decompress blocks into a contiguous
    ///    buffer (parallel for large files, sequential for small ones)
    /// 3. Compute record boundaries (arithmetic for fixed-size, scan for variable)
    /// 4. Build columns (parallel for wide schemas, sequential for narrow ones)
    pub fn into_dataframe(self) -> Result<DataFrame> {
        self.into_dataframe_projected(None)
    }

    /// Consume the reader and produce a Polars [`DataFrame`] containing only
    /// the specified columns.
    ///
    /// This avoids the cost of parsing, transcoding, and allocating columns
    /// that are not needed. Decompression still processes all record data
    /// (columns are interleaved within records), but the column-building
    /// phase — which dominates for string-heavy schemas — is limited to the
    /// requested subset.
    ///
    /// If `columns` is `None`, all columns are returned.
    pub fn into_dataframe_projected(mut self, columns: Option<&[&str]>) -> Result<DataFrame> {
        let num_records = self.header.num_records as usize;
        let fields = std::mem::take(&mut self.fields);

        // Fast path: 0-row DataFrame — return empty columns with correct schema
        if num_records == 0 {
            let projected_fields: Vec<&FieldMeta> = match columns {
                Some(names) => {
                    let field_map: std::collections::HashMap<&str, &FieldMeta> =
                        fields.iter().map(|f| (f.name.as_str(), f)).collect();
                    let unknown: Vec<&str> = names
                        .iter()
                        .copied()
                        .filter(|n| !field_map.contains_key(n))
                        .collect();
                    if !unknown.is_empty() {
                        return Err(YxdbError::InvalidFile(format!(
                            "requested columns not found in file: {:?}",
                            unknown
                        )));
                    }
                    names
                        .iter()
                        .filter_map(|n| field_map.get(n).copied())
                        .collect()
                }
                None => fields.iter().collect(),
            };
            let empty_cols: Vec<Column> = projected_fields
                .iter()
                .map(|f| {
                    let series = empty_series_for_field(f);
                    Column::from(series)
                })
                .collect();
            return DataFrame::new(0, empty_cols)
                .map_err(|e| YxdbError::ConversionError(e.to_string()));
        }

        let fixed_size = self.fixed_size;
        let has_var = self.has_var;
        let compression = self.compression;
        let record_block_index_pos = self.header.record_block_index_pos;

        // Phase 1: Memory-map remaining file data (avoids heap allocation + copy)
        let mmap = {
            let inner_stream = self.stream;
            let file = inner_stream.into_inner();
            // SAFETY: The file is opened read-only and its lifetime is bound to
            // this scope.  The mmap is not shared across threads and is only
            // accessed via immutable references after creation.
            unsafe { Mmap::map(&file) }?
        };
        let data_offset = {
            // Compute data start: header + metadata
            let meta_byte_len = self.header.meta_info_size as usize * 2;
            HEADER_SIZE + meta_byte_len
        };
        // Limit raw_data to just the block data region (excludes the RecordBlockIndex).
        let block_data_end = if record_block_index_pos > data_offset as i64 {
            (record_block_index_pos as usize).min(mmap.len())
        } else {
            mmap.len()
        };
        let raw_data = &mmap[data_offset..block_data_end];

        // Phase 2: Get decompressed record data.
        // Version 0 (compression = None): raw_data IS the record data (no block framing).
        //   Borrow directly from the mmap to avoid a full-file heap copy.
        // Version 1+ (compression = Some): parse LZF block boundaries, decompress into owned Vec.
        let all_data: Cow<'_, [u8]> = match compression {
            None => {
                // Version 0: no block framing — records stored directly in the stream
                Cow::Borrowed(raw_data)
            }
            Some(algo) => {
                let mut data = decompress_blocks(raw_data, algo, None)?;

                // For spatial-index files with variable-length records: verify
                // that all records parse correctly.  If not, the block stream
                // contains interleaved spatial grid blocks that must be
                // filtered out and the data re-decompressed.
                if self.header.has_spatial_index() && has_var {
                    if !records_fit(&data, fixed_size, num_records) {
                        data = decompress_blocks(raw_data, algo, Some(fixed_size))?;
                    }
                }

                Cow::Owned(data)
            }
        };

        // Phase 3: Compute record boundaries
        let bounds = if has_var {
            scan_variable_record_bounds(&all_data, fixed_size, num_records)
        } else {
            RecordBounds::Fixed { fixed_size }
        };

        // Phase 4: Build columns (parallel when beneficial)
        // Filter to requested columns if projection was specified.
        // Use HashSet for O(1) lookup, and preserve caller-specified column order.
        let projected_fields: Vec<&FieldMeta> = match columns {
            Some(names) => {
                let name_set: HashSet<&str> = names.iter().copied().collect();
                let field_map: HashMap<&str, &FieldMeta> = fields
                    .iter()
                    .filter(|f| name_set.contains(f.name.as_str()))
                    .map(|f| (f.name.as_str(), f))
                    .collect();
                // Check for unknown column names
                let unknown: Vec<&str> = names
                    .iter()
                    .copied()
                    .filter(|n| !field_map.contains_key(n))
                    .collect();
                if !unknown.is_empty() {
                    return Err(YxdbError::InvalidFile(format!(
                        "requested columns not found in file: {:?}",
                        unknown
                    )));
                }
                // Preserve caller's requested column order
                names
                    .iter()
                    .filter_map(|n| field_map.get(n).copied())
                    .collect()
            }
            None => fields.iter().collect(),
        };

        // Use parallel column building when:
        // - Many columns (>= 6): per-column work adds up even for simple types
        // - Large data (>= 10MB): per-column work is significant even with few columns
        //   (e.g. 5 string columns requiring UTF-16 → UTF-8 transcoding)
        const MIN_COLS_FOR_PAR: usize = 6;
        const MIN_DATA_FOR_PAR: usize = 10 * 1024 * 1024; // 10 MB

        let built_columns: Result<Vec<Column>> =
            if projected_fields.len() >= MIN_COLS_FOR_PAR || all_data.len() >= MIN_DATA_FOR_PAR {
                projected_fields
                    .par_iter()
                    .map(|field| build_column(field, &all_data, &bounds, num_records))
                    .collect()
            } else {
                projected_fields
                    .iter()
                    .map(|field| build_column(field, &all_data, &bounds, num_records))
                    .collect()
            };

        let cols = built_columns?;
        let height = cols.first().map_or(0, |c| c.len());
        DataFrame::new(height, cols).map_err(|e| YxdbError::ConversionError(e.to_string()))
    }

    /// Read the next batch of up to `batch_size` records as a [`DataFrame`].
    ///
    /// Returns `None` when all records have been consumed. This enables
    /// streaming/memory-efficient processing of large YXDB files.
    ///
    /// If `columns` is `Some`, only the named columns are materialised;
    /// every record is still read in full (YXDB is row-interleaved) but
    /// unneeded columns are skipped during the column-building phase.
    ///
    /// ```no_run
    /// use sigilyx::YxdbReader;
    ///
    /// let mut reader = YxdbReader::open("large_file.yxdb").unwrap();
    /// // Read all columns:
    /// while let Some(batch) = reader.next_batch(65_536, None).unwrap() {
    ///     println!("batch: {} rows", batch.height());
    /// }
    ///
    /// // Read only selected columns:
    /// let mut reader = YxdbReader::open("large_file.yxdb").unwrap();
    /// while let Some(batch) = reader.next_batch(65_536, Some(&["col_a", "col_b"])).unwrap() {
    ///     println!("projected batch: {} cols", batch.width());
    /// }
    /// ```
    pub fn next_batch(
        &mut self,
        batch_size: usize,
        columns: Option<&[&str]>,
    ) -> Result<Option<DataFrame>> {
        if self.current_record >= self.header.num_records {
            return Ok(None);
        }

        let remaining = (self.header.num_records - self.current_record) as usize;
        let this_batch = remaining.min(batch_size);

        // Determine which field indices to materialise and clone only those fields.
        let projected_indices: Vec<usize> = match columns {
            Some(names) => {
                let field_map: HashMap<&str, usize> = self
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (f.name.as_str(), i))
                    .collect();
                // Check for unknown column names
                let unknown: Vec<&str> = names
                    .iter()
                    .copied()
                    .filter(|n| !field_map.contains_key(n))
                    .collect();
                if !unknown.is_empty() {
                    return Err(YxdbError::InvalidFile(format!(
                        "requested columns not found in file: {:?}",
                        unknown
                    )));
                }
                names
                    .iter()
                    .filter_map(|n| field_map.get(n).copied())
                    .collect()
            }
            None => (0..self.fields.len()).collect(),
        };
        // Clone only the projected fields so we drop the borrow on self.fields.
        let projected_fields: Vec<FieldMeta> = projected_indices
            .iter()
            .map(|&i| self.fields[i].clone())
            .collect();

        let mut builders: Vec<ColumnBuilder> = projected_fields
            .iter()
            .map(|f| ColumnBuilder::new(f, this_batch))
            .collect();

        let mut record_buf = Vec::with_capacity(self.fixed_size + 1024);
        let mut count = 0;
        while count < this_batch {
            if !self.next_record(&mut record_buf)? {
                break;
            }
            for (bi, field) in projected_fields.iter().enumerate() {
                builders[bi].push_from_record(&record_buf, field)?;
            }
            count += 1;
        }

        if count == 0 {
            return Ok(None);
        }

        let columns: Vec<Column> = builders
            .into_iter()
            .zip(projected_fields.iter().map(|f| &f.name))
            .map(|(b, name)| b.into_series(name))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .map(Column::from)
            .collect();

        let height = columns.first().map_or(0, |c| c.len());
        let df = DataFrame::new(height, columns)
            .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
        Ok(Some(df))
    }

    // ── LZF block reading ──────────────────────────────────────────────

    /// Read exactly `size` bytes from the (possibly compressed) stream into `dest`.
    fn read_bytes(&mut self, dest: &mut [u8]) -> Result<()> {
        if self.compression.is_none() {
            // Version 0: no block framing, read directly from stream
            self.stream.read_exact(dest)?;
            return Ok(());
        }

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

    /// Read and decompress the next compressed block from the stream.
    ///
    /// Only called when `self.compression` is `Some(_)` (block-framed data).
    /// When the file has a spatial index, spatial grid blocks are automatically
    /// detected and skipped so the caller sees only record data.
    fn read_next_lzf_block(&mut self) -> Result<()> {
        let algo = self
            .compression
            .expect("read_next_lzf_block called with no compression");

        loop {
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
                // Ensure output buffer is large enough for oversized blocks.
                // Standard blocks decompress to at most BLOCK_SIZE (262144),
                // but oversized records may produce larger output.
                let min_out = 262144usize.max(block_len * 10);
                if self.lzf_out.len() < min_out {
                    self.lzf_out.resize(min_out, 0);
                }
                self.lzf_out_size =
                    lzf::decompress_block(algo, &self.lzf_in[..block_len], &mut self.lzf_out)?;
            }

            // Skip spatial index grid blocks (present in files with file_id = 0x00440205).
            if self.header.has_spatial_index()
                && self.has_var
                && !is_record_block(&self.lzf_out[..self.lzf_out_size], self.fixed_size)
            {
                continue;
            }

            self.lzf_out_idx = 0;
            return Ok(());
        }
    }
}

// ── Record scan helpers ───────────────────────────────────────────────

/// Check whether `num_records` variable-length records can be parsed from `data`.
///
/// Returns `true` if all records parse successfully (each record's var_len
/// stays within bounds). Used as a fast pre-check before the full record
/// boundary scan.
fn records_fit(data: &[u8], fixed_size: usize, num_records: usize) -> bool {
    let mut offset = 0usize;
    for _ in 0..num_records {
        let var_start = offset + fixed_size;
        if var_start + 4 > data.len() {
            return false;
        }
        let var_len =
            u32::from_le_bytes(data[var_start..var_start + 4].try_into().unwrap()) as usize;
        offset = var_start + 4 + var_len;
        if offset > data.len() {
            return false;
        }
    }
    true
}

/// Scan variable-length records and return their boundary offsets.
fn scan_variable_record_bounds(data: &[u8], fixed_size: usize, num_records: usize) -> RecordBounds {
    let mut ends = Vec::with_capacity(num_records + 1);
    ends.push(0usize);
    let mut offset = 0usize;
    for _ in 0..num_records {
        let var_start = offset + fixed_size;
        if var_start + 4 > data.len() {
            break;
        }
        let var_len =
            u32::from_le_bytes(data[var_start..var_start + 4].try_into().unwrap()) as usize;
        offset = var_start + 4 + var_len;
        ends.push(offset);
    }
    RecordBounds::Variable { ends }
}

// ── Spatial index block detection ─────────────────────────────────────

/// Check whether a decompressed LZF block contains record data.
///
/// Files with `file_id = 0x00440205` (WrigleyDB with spatial index) interleave
/// spatial index grid blocks within the LZF block stream.  These blocks contain
/// bounding-box / grid-cell data for the spatial index, **not** record data, and
/// must be excluded from the record stream.
///
/// Detection heuristic: every record block begins at a record boundary, so we
/// can parse the first few records from the start of the block.  If multiple
/// consecutive records all have variable-length sizes that fit within the
/// block, it is record data.  If any "record" has a variable length that
/// overflows the block, the block is spatial data.  Checking more than one
/// record makes the heuristic robust even when `fixed_size` is small and a
/// single probe could accidentally align with valid-looking bytes in the
/// spatial grid.
#[inline]
fn is_record_block(block_data: &[u8], fixed_size: usize) -> bool {
    // Verify up to 3 consecutive records starting from byte 0.
    const PROBES: usize = 3;
    let mut offset = 0usize;
    for _ in 0..PROBES {
        if offset + fixed_size + 4 > block_data.len() {
            // Not enough room for another record — all previous probes passed.
            break;
        }
        let var_len = u32::from_le_bytes(
            block_data[offset + fixed_size..offset + fixed_size + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let record_end = offset + fixed_size + 4 + var_len;
        if record_end > block_data.len() {
            return false; // variable data overflows the block → not record data
        }
        offset = record_end;
    }
    true
}

// ── Block decompression ───────────────────────────────────────────────

/// Decompress LZF block-framed data into a contiguous buffer.
///
/// Parses 4-byte block length headers, decompresses blocks in parallel
/// for large files, and compacts the result into a single contiguous buffer.
///
/// When `spatial_record_filter` is `Some(fixed_size)`, spatial index grid
/// blocks (present in files with `file_id = 0x00440205`) are detected after
/// decompression and excluded from the compacted output, so the returned
/// buffer contains only record data.
fn decompress_blocks(
    raw_data: &[u8],
    algo: CompressionAlgorithm,
    spatial_record_filter: Option<usize>,
) -> Result<Vec<u8>> {
    // Parse block boundaries (sequential scan — just reading 4-byte lengths)
    let mut blocks: Vec<(usize, usize, bool)> = Vec::new(); // (data_offset, length, is_uncompressed)
    let mut pos = 0usize;
    while pos + 4 <= raw_data.len() {
        let raw_len = u32::from_le_bytes(raw_data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        let is_uncompressed = raw_len & 0x80000000 != 0;
        let block_len = raw_len & 0x7FFFFFFF;
        if pos + block_len > raw_data.len() {
            break;
        }
        blocks.push((pos, block_len, is_uncompressed));
        pos += block_len;
    }

    const BLOCK_SIZE: usize = 262144;
    const MIN_BLOCKS_FOR_PAR: usize = 8;

    let num_blocks = blocks.len();

    // Pre-compute expected output sizes and cumulative offsets.
    // For compressed blocks, the decompressed size is at most BLOCK_SIZE for
    // standard blocks, but may be larger if the writer emitted an oversized
    // block (e.g. a single record > BLOCK_SIZE). Use a conservative estimate
    // of max(BLOCK_SIZE, compressed_length * 10) to handle both cases, and
    // retry with a larger buffer if decompression fails.
    let mut block_offsets: Vec<usize> = Vec::with_capacity(num_blocks + 1);
    block_offsets.push(0);
    for (idx, &(_offset, length, is_uncompressed)) in blocks.iter().enumerate() {
        let expected_output = if is_uncompressed {
            length
        } else {
            // Most blocks decompress to exactly BLOCK_SIZE. However, oversized
            // records may produce blocks > BLOCK_SIZE. Use a generous estimate.
            BLOCK_SIZE.max(length * 10)
        };
        block_offsets.push(block_offsets[idx] + expected_output);
    }
    let max_total = *block_offsets.last().unwrap_or(&0);

    let mut all_data: Vec<u8> = vec![0u8; max_total.max(1)];
    let all_data_ptr = all_data.as_mut_ptr();

    // SAFETY: Each block writes to a disjoint sub-slice of `all_data`.
    // Block `idx` writes to `[block_offsets[idx]..block_offsets[idx+1])`.
    // These ranges are non-overlapping by construction.
    //
    // Wrap the raw pointer so we can share it across Rayon threads.
    struct SendSyncPtr(*mut u8);
    // SAFETY: The raw pointer is only used to produce disjoint mutable slices
    // of `all_data`, one per block. No two threads access the same range.
    unsafe impl Send for SendSyncPtr {}
    // SAFETY: Same rationale — concurrent reads/writes target disjoint sub-slices.
    unsafe impl Sync for SendSyncPtr {}
    impl SendSyncPtr {
        /// Return a mutable slice starting at `offset` with length `len`.
        /// SAFETY: caller must ensure the range is valid and non-overlapping
        /// with any other concurrent slice from this pointer.
        #[inline]
        #[allow(clippy::mut_from_ref)]
        unsafe fn slice_mut(&self, offset: usize, len: usize) -> &mut [u8] {
            // SAFETY: Upheld by caller — see doc comment above.
            unsafe { std::slice::from_raw_parts_mut(self.0.add(offset), len) }
        }
    }
    let ptr = SendSyncPtr(all_data_ptr);

    let mut block_sizes: Vec<usize> = if num_blocks >= MIN_BLOCKS_FOR_PAR {
        let results: Result<Vec<usize>> = blocks
            .par_iter()
            .enumerate()
            .map(|(idx, &(offset, length, is_uncompressed))| {
                let dest_start = block_offsets[idx];
                let dest_capacity = block_offsets[idx + 1] - dest_start;
                // SAFETY: Each block writes to a disjoint sub-slice; indices are in-bounds.
                let dest = unsafe { ptr.slice_mut(dest_start, dest_capacity) };
                if is_uncompressed {
                    dest[..length].copy_from_slice(&raw_data[offset..offset + length]);
                    Ok(length)
                } else {
                    lzf::decompress_block_into(algo, &raw_data[offset..offset + length], dest)
                }
            })
            .collect();
        results?
    } else {
        let results: Result<Vec<usize>> = blocks
            .iter()
            .enumerate()
            .map(|(idx, &(offset, length, is_uncompressed))| {
                let dest_start = block_offsets[idx];
                let dest_capacity = block_offsets[idx + 1] - dest_start;
                // SAFETY: Each block writes to a disjoint sub-slice; indices are in-bounds.
                let dest = unsafe { ptr.slice_mut(dest_start, dest_capacity) };
                if is_uncompressed {
                    dest[..length].copy_from_slice(&raw_data[offset..offset + length]);
                    Ok(length)
                } else {
                    lzf::decompress_block_into(algo, &raw_data[offset..offset + length], dest)
                }
            })
            .collect();
        results?
    };

    // Phase 2.5: Filter spatial index grid blocks.
    //
    // Files with a spatial index (file_id = 0x00440205) interleave spatial
    // grid blocks within the LZF block stream.  These blocks contain spatial
    // index data (bounding boxes, grid cells), not record data.  Setting
    // their size to 0 causes the compaction step below to skip them.
    if let Some(fixed_size) = spatial_record_filter {
        for idx in 0..block_sizes.len() {
            let actual_size = block_sizes[idx];
            if actual_size > 0 {
                let start = block_offsets[idx];
                if !is_record_block(&all_data[start..start + actual_size], fixed_size) {
                    block_sizes[idx] = 0;
                }
            }
        }
    }

    // Compact: close gaps between blocks where actual size < allocated BLOCK_SIZE.
    let mut write_pos = 0usize;
    for (idx, &actual_size) in block_sizes.iter().enumerate() {
        let read_pos = block_offsets[idx];
        if write_pos != read_pos && actual_size > 0 {
            // SAFETY: `write_pos < read_pos` is guaranteed because we only
            // compact forward and block offsets are monotonically increasing.
            // The source and destination regions do not extend beyond
            // `all_data.len()` because both are bounded by `block_offsets`
            // and `actual_size <= BLOCK_SIZE`.  We use `ptr::copy` (not
            // `copy_nonoverlapping`) because the regions may overlap.
            unsafe {
                std::ptr::copy(
                    all_data.as_ptr().add(read_pos),
                    all_data.as_mut_ptr().add(write_pos),
                    actual_size,
                );
            }
        }
        write_pos += actual_size;
    }
    all_data.truncate(write_pos);

    Ok(all_data)
}

// ── Record boundary abstraction ───────────────────────────────────────

/// Efficient record boundary lookup, avoiding allocation for fixed-size records.
enum RecordBounds {
    /// All records are `fixed_size` bytes — boundaries computed arithmetically.
    Fixed { fixed_size: usize },
    /// Variable-length records — boundaries stored in a pre-computed Vec.
    Variable { ends: Vec<usize> },
}

impl RecordBounds {
    #[inline(always)]
    fn record_slice<'a>(&self, data: &'a [u8], i: usize) -> Result<&'a [u8]> {
        match self {
            RecordBounds::Fixed { fixed_size } => {
                let start = i * fixed_size;
                let end = start + fixed_size;
                if end > data.len() {
                    return Err(YxdbError::InvalidFile(format!(
                        "record {i} exceeds data bounds: offset {end} > data length {}",
                        data.len()
                    )));
                }
                Ok(&data[start..end])
            }
            RecordBounds::Variable { ends } => {
                let start = ends[i];
                let end = ends[i + 1];
                if end > data.len() {
                    return Err(YxdbError::InvalidFile(format!(
                        "variable record {i} exceeds data bounds: offset {end} > data length {}",
                        data.len()
                    )));
                }
                Ok(&data[start..end])
            }
        }
    }

    #[inline(always)]
    fn num_records(&self, total: usize) -> usize {
        match self {
            RecordBounds::Fixed { .. } => total,
            RecordBounds::Variable { ends } => ends.len() - 1,
        }
    }
}

/// Create an empty Series (0 rows) with the correct dtype for a YXDB field.
fn empty_series_for_field(field: &FieldMeta) -> Series {
    use crate::field::FieldType;
    let name = PlSmallStr::from(field.name.as_str());
    match field.field_type {
        FieldType::Bool => Series::new_empty(name, &DataType::Boolean),
        FieldType::Byte | FieldType::Int16 => Series::new_empty(name, &DataType::Int16),
        FieldType::Int32 => Series::new_empty(name, &DataType::Int32),
        FieldType::Int64 => Series::new_empty(name, &DataType::Int64),
        FieldType::Float => Series::new_empty(name, &DataType::Float32),
        FieldType::Double => Series::new_empty(name, &DataType::Float64),
        FieldType::FixedDecimal => {
            // YXDB `size` is the total ASCII char width (digits + sign + decimal point).
            // Invert the writer's formula: precision = size - 1 (sign) - 1 (decimal point if scale > 0)
            let raw_p = if field.size > 1 {
                field.size - 1 - if field.scale > 0 { 1 } else { 0 }
            } else {
                19 // fallback for legacy files
            };
            let p = raw_p.min(38); // Polars caps precision at 38
            let s = field.scale;
            Series::new_empty(name, &DataType::Decimal(p, s))
        }
        FieldType::String | FieldType::WString | FieldType::VString | FieldType::VWString => {
            Series::new_empty(name, &DataType::String)
        }
        FieldType::Date => Series::new_empty(name, &DataType::Date),
        FieldType::Time => Series::new_empty(name, &DataType::Time),
        FieldType::DateTime => {
            Series::new_empty(name, &DataType::Datetime(TimeUnit::Microseconds, None))
        }
        FieldType::Blob | FieldType::SpatialObj => Series::new_empty(name, &DataType::Binary),
    }
}

/// Build a single column from decompressed record data.
fn build_column(
    field: &FieldMeta,
    all_data: &[u8],
    bounds: &RecordBounds,
    num_records: usize,
) -> Result<Column> {
    let n = bounds.num_records(num_records);
    let mut builder = ColumnBuilder::new(field, n);
    for i in 0..n {
        let record = bounds.record_slice(all_data, i)?;
        builder.push_from_record(record, field)?;
    }
    Ok(Column::from(builder.into_series(&field.name)?))
}

/// Parse an ASCII decimal string (e.g. "1234.5678") to an i128 unscaled value
/// with the given scale. For scale=4: "1234.5678" → 12345678, "-0.01" → -100.
fn parse_decimal_i128(s: &str, scale: usize) -> i128 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    let (neg, s) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };
    let (int_part, frac_part) = match s.find('.') {
        Some(dot) => (&s[..dot], &s[dot + 1..]),
        None => (s, ""),
    };
    let mut result: i128 = 0;
    for &b in int_part.as_bytes() {
        if b.is_ascii_digit() {
            result = result * 10 + (b - b'0') as i128;
        }
    }
    let frac_bytes = frac_part.as_bytes();
    for i in 0..scale {
        result *= 10;
        if i < frac_bytes.len() {
            let b = frac_bytes[i];
            if b.is_ascii_digit() {
                result += (b - b'0') as i128;
            }
        }
    }
    if neg {
        -result
    } else {
        result
    }
}

// ── Column builders ────────────────────────────────────────────────────

/// Accumulates values for a single column and converts to a Polars [`Series`].
///
/// Numeric types use direct Arrow array construction with separate value and
/// validity buffers, avoiding the overhead of `Vec<Option<T>>` → `Series::new`.
/// String types use Polars' `StringChunkedBuilder` (already optimal).
enum ColumnBuilder {
    Bool {
        values: Vec<bool>,
        validity: MutableBitmap,
        has_nulls: bool,
    },
    Byte {
        values: Vec<i16>,
        validity: MutableBitmap,
        has_nulls: bool,
    },
    Int16 {
        values: Vec<i16>,
        validity: MutableBitmap,
        has_nulls: bool,
    },
    Int32 {
        values: Vec<i32>,
        validity: MutableBitmap,
        has_nulls: bool,
    },
    Int64 {
        values: Vec<i64>,
        validity: MutableBitmap,
        has_nulls: bool,
    },
    Float {
        values: Vec<f32>,
        validity: MutableBitmap,
        has_nulls: bool,
    },
    Double {
        values: Vec<f64>,
        validity: MutableBitmap,
        has_nulls: bool,
    },
    /// FixedDecimal stored as i128 (unscaled) for lossless Polars Decimal construction.
    Decimal {
        values: Vec<i128>,
        validity: MutableBitmap,
        has_nulls: bool,
        precision: usize,
        scale: usize,
    },
    /// Zero-allocation string builder using Polars StringChunkedBuilder.
    /// `str_buf` is a reusable buffer for UTF-16 → UTF-8 transcoding.
    StrBuilder {
        builder: StringChunkedBuilder,
        str_buf: String,
    },
    /// Date stored as days-since-epoch (i32) for direct Polars Date construction.
    DateDays {
        values: Vec<i32>,
        validity: MutableBitmap,
        has_nulls: bool,
    },
    /// Time stored as nanoseconds since midnight for direct Polars Time construction.
    TimeNs {
        values: Vec<i64>,
        validity: MutableBitmap,
        has_nulls: bool,
    },
    /// DateTime stored as us-since-epoch (i64) for direct Polars Datetime construction.
    DateTimeUs {
        values: Vec<i64>,
        validity: MutableBitmap,
        has_nulls: bool,
    },
    Blob(Vec<Option<Vec<u8>>>),
}

/// Hinnant's algorithm raw-day value for 1970-01-01 (Unix epoch).
const UNIX_EPOCH_DAYS: i32 = 719_468;

/// Parse "YYYY-MM-DD" ASCII bytes directly to days-since-Unix-epoch.
#[inline]
fn parse_date_to_days(buf: &[u8]) -> Option<i32> {
    // buf must be at least 10 bytes: "YYYY-MM-DD"
    if buf.len() < 10 {
        return None;
    }
    let y = parse_4_digits(buf)? as i32;
    let m = parse_2_digits(&buf[5..])? as u32;
    let d = parse_2_digits(&buf[8..])? as u32;
    Some(civil_to_days(y, m, d))
}

/// Parse "HH:MM:SS" ASCII bytes directly to nanoseconds since midnight.
/// Polars Time type stores values as i64 nanoseconds.
#[inline]
fn parse_time_to_ns(buf: &[u8]) -> Option<i64> {
    if buf.len() < 8 {
        return None;
    }
    let h = parse_2_digits(buf)? as i64;
    let m = parse_2_digits(&buf[3..])? as i64;
    let s = parse_2_digits(&buf[6..])? as i64;
    Some((h * 3600 + m * 60 + s) * 1_000_000_000)
}

/// Parse "YYYY-MM-DD HH:MM:SS" ASCII bytes directly to us-since-Unix-epoch.
#[inline]
fn parse_datetime_to_us(buf: &[u8]) -> Option<i64> {
    if buf.len() < 19 {
        return None;
    }
    let days = parse_date_to_days(buf)? as i64;
    let h = parse_2_digits(&buf[11..])? as i64;
    let min = parse_2_digits(&buf[14..])? as i64;
    let s = parse_2_digits(&buf[17..])? as i64;
    Some(days * 86_400_000_000 + h * 3_600_000_000 + min * 60_000_000 + s * 1_000_000)
}

#[inline]
fn parse_2_digits(b: &[u8]) -> Option<u16> {
    let d0 = b[0].wrapping_sub(b'0');
    let d1 = b[1].wrapping_sub(b'0');
    if d0 > 9 || d1 > 9 {
        return None;
    }
    Some(d0 as u16 * 10 + d1 as u16)
}

#[inline]
fn parse_4_digits(b: &[u8]) -> Option<u16> {
    let d0 = b[0].wrapping_sub(b'0') as u16;
    let d1 = b[1].wrapping_sub(b'0') as u16;
    let d2 = b[2].wrapping_sub(b'0') as u16;
    let d3 = b[3].wrapping_sub(b'0') as u16;
    if d0 > 9 || d1 > 9 || d2 > 9 || d3 > 9 {
        return None;
    }
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

/// Read N bytes from buf at offset without bounds checking.
///
/// # Safety
/// Caller must ensure `off + N <= buf.len()`.
#[inline(always)]
unsafe fn read_bytes_unchecked<const N: usize>(buf: &[u8], off: usize) -> [u8; N] {
    let mut out = [0u8; N];
    // SAFETY: Upheld by caller — `off + N <= buf.len()`.
    unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr().add(off), out.as_mut_ptr(), N) };
    out
}

/// Fast UTF-16LE to UTF-8 transcoding, writing directly into a String's buffer.
///
/// Uses SSE2-accelerated batch ASCII fast-path on x86_64, processing 8 code
/// units at a time when all are ASCII. Falls back to scalar for non-ASCII chunks.
#[inline]
fn transcode_utf16le(bytes: &[u8], out: &mut String) {
    out.clear();
    let n = bytes.len() / 2;
    out.reserve(n);

    // SAFETY: We maintain the UTF-8 invariant by only pushing valid UTF-8
    // sequences — ASCII bytes are validated via `is_ascii` before push, and
    // non-ASCII code units go through `char::from_u32` / `encode_utf8`.
    let v = unsafe { out.as_mut_vec() };
    let mut i = 0;

    // SSE2 fast path: process 8 UTF-16LE code units (16 bytes) at a time
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // SSE2 is always available on x86_64
        // SAFETY: SSE2 intrinsics operate on the raw byte slice within bounds
        // (guarded by `i + 16 <= bytes.len()`). Output bytes are valid UTF-8
        // because we only fast-path pure-ASCII code units (< 128).
        unsafe {
            let zero = _mm_setzero_si128();
            // Mask to extract the high byte of each u16: 0xFF00 repeated
            let hi_byte_mask = _mm_set1_epi16(0xFF00u16 as i16);
            while i + 16 <= bytes.len() {
                let chunk = _mm_loadu_si128(bytes.as_ptr().add(i) as *const __m128i);
                // A u16 code unit is ASCII iff its value is in [0, 127], which
                // requires the high byte to be 0x00 AND the low byte to be < 0x80.
                //
                // Check 1: no byte has its high bit set (all bytes < 0x80).
                let byte_mask = _mm_movemask_epi8(chunk);
                // Check 2: the high byte of every u16 is zero.
                // _mm_cmpeq_epi8 + movemask on just the high bytes: if any
                // high byte is nonzero the code unit is >= 256 (not ASCII).
                let hi_bytes = _mm_and_si128(chunk, hi_byte_mask);
                let hi_is_zero = _mm_cmpeq_epi8(hi_bytes, zero);
                let hi_mask = _mm_movemask_epi8(hi_is_zero);
                // hi_mask == 0xFFFF means every byte matched zero after masking
                // (i.e. all high bytes of each u16 are 0x00).
                if byte_mask == 0 && hi_mask == 0xFFFF {
                    // All 8 code units are ASCII — pack to 8 bytes
                    let packed = _mm_packus_epi16(chunk, zero);
                    // Write 8 bytes at once
                    v.reserve(8);
                    let len = v.len();
                    std::ptr::copy_nonoverlapping(
                        &packed as *const __m128i as *const u8,
                        v.as_mut_ptr().add(len),
                        8,
                    );
                    v.set_len(len + 8);
                    i += 16;
                } else {
                    // Non-ASCII chunk: process one code unit scalar, then retry SIMD.
                    // For surrogates (rare, >U+FFFF chars), bail to the full scalar
                    // path which handles paired surrogates correctly.
                    let lo = bytes[i];
                    let hi = bytes[i + 1];
                    let cu = u16::from_le_bytes([lo, hi]);
                    if (0xD800..=0xDFFF).contains(&cu) {
                        break; // surrogate — let scalar path handle the pair
                    }
                    if hi == 0 && lo < 0x80 {
                        v.push(lo);
                    } else if cu < 0x800 {
                        v.push(0xC0 | ((cu >> 6) as u8));
                        v.push(0x80 | ((cu & 0x3F) as u8));
                    } else {
                        v.push(0xE0 | ((cu >> 12) as u8));
                        v.push(0x80 | (((cu >> 6) & 0x3F) as u8));
                        v.push(0x80 | ((cu & 0x3F) as u8));
                    }
                    i += 2;
                }
            }
        }
    }

    // Scalar path for remaining bytes (or all bytes on non-x86_64)
    while i + 1 < bytes.len() {
        let lo = bytes[i];
        let hi = bytes[i + 1];

        if hi == 0 {
            if lo < 0x80 {
                v.push(lo);
            } else {
                // Latin-1 range (U+0080..U+00FF) -> 2-byte UTF-8
                v.push(0xC0 | (lo >> 6));
                v.push(0x80 | (lo & 0x3F));
            }
        } else {
            let cu = u16::from_le_bytes([lo, hi]);
            if cu < 0x800 {
                v.push(0xC0 | ((cu >> 6) as u8));
                v.push(0x80 | ((cu & 0x3F) as u8));
            } else if (0xD800..=0xDBFF).contains(&cu) {
                // High surrogate
                i += 2;
                if i + 1 < bytes.len() {
                    let cu2 = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
                    if (0xDC00..=0xDFFF).contains(&cu2) {
                        let cp = 0x10000 + ((cu as u32 - 0xD800) << 10) + (cu2 as u32 - 0xDC00);
                        v.push(0xF0 | ((cp >> 18) as u8));
                        v.push(0x80 | (((cp >> 12) & 0x3F) as u8));
                        v.push(0x80 | (((cp >> 6) & 0x3F) as u8));
                        v.push(0x80 | ((cp & 0x3F) as u8));
                    } else {
                        v.extend_from_slice(&[0xEF, 0xBF, 0xBD]);
                    }
                } else {
                    v.extend_from_slice(&[0xEF, 0xBF, 0xBD]);
                }
            } else if (0xDC00..=0xDFFF).contains(&cu) {
                v.extend_from_slice(&[0xEF, 0xBF, 0xBD]);
            } else {
                // BMP non-surrogate -> 3-byte UTF-8
                v.push(0xE0 | ((cu >> 12) as u8));
                v.push(0x80 | (((cu >> 6) & 0x3F) as u8));
                v.push(0x80 | ((cu & 0x3F) as u8));
            }
        }

        i += 2;
    }
}

impl ColumnBuilder {
    fn new(field: &FieldMeta, capacity: usize) -> Self {
        match field.field_type {
            FieldType::Bool => ColumnBuilder::Bool {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
            },
            FieldType::Byte => ColumnBuilder::Byte {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
            },
            FieldType::Int16 => ColumnBuilder::Int16 {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
            },
            FieldType::Int32 => ColumnBuilder::Int32 {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
            },
            FieldType::Int64 => ColumnBuilder::Int64 {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
            },
            FieldType::Float => ColumnBuilder::Float {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
            },
            FieldType::Double => ColumnBuilder::Double {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
            },
            FieldType::FixedDecimal => ColumnBuilder::Decimal {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
                // Invert writer's formula: YXDB size = precision + 1 (sign) + 1 (decimal point if scale > 0)
                precision: {
                    let raw_p = if field.size > 1 {
                        field.size - 1 - if field.scale > 0 { 1 } else { 0 }
                    } else {
                        19
                    };
                    raw_p.min(38)
                },
                scale: field.scale,
            },
            FieldType::String | FieldType::WString | FieldType::VString | FieldType::VWString => {
                ColumnBuilder::StrBuilder {
                    builder: StringChunkedBuilder::new(
                        PlSmallStr::from(field.name.as_str()),
                        capacity,
                    ),
                    str_buf: String::with_capacity(64),
                }
            }
            FieldType::Date => ColumnBuilder::DateDays {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
            },
            FieldType::Time => ColumnBuilder::TimeNs {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
            },
            FieldType::DateTime => ColumnBuilder::DateTimeUs {
                values: Vec::with_capacity(capacity),
                validity: MutableBitmap::with_capacity(capacity),
                has_nulls: false,
            },
            FieldType::Blob | FieldType::SpatialObj => {
                ColumnBuilder::Blob(Vec::with_capacity(capacity))
            }
        }
    }

    /// Push a value directly from the record buffer into this builder.
    ///
    /// This is the hot-path method — it avoids creating any intermediate
    /// `FieldValue` enum and parses dates/datetimes to native integers.
    ///
    /// # Safety rationale for `unsafe` blocks
    /// Field offsets and sizes are computed from the YXDB header metadata in
    /// `YxdbReader::open`. The record buffer is read by `next_record` which
    /// reads exactly `fixed_size` bytes for the fixed portion. Therefore
    /// `field.offset + field.fixed_bytes() <= fixed_size <= record.len()`
    /// holds for all fixed-size accesses.
    #[inline]
    #[allow(clippy::undocumented_unsafe_blocks)]
    fn push_from_record(&mut self, record: &[u8], field: &FieldMeta) -> Result<()> {
        let off = field.offset;
        match self {
            ColumnBuilder::Bool {
                values,
                validity,
                has_nulls,
            } => {
                let b = unsafe { *record.get_unchecked(off) };
                if b == 2 {
                    values.push(false);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    values.push(b == 1);
                    validity.push(true);
                }
            }
            ColumnBuilder::Byte {
                values,
                validity,
                has_nulls,
            } => unsafe {
                if *record.get_unchecked(off + 1) == 1 {
                    values.push(0);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    values.push(*record.get_unchecked(off) as i16);
                    validity.push(true);
                }
            },
            ColumnBuilder::Int16 {
                values,
                validity,
                has_nulls,
            } => unsafe {
                if *record.get_unchecked(off + 2) == 1 {
                    values.push(0);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    values.push(i16::from_le_bytes(read_bytes_unchecked::<2>(record, off)));
                    validity.push(true);
                }
            },
            ColumnBuilder::Int32 {
                values,
                validity,
                has_nulls,
            } => unsafe {
                if *record.get_unchecked(off + 4) == 1 {
                    values.push(0);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    values.push(i32::from_le_bytes(read_bytes_unchecked::<4>(record, off)));
                    validity.push(true);
                }
            },
            ColumnBuilder::Int64 {
                values,
                validity,
                has_nulls,
            } => unsafe {
                if *record.get_unchecked(off + 8) == 1 {
                    values.push(0);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    values.push(i64::from_le_bytes(read_bytes_unchecked::<8>(record, off)));
                    validity.push(true);
                }
            },
            ColumnBuilder::Float {
                values,
                validity,
                has_nulls,
            } => unsafe {
                if *record.get_unchecked(off + 4) == 1 {
                    values.push(0.0);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    values.push(f32::from_le_bytes(read_bytes_unchecked::<4>(record, off)));
                    validity.push(true);
                }
            },
            ColumnBuilder::Double {
                values,
                validity,
                has_nulls,
            } => unsafe {
                if *record.get_unchecked(off + 8) == 1 {
                    values.push(0.0);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    values.push(f64::from_le_bytes(read_bytes_unchecked::<8>(record, off)));
                    validity.push(true);
                }
            },
            ColumnBuilder::Decimal {
                values,
                validity,
                has_nulls,
                scale,
                ..
            } => unsafe {
                if *record.get_unchecked(off + field.size) == 1 {
                    values.push(0);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    let slice = &record[off..off + field.size];
                    let len = slice.iter().position(|&b| b == 0).unwrap_or(field.size);
                    let s = std::str::from_utf8(&slice[..len]).unwrap_or("0");
                    values.push(parse_decimal_i128(s, *scale));
                    validity.push(true);
                }
            },
            ColumnBuilder::StrBuilder { builder, str_buf } => match field.field_type {
                FieldType::String => unsafe {
                    if *record.get_unchecked(off + field.size) == 1 {
                        builder.append_null();
                    } else {
                        let slice = &record[off..off + field.size];
                        let len = slice.iter().position(|&b| b == 0).unwrap_or(field.size);
                        match std::str::from_utf8(&slice[..len]) {
                            Ok(s) => builder.append_value(s),
                            Err(_) => {
                                let cow = String::from_utf8_lossy(&slice[..len]);
                                builder.append_value(cow.as_ref());
                            }
                        }
                    }
                },
                FieldType::WString => {
                    let null_byte_off = off + field.size * 2;
                    unsafe {
                        if *record.get_unchecked(null_byte_off) == 1 {
                            builder.append_null();
                        } else {
                            let byte_len = field.size * 2;
                            let slice = &record[off..off + byte_len];
                            let char_count = slice
                                .chunks_exact(2)
                                .position(|c| c[0] == 0 && c[1] == 0)
                                .unwrap_or(field.size);
                            transcode_utf16le(&slice[..char_count * 2], str_buf);
                            builder.append_value(str_buf.as_str());
                        }
                    }
                }
                FieldType::VString => match record::locate_var_data(record, off) {
                    None => builder.append_null(),
                    Some([]) => builder.append_value(""),
                    Some(bytes) => match std::str::from_utf8(bytes) {
                        Ok(s) => builder.append_value(s),
                        Err(_) => {
                            let cow = String::from_utf8_lossy(bytes);
                            builder.append_value(cow.as_ref());
                        }
                    },
                },
                FieldType::VWString => match record::locate_var_data(record, off) {
                    None => builder.append_null(),
                    Some([]) => builder.append_value(""),
                    Some(bytes) => {
                        transcode_utf16le(bytes, str_buf);
                        builder.append_value(str_buf.as_str());
                    }
                },
                _ => unreachable!(),
            },
            ColumnBuilder::DateDays {
                values,
                validity,
                has_nulls,
            } => unsafe {
                if *record.get_unchecked(off + 10) == 1 {
                    values.push(0);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    match parse_date_to_days(&record[off..off + 10]) {
                        Some(v) => {
                            values.push(v);
                            validity.push(true);
                        }
                        None => {
                            values.push(0);
                            validity.push(false);
                            *has_nulls = true;
                        }
                    }
                }
            },
            ColumnBuilder::TimeNs {
                values,
                validity,
                has_nulls,
            } => unsafe {
                if *record.get_unchecked(off + 8) == 1 {
                    values.push(0);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    match parse_time_to_ns(&record[off..off + 8]) {
                        Some(v) => {
                            values.push(v);
                            validity.push(true);
                        }
                        None => {
                            values.push(0);
                            validity.push(false);
                            *has_nulls = true;
                        }
                    }
                }
            },
            ColumnBuilder::DateTimeUs {
                values,
                validity,
                has_nulls,
            } => unsafe {
                if *record.get_unchecked(off + 19) == 1 {
                    values.push(0);
                    validity.push(false);
                    *has_nulls = true;
                } else {
                    match parse_datetime_to_us(&record[off..off + 19]) {
                        Some(v) => {
                            values.push(v);
                            validity.push(true);
                        }
                        None => {
                            values.push(0);
                            validity.push(false);
                            *has_nulls = true;
                        }
                    }
                }
            },
            ColumnBuilder::Blob(v) => {
                v.push(record::parse_var_data(record, off));
            }
        }
        Ok(())
    }

    fn into_series(self, name: &str) -> Result<Series> {
        let s = match self {
            ColumnBuilder::Bool {
                values,
                validity,
                has_nulls,
            } => {
                if has_nulls {
                    // Build BooleanArray with validity directly
                    let bitmap: arrow::bitmap::Bitmap = validity.into();
                    let values_bitmap = arrow::bitmap::Bitmap::from_iter(values.iter().copied());
                    let arr =
                        arrow::array::BooleanArray::from_data_default(values_bitmap, Some(bitmap));
                    let ca = BooleanChunked::with_chunk(name.into(), arr);
                    ca.into_series()
                } else {
                    BooleanChunked::new(name.into(), &values).into_series()
                }
            }
            ColumnBuilder::Byte {
                values,
                validity,
                has_nulls,
            } => {
                if has_nulls {
                    Int16Chunked::from_vec_validity(name.into(), values, Some(validity.into()))
                        .into_series()
                } else {
                    Int16Chunked::from_vec(name.into(), values).into_series()
                }
            }
            ColumnBuilder::Int16 {
                values,
                validity,
                has_nulls,
            } => {
                if has_nulls {
                    Int16Chunked::from_vec_validity(name.into(), values, Some(validity.into()))
                        .into_series()
                } else {
                    Int16Chunked::from_vec(name.into(), values).into_series()
                }
            }
            ColumnBuilder::Int32 {
                values,
                validity,
                has_nulls,
            } => {
                if has_nulls {
                    Int32Chunked::from_vec_validity(name.into(), values, Some(validity.into()))
                        .into_series()
                } else {
                    Int32Chunked::from_vec(name.into(), values).into_series()
                }
            }
            ColumnBuilder::Int64 {
                values,
                validity,
                has_nulls,
            } => {
                if has_nulls {
                    Int64Chunked::from_vec_validity(name.into(), values, Some(validity.into()))
                        .into_series()
                } else {
                    Int64Chunked::from_vec(name.into(), values).into_series()
                }
            }
            ColumnBuilder::Float {
                values,
                validity,
                has_nulls,
            } => {
                if has_nulls {
                    Float32Chunked::from_vec_validity(name.into(), values, Some(validity.into()))
                        .into_series()
                } else {
                    Float32Chunked::from_vec(name.into(), values).into_series()
                }
            }
            ColumnBuilder::Double {
                values,
                validity,
                has_nulls,
            } => {
                if has_nulls {
                    Float64Chunked::from_vec_validity(name.into(), values, Some(validity.into()))
                        .into_series()
                } else {
                    Float64Chunked::from_vec(name.into(), values).into_series()
                }
            }
            ColumnBuilder::Decimal {
                values,
                validity,
                has_nulls,
                precision,
                scale,
            } => {
                let ca = if has_nulls {
                    Int128Chunked::from_vec_validity(name.into(), values, Some(validity.into()))
                } else {
                    Int128Chunked::from_vec(name.into(), values)
                };
                ca.into_decimal_unchecked(precision, scale).into_series()
            }
            ColumnBuilder::StrBuilder { builder, .. } => builder.finish().into_series(),
            ColumnBuilder::DateDays {
                values,
                validity,
                has_nulls,
            } => {
                let ca = if has_nulls {
                    Int32Chunked::from_vec_validity(name.into(), values, Some(validity.into()))
                } else {
                    Int32Chunked::from_vec(name.into(), values)
                };
                ca.into_date().into_series()
            }
            ColumnBuilder::TimeNs {
                values,
                validity,
                has_nulls,
            } => {
                let ca = if has_nulls {
                    Int64Chunked::from_vec_validity(name.into(), values, Some(validity.into()))
                } else {
                    Int64Chunked::from_vec(name.into(), values)
                };
                ca.into_time().into_series()
            }
            ColumnBuilder::DateTimeUs {
                values,
                validity,
                has_nulls,
            } => {
                let ca = if has_nulls {
                    Int64Chunked::from_vec_validity(name.into(), values, Some(validity.into()))
                } else {
                    Int64Chunked::from_vec(name.into(), values)
                };
                ca.into_datetime(TimeUnit::Microseconds, None).into_series()
            }
            ColumnBuilder::Blob(v) => {
                // Store as Binary
                let values: Vec<Option<&[u8]>> = v.iter().map(|opt| opt.as_deref()).collect();
                Series::new(name.into(), values)
            }
        };
        Ok(s)
    }
}

// ── Row-by-row reader ─────────────────────────────────────────────────

/// A row-by-row YXDB file reader with field value extraction.
///
/// This provides a cursor-style API for iterating records one at a time
/// and extracting typed field values. It avoids building columnar data
/// structures, making it suitable for streaming processing and for
/// benchmarking raw row-iteration speed.
///
/// # Example
///
/// ```no_run
/// use sigilyx::YxdbRowReader;
///
/// let mut reader = YxdbRowReader::open("data.yxdb").unwrap();
/// while reader.next().unwrap() {
///     let id = reader.read_index(0).unwrap();
///     let name = reader.read_name("Name").unwrap();
///     println!("{:?} {:?}", id, name);
/// }
/// ```
pub struct YxdbRowReader {
    inner: YxdbReader,
    record_buf: Vec<u8>,
    name_map: HashMap<String, usize>,
    has_current: bool,
}

impl YxdbRowReader {
    /// Open a YXDB file for row-by-row reading.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let inner = YxdbReader::open(path)?;
        let name_map: HashMap<String, usize> = inner
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| (f.name.clone(), i))
            .collect();
        let capacity = inner.fixed_size + 1024;
        Ok(YxdbRowReader {
            inner,
            record_buf: Vec::with_capacity(capacity),
            name_map,
            has_current: false,
        })
    }

    /// Advance to the next record. Returns `true` if a record is available.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<bool> {
        self.has_current = self.inner.next_record(&mut self.record_buf)?;
        Ok(self.has_current)
    }

    /// Read a field value by column index (0-based).
    pub fn read_index(&self, index: usize) -> Result<FieldValue> {
        if !self.has_current {
            return Err(YxdbError::ConversionError(
                "no current record -- call next() first".into(),
            ));
        }
        record::extract_field_index(&self.record_buf, &self.inner.fields, index)
    }

    /// Read a field value by column name.
    pub fn read_name(&self, name: &str) -> Result<FieldValue> {
        let index = self
            .name_map
            .get(name)
            .ok_or_else(|| YxdbError::ConversionError(format!("unknown field name: {}", name)))?;
        self.read_index(*index)
    }

    /// Read all field values from the current record as a Vec.
    ///
    /// This is more efficient than calling [`read_index`] in a loop when
    /// you need all values, especially across an FFI boundary.
    pub fn read_all(&self) -> Result<Vec<FieldValue>> {
        if !self.has_current {
            return Err(YxdbError::ConversionError(
                "no current record -- call next() first".into(),
            ));
        }
        record::extract_all_fields(&self.record_buf, &self.inner.fields)
    }

    /// Return the total number of records in the file (from header).
    pub fn num_records(&self) -> u64 {
        self.inner.header.num_records
    }

    /// Return field metadata.
    pub fn fields(&self) -> &[FieldMeta] {
        &self.inner.fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: create a DataFrame from columns, inferring height.
    fn test_df(columns: Vec<Column>) -> DataFrame {
        let h = columns.first().map_or(0, |c| c.len());
        DataFrame::new(h, columns).unwrap()
    }

    fn test_path(name: &str) -> String {
        format!("{}/test_files/{}", env!("CARGO_MANIFEST_DIR"), name)
    }

    // ── AllTypes.yxdb: 2 rows × 16 columns covering every field type ──

    #[test]
    fn all_types_shape() {
        let df =
            crate::read_yxdb(test_path("AllTypes.yxdb"), crate::SpatialMode::Raw, false).unwrap();
        assert_eq!(df.height(), 2);
        assert_eq!(df.width(), 16);
    }

    #[test]
    fn all_types_integer_values() {
        let df =
            crate::read_yxdb(test_path("AllTypes.yxdb"), crate::SpatialMode::Raw, false).unwrap();
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
        let df =
            crate::read_yxdb(test_path("AllTypes.yxdb"), crate::SpatialMode::Raw, false).unwrap();
        let col = df.column("BoolCol").unwrap().bool().unwrap();
        assert_eq!(col.get(0), Some(true));
        assert_eq!(col.get(1), Some(false));
    }

    #[test]
    fn all_types_float_values() {
        let df =
            crate::read_yxdb(test_path("AllTypes.yxdb"), crate::SpatialMode::Raw, false).unwrap();
        let f32_col = df.column("FloatCol").unwrap().f32().unwrap();
        assert!((f32_col.get(0).unwrap() - 2.5).abs() < 0.01);

        let f64_col = df.column("DoubleCol").unwrap().f64().unwrap();
        assert!((f64_col.get(0).unwrap() - std::f64::consts::PI).abs() < 1e-10);
        assert!((f64_col.get(1).unwrap() - 0.0).abs() < 1e-10);

        let dec_col = df.column("DecimalCol").unwrap();
        assert!(matches!(dec_col.dtype(), DataType::Decimal(_, _)));
        let dec_ca = dec_col.decimal().unwrap();
        // 1234.5678 with scale=4 → unscaled i128 = 12345678
        assert_eq!(dec_ca.phys.get(0), Some(12345678i128));
    }

    #[test]
    fn all_types_string_values() {
        let df =
            crate::read_yxdb(test_path("AllTypes.yxdb"), crate::SpatialMode::Raw, false).unwrap();
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
        let df =
            crate::read_yxdb(test_path("AllTypes.yxdb"), crate::SpatialMode::Raw, false).unwrap();

        // DateCol → Polars Date (days since epoch)
        let date_col = df.column("DateCol").unwrap().date().unwrap();
        // 2025-03-15 → days since 1970-01-01
        let expected_date = chrono_date_to_days(2025, 3, 15);
        assert_eq!(date_col.phys.get(0), Some(expected_date));

        // DateTimeCol → Polars Datetime (us since epoch)
        let dt_col = df.column("DateTimeCol").unwrap().datetime().unwrap();
        // 2025-03-15 08:30:00 → us since 1970-01-01
        let expected_dt = chrono_date_to_days(2025, 3, 15) as i64 * 86_400_000_000
            + 8 * 3_600_000_000
            + 30 * 60_000_000;
        assert_eq!(dt_col.phys.get(0), Some(expected_dt));

        // TimeCol → Polars Time (stored as i64 nanoseconds since midnight)
        let time_col = df.column("TimeCol").unwrap();
        assert_eq!(time_col.dtype(), &DataType::Time);
        assert!(!time_col.is_null().any());
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
        let df =
            crate::read_yxdb(test_path("AllTypes.yxdb"), crate::SpatialMode::Raw, false).unwrap();
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
        let df =
            crate::read_yxdb(test_path("NullValues.yxdb"), crate::SpatialMode::Raw, false).unwrap();
        assert_eq!(df.height(), 3);

        // Row 0 is fully populated
        let id_col = df.column("Id").unwrap().i32().unwrap();
        assert_eq!(id_col.get(0), Some(1));

        let str_col = df.column("NullStr").unwrap().str().unwrap();
        assert_eq!(str_col.get(0), Some("hello"));
    }

    #[test]
    fn null_values_all_null_row() {
        let df =
            crate::read_yxdb(test_path("NullValues.yxdb"), crate::SpatialMode::Raw, false).unwrap();

        // Row 1: all null except Id
        let id_col = df.column("Id").unwrap().i32().unwrap();
        assert_eq!(id_col.get(1), Some(2));

        // Check nulls via the typed chunked arrays
        assert!(df
            .column("NullByte")
            .unwrap()
            .i16()
            .unwrap()
            .get(1)
            .is_none());
        assert!(df
            .column("NullInt16")
            .unwrap()
            .i16()
            .unwrap()
            .get(1)
            .is_none());
        assert!(df
            .column("NullInt32")
            .unwrap()
            .i32()
            .unwrap()
            .get(1)
            .is_none());
        assert!(df
            .column("NullInt64")
            .unwrap()
            .i64()
            .unwrap()
            .get(1)
            .is_none());
        assert!(df
            .column("NullFloat")
            .unwrap()
            .f32()
            .unwrap()
            .get(1)
            .is_none());
        assert!(df
            .column("NullDouble")
            .unwrap()
            .f64()
            .unwrap()
            .get(1)
            .is_none());
        assert!(df
            .column("NullStr")
            .unwrap()
            .str()
            .unwrap()
            .get(1)
            .is_none());
        assert!(df
            .column("NullBlob")
            .unwrap()
            .binary()
            .unwrap()
            .get(1)
            .is_none());
    }

    #[test]
    fn null_values_mixed_row() {
        let df =
            crate::read_yxdb(test_path("NullValues.yxdb"), crate::SpatialMode::Raw, false).unwrap();

        // Row 2: mixed — NullByte is null, NullInt16 is 50
        assert!(df
            .column("NullByte")
            .unwrap()
            .i16()
            .unwrap()
            .get(2)
            .is_none());
        let i16_col = df.column("NullInt16").unwrap().i16().unwrap();
        assert_eq!(i16_col.get(2), Some(50));
        assert!(df
            .column("NullInt32")
            .unwrap()
            .i32()
            .unwrap()
            .get(2)
            .is_none());
    }

    // ── ManyRecords.yxdb: 50,000 rows for LZF block stress test ──

    #[test]
    fn many_records_shape() {
        let df = crate::read_yxdb(
            test_path("ManyRecords.yxdb"),
            crate::SpatialMode::Raw,
            false,
        )
        .unwrap();
        assert_eq!(df.height(), 50_000);
        assert_eq!(df.width(), 3);
    }

    #[test]
    fn many_records_id_sum() {
        let df = crate::read_yxdb(
            test_path("ManyRecords.yxdb"),
            crate::SpatialMode::Raw,
            false,
        )
        .unwrap();
        let id_col = df.column("Id").unwrap().i32().unwrap();
        let id_sum: i64 = id_col.into_iter().map(|v| v.unwrap_or(0) as i64).sum();
        // sum(1..=50000) = 50000 * 50001 / 2 = 1_250_025_000
        assert_eq!(id_sum, 1_250_025_000);
    }

    #[test]
    fn many_records_label_check() {
        let df = crate::read_yxdb(
            test_path("ManyRecords.yxdb"),
            crate::SpatialMode::Raw,
            false,
        )
        .unwrap();
        let label_col = df.column("Label").unwrap().str().unwrap();
        assert_eq!(label_col.get(0), Some("row_00001"));
        assert_eq!(label_col.get(49_999), Some("row_50000"));
    }

    // ── LargeBlob.yxdb: large binary data ──

    #[test]
    fn large_blob_sizes() {
        let df =
            crate::read_yxdb(test_path("LargeBlob.yxdb"), crate::SpatialMode::Raw, false).unwrap();
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
        let df =
            crate::read_yxdb(test_path("People.yxdb"), crate::SpatialMode::Raw, false).unwrap();
        assert_eq!(df.height(), 200);
        assert_eq!(df.width(), 8);
        assert!(df
            .get_column_names()
            .iter()
            .any(|n| n.as_str() == "FirstName"));
        assert!(df.get_column_names().iter().any(|n| n.as_str() == "Salary"));
    }

    #[test]
    fn people_no_null_ids() {
        let df =
            crate::read_yxdb(test_path("People.yxdb"), crate::SpatialMode::Raw, false).unwrap();
        assert_eq!(df.column("PersonId").unwrap().null_count(), 0);
    }

    // ── Strings.yxdb: string edge cases ──

    #[test]
    fn strings_edge_cases() {
        let df =
            crate::read_yxdb(test_path("Strings.yxdb"), crate::SpatialMode::Raw, false).unwrap();
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
        let df =
            crate::read_yxdb(test_path("Strings.yxdb"), crate::SpatialMode::Raw, false).unwrap();
        let vwstr = df.column("VarWStr").unwrap().str().unwrap();
        // Row 0: "wïdé" (unicode in wide string)
        assert_eq!(vwstr.get(0), Some("wïdé"));
        // Row 4: Japanese characters
        assert_eq!(vwstr.get(4), Some("日本語テスト"));
    }

    // ── SingleColumn.yxdb: simplest valid file ──

    #[test]
    fn single_column_values() {
        let df = crate::read_yxdb(
            test_path("SingleColumn.yxdb"),
            crate::SpatialMode::Raw,
            false,
        )
        .unwrap();
        assert_eq!(df.height(), 5);
        let col = df.column("Value").unwrap().i32().unwrap();
        assert_eq!(col.get(0), Some(10));
        assert_eq!(col.get(4), Some(50));
    }

    // ── Column projection tests ──

    #[test]
    fn projection_subset() {
        let reader = YxdbReader::open(test_path("AllTypes.yxdb")).unwrap();
        let df = reader
            .into_dataframe_projected(Some(&["BoolCol", "Int32Col"]))
            .unwrap();
        assert_eq!(df.width(), 2);
        assert_eq!(df.height(), 2);
        assert!(df.column("BoolCol").is_ok());
        assert!(df.column("Int32Col").is_ok());
        assert!(df.column("StringCol").is_err());
    }

    #[test]
    fn projection_none_returns_all() {
        let reader = YxdbReader::open(test_path("AllTypes.yxdb")).unwrap();
        let df = reader.into_dataframe_projected(None).unwrap();
        assert_eq!(df.width(), 16);
    }

    #[test]
    fn projection_rejects_unknown_columns() {
        let reader = YxdbReader::open(test_path("AllTypes.yxdb")).unwrap();
        let result = reader.into_dataframe_projected(Some(&["Int32Col", "NoSuchColumn"]));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("NoSuchColumn"),
            "Error should mention the unknown column name"
        );
    }

    #[test]
    fn projection_variable_records() {
        let reader = YxdbReader::open(test_path("Strings.yxdb")).unwrap();
        let df = reader.into_dataframe_projected(Some(&["VarStr"])).unwrap();
        assert_eq!(df.width(), 1);
        assert_eq!(df.height(), 6);
        let vstr = df.column("VarStr").unwrap().str().unwrap();
        assert_eq!(vstr.get(0), Some("variable"));
    }

    #[test]
    fn read_yxdb_columns_convenience() {
        let df = crate::read_yxdb_columns(
            test_path("People.yxdb"),
            &["PersonId", "FirstName"],
            crate::SpatialMode::Raw,
            false,
        )
        .unwrap();
        assert_eq!(df.width(), 2);
        assert_eq!(df.height(), 200);
    }

    // ── Error handling ──

    #[test]
    fn reject_invalid_text_file() {
        let result = YxdbReader::open(test_path("not_a_yxdb.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn reject_too_small_file() {
        let result = YxdbReader::open(test_path("too_small.bin"));
        assert!(result.is_err());
    }

    #[test]
    fn reject_nonexistent_file() {
        let result = YxdbReader::open(test_path("does_not_exist.yxdb"));
        assert!(result.is_err());
    }

    // ── YxdbRowReader tests ──

    #[test]
    fn row_reader_all_types() {
        let mut reader = YxdbRowReader::open(test_path("AllTypes.yxdb")).unwrap();
        assert_eq!(reader.num_records(), 2);
        assert_eq!(reader.fields().len(), 16);

        // Read row 0
        assert!(reader.next().unwrap());
        let vals = reader.read_all().unwrap();
        assert_eq!(vals.len(), 16);

        // Check values by name
        assert_eq!(
            reader.read_name("BoolCol").unwrap(),
            FieldValue::Bool(Some(true))
        );
        assert_eq!(
            reader.read_name("ByteCol").unwrap(),
            FieldValue::Byte(Some(7))
        );
        assert_eq!(
            reader.read_name("Int16Col").unwrap(),
            FieldValue::Int16(Some(-1234))
        );
        assert_eq!(
            reader.read_name("Int32Col").unwrap(),
            FieldValue::Int32(Some(42000))
        );
        assert_eq!(
            reader.read_name("Int64Col").unwrap(),
            FieldValue::Int64(Some(9_000_000_000))
        );

        // read_index should also work for the first field
        let first_by_name = reader.read_name(reader.fields()[0].name.as_str()).unwrap();
        let first_by_index = reader.read_index(0).unwrap();
        assert_eq!(first_by_name, first_by_index);

        // Read row 1
        assert!(reader.next().unwrap());
        assert_eq!(
            reader.read_name("BoolCol").unwrap(),
            FieldValue::Bool(Some(false))
        );
        assert_eq!(
            reader.read_name("ByteCol").unwrap(),
            FieldValue::Byte(Some(255))
        );

        // No more rows
        assert!(!reader.next().unwrap());
    }

    #[test]
    fn row_reader_name_lookup() {
        let mut reader = YxdbRowReader::open(test_path("AllTypes.yxdb")).unwrap();
        assert!(reader.next().unwrap());

        // read_name should return same values as read_index
        let by_name = reader.read_name("Int32Col").unwrap();
        let by_index = reader.read_index(3).unwrap();
        assert_eq!(by_name, by_index);
        assert_eq!(by_name, FieldValue::Int32(Some(42000)));

        // Unknown name should error
        assert!(reader.read_name("NonExistent").is_err());
    }

    #[test]
    fn row_reader_null_handling() {
        let mut reader = YxdbRowReader::open(test_path("NullValues.yxdb")).unwrap();
        assert_eq!(reader.num_records(), 3);

        // Row 0: fully populated
        assert!(reader.next().unwrap());
        let row0_id = reader.read_name("Id").unwrap();
        assert_eq!(row0_id, FieldValue::Int32(Some(1)));

        // Row 1: all null except Id
        assert!(reader.next().unwrap());
        let row1_id = reader.read_name("Id").unwrap();
        assert_eq!(row1_id, FieldValue::Int32(Some(2)));
        let row1_str = reader.read_name("NullStr").unwrap();
        assert_eq!(row1_str, FieldValue::String(None));
    }

    #[test]
    fn row_reader_many_records() {
        let mut reader = YxdbRowReader::open(test_path("ManyRecords.yxdb")).unwrap();
        assert_eq!(reader.num_records(), 50_000);

        let mut count = 0u64;
        while reader.next().unwrap() {
            count += 1;
            // Just iterate — don't extract values to test pure iteration speed
        }
        assert_eq!(count, 50_000);
    }

    #[test]
    fn row_reader_error_before_next() {
        let reader = YxdbRowReader::open(test_path("AllTypes.yxdb")).unwrap();
        // Attempting to read without calling next() should error
        assert!(reader.read_index(0).is_err());
        assert!(reader.read_all().is_err());
        assert!(reader.read_name("BoolCol").is_err());
    }

    // ── transcode_utf16le tests ──

    #[test]
    fn transcode_pure_ascii() {
        let input: Vec<u8> = "Hello, World!"
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, "Hello, World!");
    }

    #[test]
    fn transcode_latin_extended_u0100_range() {
        // U+0100 ("Ā") through U+017F — Latin Extended-A
        // These have high byte 0x01 and low byte < 0x80, which previously
        // fooled the SIMD path into treating them as ASCII.
        let text = "ĀĂĄĆĈĊČ";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_eight_consecutive_u0100() {
        // Exactly 8 code units of U+0100 — triggers SIMD batch on x86_64
        let text = "ĀĀĀĀĀĀĀĀ";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        assert_eq!(input.len(), 16); // 8 code units × 2 bytes
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_mixed_ascii_and_u0100_range() {
        // Mix of ASCII and Latin Extended that spans SIMD boundaries
        let text = "AĀBĂCĄDĆEĈFĊGČHā";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_cyrillic() {
        // Cyrillic (U+0400-U+04FF): high byte in [0x04], low byte varies
        let text = "Привет мир";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_greek() {
        // Greek (U+0370-U+03FF): both bytes < 0x80 for some chars
        let text = "αβγδεζηθ";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_cjk() {
        // CJK Unified Ideographs (U+4E00-U+9FFF): high byte >= 0x80
        let text = "日本語テスト";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_surrogate_pairs() {
        // U+1F600 (😀) requires a surrogate pair in UTF-16
        let text = "A😀B";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_empty() {
        let mut out = String::new();
        transcode_utf16le(&[], &mut out);
        assert_eq!(out, "");
    }

    #[test]
    fn transcode_single_char() {
        let input = 0x0041u16.to_le_bytes(); // 'A'
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, "A");
    }

    #[test]
    fn transcode_long_ascii_then_nonascii() {
        // 16 ASCII chars (2 full SIMD batches) then non-ASCII
        let text = "0123456789ABCDEFÜnïcödé";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_reuse_buffer() {
        let mut out = String::new();
        // First call
        let input1: Vec<u8> = "Hello"
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        transcode_utf16le(&input1, &mut out);
        assert_eq!(out, "Hello");
        // Second call should clear and overwrite
        let input2: Vec<u8> = "World"
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        transcode_utf16le(&input2, &mut out);
        assert_eq!(out, "World");
    }

    // ── Additional SIMD transcoding edge cases ──────────────────────

    #[test]
    fn transcode_exactly_8_ascii() {
        // 8 code units = exactly one SIMD batch on SSE2
        let text = "ABCDEFGH";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        assert_eq!(input.len(), 16);
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_exactly_16_ascii() {
        // 16 code units = two full SIMD batches
        let text = "0123456789ABCDEF";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        assert_eq!(input.len(), 32);
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_7_ascii_then_nonascii() {
        // 7 ASCII (< SIMD batch of 8) + non-ASCII
        let text = "1234567Ü";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_9_ascii_then_nonascii() {
        // 9 ASCII (1 SIMD batch + 1 scalar) + non-ASCII
        let text = "123456789é";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_bmp_boundary() {
        // U+FFFD (replacement character) — highest non-surrogate BMP codepoint
        let text = "\u{FFFD}A\u{FFFD}";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_multi_surrogate_pairs() {
        // Multiple supplementary plane characters in a row
        let text = "𝄞𝄞𝄞𝄞"; // U+1D11E (Musical Symbol G Clef), 4 times
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_null_codepoints() {
        // Embedded U+0000 — should be preserved
        let input: Vec<u8> = vec![
            0x41, 0x00, // 'A'
            0x00, 0x00, // U+0000
            0x42, 0x00, // 'B'
        ];
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, "A\0B");
    }

    #[test]
    fn transcode_latin_ext_b() {
        // U+0180-U+024F — Latin Extended-B
        let text = "ƀƁƂƃƄƅƆƇ";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_arabic() {
        let text = "مرحبا بالعالم";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_thai() {
        let text = "สวัสดีครับ";
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_very_long_ascii() {
        // 1000 ASCII chars — many SIMD batches
        let text: String = (0..1000)
            .map(|i| char::from(b'A' + (i % 26) as u8))
            .collect();
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn transcode_very_long_nonascii() {
        // 500 CJK characters — all go through non-ASCII path
        let text: String = (0..500).map(|_| '日').collect();
        let input: Vec<u8> = text
            .encode_utf16()
            .flat_map(|cu| cu.to_le_bytes())
            .collect();
        let mut out = String::new();
        transcode_utf16le(&input, &mut out);
        assert_eq!(out, text);
    }

    // ── Column reader / projection edge cases ────────────────────────

    #[test]
    fn projection_empty_list_returns_no_columns() {
        let reader = YxdbReader::open(test_path("AllTypes.yxdb")).unwrap();
        let df = reader.into_dataframe_projected(Some(&[])).unwrap();
        assert_eq!(df.width(), 0);
    }

    #[test]
    fn projection_duplicate_column_names() {
        // Polars rejects DataFrames with duplicate column names.
        // When a projection lists the same column twice, the reader should
        // either deduplicate or propagate the error from Polars.
        let reader = YxdbReader::open(test_path("AllTypes.yxdb")).unwrap();
        let result = reader.into_dataframe_projected(Some(&["Int32Col", "Int32Col"]));
        // Accept either: Ok with 1 column (dedup) or Err (Polars duplicate rejection)
        match result {
            Ok(df) => {
                assert_eq!(df.width(), 1);
                assert_eq!(df.height(), 2);
            }
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("duplicate"), "unexpected error: {msg}");
            }
        }
    }

    // ── Row reader edge cases ────────────────────────────────────────

    #[test]
    fn row_reader_read_index_all_fields() {
        let mut reader = YxdbRowReader::open(test_path("AllTypes.yxdb")).unwrap();
        assert!(reader.next().unwrap());
        // Read every field by index — should not panic
        for i in 0..reader.fields().len() {
            let _ = reader.read_index(i).unwrap();
        }
    }

    #[test]
    fn row_reader_out_of_bounds_index() {
        let mut reader = YxdbRowReader::open(test_path("AllTypes.yxdb")).unwrap();
        assert!(reader.next().unwrap());
        let result = reader.read_index(9999);
        assert!(result.is_err());
    }

    #[test]
    fn row_reader_single_column_file() {
        let mut reader = YxdbRowReader::open(test_path("SingleColumn.yxdb")).unwrap();
        assert_eq!(reader.num_records(), 5);
        let mut sum = 0i64;
        while reader.next().unwrap() {
            if let FieldValue::Int32(Some(v)) = reader.read_index(0).unwrap() {
                sum += v as i64;
            }
        }
        assert_eq!(sum, 150); // 10+20+30+40+50
    }

    #[test]
    fn row_reader_large_blob() {
        let mut reader = YxdbRowReader::open(test_path("LargeBlob.yxdb")).unwrap();
        assert!(reader.next().unwrap());
        let val = reader.read_name("Data").unwrap();
        match val {
            FieldValue::Blob(Some(data)) => assert_eq!(data.len(), 512_000),
            other => panic!("expected large blob, got {other:?}"),
        }
    }

    // ══════════════════════════════════════════════════════════════════
    // Regression tests — audit findings (v0.1.1)
    // ══════════════════════════════════════════════════════════════════

    /// Audit #6 — File smaller than 512 bytes produces a clear error message.
    #[test]
    fn regression_small_file_error() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"too short").unwrap();
        tmp.flush().unwrap();

        let err = crate::read_yxdb(tmp.path(), crate::SpatialMode::Raw, false).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("too small") || msg.contains("512"),
            "expected descriptive error for tiny file, got: {msg}"
        );
    }

    /// Audit #11 — Projecting an unknown column name returns an error.
    #[test]
    fn regression_unknown_column_rejected() {
        let reader = YxdbReader::open(test_path("AllTypes.yxdb")).unwrap();
        let result = reader.into_dataframe_projected(Some(&["Int32Col", "DoesNotExist"]));
        assert!(result.is_err(), "expected error for unknown column");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("DoesNotExist"),
            "error should mention the missing column name, got: {msg}"
        );
    }

    /// Audit #11 (companion) — All-unknown columns also error.
    #[test]
    fn regression_all_unknown_columns_rejected() {
        let reader = YxdbReader::open(test_path("AllTypes.yxdb")).unwrap();
        let result = reader.into_dataframe_projected(Some(&["Fake1", "Fake2"]));
        assert!(result.is_err());
    }

    /// Audit #7 — Empty DataFrame (0 rows) reads back with correct schema.
    #[test]
    fn regression_empty_dataframe_schema_preserved() {
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let empty_i32: Vec<i32> = vec![];
        let empty_str: Vec<&str> = vec![];
        let df = df! {
            "id" => empty_i32,
            "name" => empty_str
        }
        .unwrap();

        let tmp = NamedTempFile::new().unwrap();
        crate::write_yxdb(tmp.path(), &df, &[]).unwrap();

        // Full read
        let df2 = crate::read_yxdb(tmp.path(), crate::SpatialMode::Raw, false).unwrap();
        assert_eq!(df2.height(), 0);
        assert_eq!(df2.width(), 2);

        // Projected read
        let reader = YxdbReader::open(tmp.path()).unwrap();
        let df3 = reader.into_dataframe_projected(Some(&["name"])).unwrap();
        assert_eq!(df3.height(), 0);
        assert_eq!(df3.width(), 1);
        assert_eq!(df3.get_column_names()[0].as_str(), "name");
    }

    /// Audit #5 — Large binary blob (> BLOCK_SIZE) roundtrips through reader.
    /// Ensures the dynamic LZF buffer sizing doesn't choke on oversized
    /// uncompressed blocks.
    #[test]
    fn regression_large_blob_decompression() {
        use polars::prelude::*;
        use tempfile::NamedTempFile;

        let big_blob: Vec<u8> = vec![0xAB; 300_000];
        let df = test_df(vec![Column::new(
            "payload".into(),
            vec![big_blob.as_slice()],
        )]);

        let tmp = NamedTempFile::new().unwrap();
        crate::write_yxdb(tmp.path(), &df, &[]).unwrap();

        let df2 = crate::read_yxdb(tmp.path(), crate::SpatialMode::Raw, false).unwrap();
        let col = df2.column("payload").unwrap().binary().unwrap();
        assert_eq!(col.get(0).unwrap().len(), 300_000);
        assert!(col.get(0).unwrap().iter().all(|&b| b == 0xAB));
    }

    /// Audit #11 — Projected read via read_yxdb_columns also rejects unknowns.
    #[test]
    fn regression_read_yxdb_columns_rejects_unknown() {
        let result = crate::read_yxdb_columns(
            test_path("AllTypes.yxdb"),
            &["Int32Col", "Nonexistent"],
            crate::SpatialMode::Raw,
            false,
        );
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("Nonexistent"),
            "error should mention the unknown column, got: {msg}"
        );
    }
}
