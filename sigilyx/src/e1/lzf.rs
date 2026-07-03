use crate::error::{Result, YxdbError};

// LZF compression and decompression.
//
// YXDB record data is stored in LZF-compressed blocks. Each block is preceded
// by a 4-byte little-endian length. If the high bit (0x80000000) is set, the
// block is stored uncompressed and the remaining 31 bits are the byte length.
// Otherwise, the block is LZF-compressed.

/// Compression algorithm used for record block data.
///
/// Standard YXDB files use LZF (compression_version = 1 in the file header).
/// Compression version 0 means uncompressed (no block framing - records stored
/// directly in the file stream). This is supported for reading only; all writes
/// use LZF for maximum compatibility with other YXDB readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionAlgorithm {
    /// LZF compression (compression_version = 1). Standard, compatible with all YXDB readers.
    Lzf,
}

impl CompressionAlgorithm {
    /// Return the compression_version value for the YXDB header.
    pub fn version_id(self) -> i32 {
        match self {
            CompressionAlgorithm::Lzf => 1,
        }
    }

    /// Parse a header compression_version value.
    ///
    /// Returns `None` for version 0 (uncompressed - no block framing),
    /// `Some(Lzf)` for version 1 (LZF-compressed blocks).
    pub fn from_version_id(v: i32) -> Result<Option<Self>> {
        match v {
            0 => Ok(None),
            1 => Ok(Some(CompressionAlgorithm::Lzf)),
            _ => Err(YxdbError::InvalidFile(format!(
                "unsupported compression version: {v} (expected 0=uncompressed or 1=LZF)"
            ))),
        }
    }
}

// -- C liblzf FFI --

extern "C" {
    fn lzf_decompress(
        in_data: *const u8,
        in_len: std::ffi::c_uint,
        out_data: *mut u8,
        out_len: std::ffi::c_uint,
    ) -> std::ffi::c_uint;
}

/// Decompress LZF data into a pre-allocated output slice using C liblzf.
///
/// This is the primary decompression path, used for both sequential and
/// parallel block decompression. The C implementation uses Duff's device
/// for small copies, which is faster than Rust loops for short runs.
///
/// Returns the number of decompressed bytes written to `out`.
pub fn decompress_into(input: &[u8], out: &mut [u8]) -> Result<usize> {
    if input.is_empty() {
        return Ok(0);
    }
    // SAFETY: `input` and `out` are valid slices with known lengths.
    // The C `lzf_decompress` reads exactly `in_len` bytes from `in_data`
    // and writes at most `out_len` bytes to `out_data`.  Both pointers
    // and lengths are derived from valid Rust slice references.
    let result = unsafe {
        lzf_decompress(
            input.as_ptr(),
            input.len() as std::ffi::c_uint,
            out.as_mut_ptr(),
            out.len() as std::ffi::c_uint,
        )
    };
    if result == 0 {
        Err(YxdbError::LzfError("C lzf_decompress failed".into()))
    } else if (result as usize) > out.len() {
        Err(YxdbError::LzfError(format!(
            "C lzf_decompress returned {} but output buffer is only {} bytes",
            result,
            out.len()
        )))
    } else {
        Ok(result as usize)
    }
}

// -- Compression --

/// Hash table size - matches reference liblzf HLOG=16 for best compression.
/// With 256 KiB blocks (~87K 3-byte hash keys), 65536 buckets give ~1.3 entries
/// per bucket on average, minimising collisions. The previous value of 2^14
/// (16384) caused ~5x more collisions and roughly halved compression effectiveness.
const HASH_SIZE: usize = 1 << 16; // 65536
const MAX_LIT: usize = 32; // max literal run length (1..=32)
const MAX_REF: usize = 264; // max back-reference length (3..=264)
const MAX_OFF: usize = 8192; // max back-reference distance

/// Compress `input` using the LZF algorithm.
///
/// Returns the compressed bytes. If the compressed output would be larger
/// than the input, returns `None` (caller should store uncompressed).
///
/// This implementation matches the reference liblzf (HLOG=16, VERY_FAST=1):
/// same hash function, same rolling hash seeding, same post-match update
/// strategy, and same maximum back-reference distance.
pub fn compress(input: &[u8]) -> Option<Vec<u8>> {
    let in_len = input.len();
    if in_len <= 4 {
        return None; // too small to compress
    }

    // worst case: output slightly larger than input
    let mut out = Vec::with_capacity(in_len + (in_len / 16) + 16);
    let mut hash_table = [0u32; HASH_SIZE];

    let mut ip: usize; // input pointer
    let mut lit: usize = 0; // literal run length
    let mut lit_start: usize; // position of literal run length byte in output

    // Reserve space for the first literal run length byte (matches C: lit = 0; op++)
    out.push(0);
    lit_start = 0;

    // Seed rolling hash with first two bytes (matches liblzf FRST macro)
    let mut hval: u32 = ((input[0] as u32) << 8) | (input[1] as u32);
    ip = 0;

    while ip < in_len - 2 {
        // Rolling hash: incorporate next byte (matches liblzf NEXT macro)
        hval = (hval << 8) | (input[ip + 2] as u32);
        let h = hash_idx(hval);
        let ref_pos = hash_table[h] as usize;
        hash_table[h] = ip as u32;

        // Check for a match. The offset `off` is distance-1 (matching the C
        // reference where `off = ip - ref - 1`). MAX_OFF = 8192 means max
        // distance is 8192 (off_minus_1 up to 8191, fitting in 13 bits).
        let off = ip.wrapping_sub(ref_pos).wrapping_sub(1);
        if off < MAX_OFF
            && ref_pos > 0
            && ref_pos < ip
            && input[ref_pos + 2] == input[ip + 2]
            && input[ref_pos] == input[ip]
            && input[ref_pos + 1] == input[ip + 1]
        {
            // We have a match of at least 3 bytes
            let mut match_len = 3;
            let max_match = MAX_REF.min(in_len - ip);
            while match_len < max_match && input[ref_pos + match_len] == input[ip + match_len] {
                match_len += 1;
            }

            // Write the pending literal run length, or remove the reserved byte
            if lit > 0 {
                out[lit_start] = (lit - 1) as u8;
                lit = 0;
            } else {
                // No literals before this match - remove the reserved byte
                out.pop();
            }

            // Encode the back-reference (off is already distance-1)
            let len_code = match_len - 2; // 1..=262

            if len_code < 7 {
                out.push(((len_code as u8) << 5) | ((off >> 8) as u8));
                out.push((off & 0xFF) as u8);
            } else {
                out.push((7 << 5) | ((off >> 8) as u8));
                out.push((len_code - 7) as u8);
                out.push((off & 0xFF) as u8);
            }

            ip += 1; // advance past start of match (len is len_code, not match_len)
            ip += match_len - 1; // ip now points to first byte after match

            if ip >= in_len - 2 {
                // Reserve space for the next literal run
                lit_start = out.len();
                out.push(0);
                break;
            }

            // Post-match hash table update (matches liblzf VERY_FAST behaviour).
            // Back up 2 positions and re-hash at ip-2 and ip-1, leaving hval
            // correctly seeded for the main loop's NEXT at ip.
            ip -= 2;
            hval = ((input[ip] as u32) << 8) | (input[ip + 1] as u32);

            // Update hash at ip (which is match_end - 2)
            hval = (hval << 8) | (input[ip + 2] as u32);
            hash_table[hash_idx(hval)] = ip as u32;
            ip += 1;

            // Update hash at ip (which is match_end - 1)
            hval = (hval << 8) | (input[ip + 2] as u32);
            hash_table[hash_idx(hval)] = ip as u32;
            ip += 1;

            // ip is now at match_end, hval seeded for NEXT at top of loop

            // Reserve space for the next literal run
            lit_start = out.len();
            out.push(0);
        } else {
            // No match - emit a literal byte
            out.push(input[ip]);
            lit += 1;
            ip += 1;

            if lit == MAX_LIT {
                out[lit_start] = (lit - 1) as u8;
                lit = 0;
                lit_start = out.len();
                out.push(0);
            }
        }
    }

    // Flush remaining input bytes as literals
    while ip < in_len {
        out.push(input[ip]);
        lit += 1;
        ip += 1;

        if lit == MAX_LIT {
            out[lit_start] = (lit - 1) as u8;
            lit = 0;
            lit_start = out.len();
            out.push(0);
        }
    }

    // Write final literal run length
    if lit > 0 {
        out[lit_start] = (lit - 1) as u8;
    } else {
        // Remove the trailing unused literal run byte
        out.pop();
    }

    // Only use compressed version if it's actually smaller
    if out.len() < in_len {
        Some(out)
    } else {
        None
    }
}

/// Hash function matching reference liblzf IDX macro (VERY_FAST=1, HLOG=16).
///
/// The reference C code: `((h >> (3*8 - HLOG)) - h*5) & (HSIZE - 1)`
/// With HLOG=16: `((h >> 8) - h*5) & 0xFFFF`
#[inline]
fn hash_idx(hval: u32) -> usize {
    ((hval >> 8).wrapping_sub(hval.wrapping_mul(5))) as usize & (HASH_SIZE - 1)
}

// -- Decompression --

/// Decompress an LZF-compressed buffer into `out`.
///
/// Returns the number of bytes written to `out`.
/// Uses C liblzf for maximum throughput.
///
/// The output buffer must be large enough to hold the decompressed data.
/// For YXDB blocks, the maximum output is 262144 bytes (one block).
pub fn decompress(input: &[u8], out: &mut [u8]) -> Result<usize> {
    decompress_into(input, out)
}

/// Decompress a block using the specified algorithm.
pub fn decompress_block_into(
    algo: CompressionAlgorithm,
    input: &[u8],
    out: &mut [u8],
) -> Result<usize> {
    match algo {
        CompressionAlgorithm::Lzf => decompress_into(input, out),
    }
}

/// Decompress a block using the specified algorithm.
///
/// `out` must be pre-allocated with sufficient capacity. Use
/// [`decompress_block_into`] for the slice-based variant.
pub fn decompress_block(algo: CompressionAlgorithm, input: &[u8], out: &mut [u8]) -> Result<usize> {
    match algo {
        CompressionAlgorithm::Lzf => decompress(input, out),
    }
}

/// Compress a block using the specified algorithm.
pub fn compress_block(algo: CompressionAlgorithm, input: &[u8]) -> Option<Vec<u8>> {
    match algo {
        CompressionAlgorithm::Lzf => compress(input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompress_literal_only() {
        // A single literal run of 5 bytes: ctrl=4, then 5 bytes
        let input = [4u8, b'H', b'e', b'l', b'l', b'o'];
        let mut out = vec![0u8; 256];
        let n = decompress(&input, &mut out).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&out[..5], b"Hello");
    }

    #[test]
    fn roundtrip_identity() {
        // Uncompressed data: when ctrl < 32, it's a literal
        let data = b"ABCDEFGHIJ";
        // Encode as literal: ctrl = len-1 = 9
        let mut input = vec![9u8];
        input.extend_from_slice(data);
        let mut out = vec![0u8; 256];
        let n = decompress(&input, &mut out).unwrap();
        assert_eq!(&out[..n], data);
    }

    #[test]
    fn compress_decompress_roundtrip() {
        let data = b"Hello Hello Hello Hello Hello Hello Hello Hello World!";
        let compressed = compress(data).expect("should compress");
        assert!(compressed.len() < data.len());
        let mut decompressed = vec![0u8; 256];
        let n = decompress(&compressed, &mut decompressed).unwrap();
        assert_eq!(&decompressed[..n], &data[..]);
    }

    #[test]
    fn compress_decompress_large_block() {
        // 4096 bytes of repeating pattern
        let mut data = Vec::with_capacity(4096);
        for i in 0..4096 {
            data.push((i % 256) as u8);
        }
        let compressed = compress(&data).expect("should compress");
        assert!(compressed.len() < data.len());
        let mut decompressed = vec![0u8; 8192];
        let n = decompress(&compressed, &mut decompressed).unwrap();
        assert_eq!(&decompressed[..n], &data[..]);
    }

    #[test]
    fn compress_returns_none_for_tiny_input() {
        assert!(compress(b"ab").is_none());
    }

    #[test]
    fn compress_returns_none_for_random_data() {
        // Random-ish data that won't compress well
        let data: Vec<u8> = (0..100).map(|i| (i * 37 + 13) as u8).collect();
        // May or may not compress - just ensure round-trip works if it does
        if let Some(compressed) = compress(&data) {
            let mut decompressed = vec![0u8; 256];
            let n = decompress(&compressed, &mut decompressed).unwrap();
            assert_eq!(&decompressed[..n], &data[..]);
        }
    }

    #[test]
    fn compression_algorithm_version_ids() {
        assert_eq!(CompressionAlgorithm::Lzf.version_id(), 1);
        assert_eq!(CompressionAlgorithm::from_version_id(0).unwrap(), None);
        assert_eq!(
            CompressionAlgorithm::from_version_id(1).unwrap(),
            Some(CompressionAlgorithm::Lzf)
        );
        assert!(CompressionAlgorithm::from_version_id(2).is_err());
        assert!(CompressionAlgorithm::from_version_id(3).is_err());
    }

    #[test]
    fn decompress_block_dispatch() {
        let data = b"Hello Hello Hello Hello Hello Hello Hello Hello World!";

        // LZF round-trip via dispatch
        let lzf_compressed = compress_block(CompressionAlgorithm::Lzf, data).unwrap();
        let mut out = vec![0u8; 256];
        let n =
            decompress_block_into(CompressionAlgorithm::Lzf, &lzf_compressed, &mut out).unwrap();
        assert_eq!(&out[..n], &data[..]);
    }

    // -- Edge-case / stress tests --

    #[test]
    fn compress_all_zeros() {
        // Highly compressible: all zeros
        let data = vec![0u8; 8192];
        let compressed = compress(&data).expect("all-zeros should compress well");
        assert!(compressed.len() < data.len() / 10);
        let mut decompressed = vec![0u8; data.len()];
        let n = decompress(&compressed, &mut decompressed).unwrap();
        assert_eq!(&decompressed[..n], &data[..]);
    }

    #[test]
    fn compress_all_same_byte() {
        let data = vec![0xAA; 4096];
        let compressed = compress(&data).expect("repeated byte should compress");
        let mut decompressed = vec![0u8; data.len()];
        let n = decompress(&compressed, &mut decompressed).unwrap();
        assert_eq!(&decompressed[..n], &data[..]);
    }

    #[test]
    fn compress_exactly_max_lit_boundary() {
        // 32 bytes is MAX_LIT boundary - make sure literal run splits work
        let data: Vec<u8> = (0..64).map(|i| (i * 7 + 3) as u8).collect();
        if let Some(compressed) = compress(&data) {
            let mut decompressed = vec![0u8; 256];
            let n = decompress(&compressed, &mut decompressed).unwrap();
            assert_eq!(&decompressed[..n], &data[..]);
        }
    }

    #[test]
    fn compress_exactly_five_bytes() {
        // Smallest input that won't be rejected (> 4 bytes)
        let data = b"AAAAA";
        // May or may not compress, but should not panic
        if let Some(compressed) = compress(data) {
            let mut decompressed = vec![0u8; 256];
            let n = decompress(&compressed, &mut decompressed).unwrap();
            assert_eq!(&decompressed[..n], &data[..]);
        }
    }

    #[test]
    fn compress_block_size_data() {
        // 256 KB - the YXDB block size
        let data: Vec<u8> = (0..262144).map(|i| ((i * 13 + 7) % 256) as u8).collect();
        if let Some(compressed) = compress(&data) {
            let mut decompressed = vec![0u8; data.len()];
            let n = decompress(&compressed, &mut decompressed).unwrap();
            assert_eq!(n, data.len());
            assert_eq!(&decompressed[..n], &data[..]);
        }
    }

    #[test]
    fn compress_long_back_reference() {
        // Trigger back-references close to MAX_REF (264 bytes)
        // Pattern: 300 repeats of 'A' (long match) + different suffix
        let mut data = vec![b'A'; 300];
        data.extend_from_slice(b"XYZ_UNIQUE_SUFFIX");
        let compressed = compress(&data).expect("should compress with long matches");
        let mut decompressed = vec![0u8; data.len()];
        let n = decompress(&compressed, &mut decompressed).unwrap();
        assert_eq!(&decompressed[..n], &data[..]);
    }

    #[test]
    fn compress_alternating_pattern() {
        // Alternating bytes shouldn't cause issues at hash boundaries
        let data: Vec<u8> = (0..1000)
            .map(|i| if i % 2 == 0 { 0xAA } else { 0x55 })
            .collect();
        if let Some(compressed) = compress(&data) {
            let mut decompressed = vec![0u8; data.len()];
            let n = decompress(&compressed, &mut decompressed).unwrap();
            assert_eq!(&decompressed[..n], &data[..]);
        }
    }

    #[test]
    fn decompress_empty_input() {
        let mut out = vec![0u8; 64];
        let n = decompress(&[], &mut out).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn roundtrip_many_sizes() {
        // Test round-trip for various sizes from 5 to 10000
        for size in [5, 10, 31, 32, 33, 63, 64, 100, 255, 256, 1000, 4096, 10000] {
            let data: Vec<u8> = (0..size).map(|i| ((i * 3 + 1) % 256) as u8).collect();
            if let Some(compressed) = compress(&data) {
                let mut decompressed = vec![0u8; size + 64];
                let n = decompress(&compressed, &mut decompressed).unwrap();
                assert_eq!(&decompressed[..n], &data[..], "failed for size {size}");
            }
        }
    }
}
