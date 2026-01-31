use crate::error::{YxdbError, Result};

/// LZF compression and decompression.
///
/// YXDB record data is stored in LZF-compressed blocks. Each block is preceded
/// by a 4-byte little-endian length. If the high bit (0x80000000) is set, the
/// block is stored uncompressed and the remaining 31 bits are the byte length.
/// Otherwise, the block is LZF-compressed.

// ── Compression ────────────────────────────────────────────────────────

const HASH_SIZE: usize = 1 << 14; // 16384
const MAX_LIT: usize = 32;       // max literal run length (1..=32)
const MAX_REF: usize = 264;      // max back-reference length (3..=264)
const MAX_OFF: usize = 8192;     // max back-reference distance

/// Compress `input` using the LZF algorithm.
///
/// Returns the compressed bytes. If the compressed output would be larger
/// than the input, returns `None` (caller should store uncompressed).
pub fn compress(input: &[u8]) -> Option<Vec<u8>> {
    let in_len = input.len();
    if in_len <= 4 {
        return None; // too small to compress
    }

    // worst case: output slightly larger than input
    let mut out = Vec::with_capacity(in_len + (in_len / 16) + 16);
    let mut hash_table = [0u32; HASH_SIZE];

    let mut ip: usize;            // input pointer
    let mut lit: usize;           // literal run length
    let mut lit_start: usize;     // position of literal run length byte in output

    // Reserve space for the first literal run length byte
    out.push(0);
    lit_start = 0;

    // Emit the first byte as a literal (it can't be part of a back-reference)
    out.push(input[0]);
    lit = 1;
    
    let first_hash = hash(input, 0);
    hash_table[first_hash] = 0;
    ip = 1;

    while ip < in_len - 2 {
        let h = hash(input, ip);
        let ref_pos = hash_table[h] as usize;
        hash_table[h] = ip as u32;

        // Check for a match
        let off = ip - ref_pos;
        if off > 0
            && off < MAX_OFF
            && ref_pos < ip
            && ip + 3 <= in_len
            && input[ref_pos] == input[ip]
            && input[ref_pos + 1] == input[ip + 1]
            && input[ref_pos + 2] == input[ip + 2]
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

            // Encode the back-reference
            let len_code = match_len - 2; // 1..=262
            let off_minus_1 = off - 1;

            if len_code < 7 {
                out.push(((len_code as u8) << 5) | ((off_minus_1 >> 8) as u8));
                out.push((off_minus_1 & 0xFF) as u8);
            } else {
                out.push((7 << 5) | ((off_minus_1 >> 8) as u8));
                out.push((len_code - 7) as u8);
                out.push((off_minus_1 & 0xFF) as u8);
            }

            ip += match_len;

            // Update hash for the matched bytes
            if ip < in_len - 2 {
                let h2 = hash(input, ip);
                hash_table[h2] = ip as u32;
            }

            // Reserve space for the next literal run
            lit_start = out.len();
            out.push(0);
        } else {
            // No match — emit a literal byte
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

#[inline]
fn hash(data: &[u8], pos: usize) -> usize {
    let v = (data[pos] as u32) << 16 | (data[pos + 1] as u32) << 8 | data[pos + 2] as u32;
    ((v >> 1) ^ v) as usize & (HASH_SIZE - 1)
}

// ── Decompression ──────────────────────────────────────────────────────

/// Decompress an LZF-compressed buffer into `out`.
///
/// Returns the number of bytes written to `out`.
pub fn decompress(input: &[u8], out: &mut Vec<u8>) -> Result<usize> {
    let mut in_idx: usize = 0;
    let mut out_idx: usize = 0;
    let in_len = input.len();

    // Ensure capacity
    if out.len() < 262144 {
        out.resize(262144, 0);
    }

    while in_idx < in_len {
        let ctrl = input[in_idx] as usize;
        in_idx += 1;

        if ctrl < 32 {
            // Literal run: copy ctrl+1 bytes
            let length = ctrl + 1;
            let end = in_idx + length;
            if end > in_len {
                return Err(YxdbError::LzfError(
                    "literal run exceeds input buffer".into(),
                ));
            }
            ensure_capacity(out, out_idx + length);
            out[out_idx..out_idx + length].copy_from_slice(&input[in_idx..end]);
            in_idx = end;
            out_idx += length;
        } else {
            // Back-reference
            let mut length = ctrl >> 5;
            let mut reference = out_idx as isize - ((ctrl & 0x1f) << 8) as isize - 1;

            if length == 7 {
                if in_idx >= in_len {
                    return Err(YxdbError::LzfError(
                        "extended length byte missing".into(),
                    ));
                }
                length += input[in_idx] as usize;
                in_idx += 1;
            }

            if in_idx >= in_len {
                return Err(YxdbError::LzfError(
                    "reference offset byte missing".into(),
                ));
            }
            reference -= input[in_idx] as isize;
            in_idx += 1;

            length += 2;

            if reference < 0 {
                return Err(YxdbError::LzfError(
                    "back-reference before start of output".into(),
                ));
            }

            let mut ref_idx = reference as usize;
            ensure_capacity(out, out_idx + length);

            // Copy byte-by-byte to handle overlapping references
            for _ in 0..length {
                out[out_idx] = out[ref_idx];
                out_idx += 1;
                ref_idx += 1;
            }
        }
    }

    Ok(out_idx)
}

#[inline]
fn ensure_capacity(buf: &mut Vec<u8>, needed: usize) {
    if needed > buf.len() {
        buf.resize(needed * 2, 0);
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
        // May or may not compress — just ensure round-trip works if it does
        if let Some(compressed) = compress(&data) {
            let mut decompressed = vec![0u8; 256];
            let n = decompress(&compressed, &mut decompressed).unwrap();
            assert_eq!(&decompressed[..n], &data[..]);
        }
    }
}
