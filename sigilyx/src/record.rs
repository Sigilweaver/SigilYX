use crate::error::Result;
use crate::field::{FieldMeta, FieldType};

/// Value extracted from a single field in a record.
///
/// Each variant wraps the Rust-native type. `None` means the field is null.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Bool(Option<bool>),
    Byte(Option<u8>),
    Int16(Option<i16>),
    Int32(Option<i32>),
    Int64(Option<i64>),
    Float(Option<f32>),
    Double(Option<f64>),
    String(Option<String>),
    Date(Option<String>),      // "YYYY-MM-DD"
    Time(Option<String>),      // "HH:MM:SS"
    DateTime(Option<String>),  // "YYYY-MM-DD HH:MM:SS"
    Blob(Option<Vec<u8>>),
}

/// Extract a field's value from a record buffer.
///
/// `record` is the full record buffer (fixed portion + variable portion).
/// `field` contains the offset and type information.
pub fn extract_field(record: &[u8], field: &FieldMeta) -> Result<FieldValue> {
    let off = field.offset;

    match field.field_type {
        FieldType::Bool => {
            let v = record[off];
            // 2 = null, 1 = true, 0 = false
            if v == 2 {
                Ok(FieldValue::Bool(None))
            } else {
                Ok(FieldValue::Bool(Some(v == 1)))
            }
        }

        FieldType::Byte => {
            if record[off + 1] == 1 {
                Ok(FieldValue::Byte(None))
            } else {
                Ok(FieldValue::Byte(Some(record[off])))
            }
        }

        FieldType::Int16 => {
            if record[off + 2] == 1 {
                Ok(FieldValue::Int16(None))
            } else {
                let v = i16::from_le_bytes(record[off..off + 2].try_into().unwrap());
                Ok(FieldValue::Int16(Some(v)))
            }
        }

        FieldType::Int32 => {
            if record[off + 4] == 1 {
                Ok(FieldValue::Int32(None))
            } else {
                let v = i32::from_le_bytes(record[off..off + 4].try_into().unwrap());
                Ok(FieldValue::Int32(Some(v)))
            }
        }

        FieldType::Int64 => {
            if record[off + 8] == 1 {
                Ok(FieldValue::Int64(None))
            } else {
                let v = i64::from_le_bytes(record[off..off + 8].try_into().unwrap());
                Ok(FieldValue::Int64(Some(v)))
            }
        }

        FieldType::Float => {
            if record[off + 4] == 1 {
                Ok(FieldValue::Float(None))
            } else {
                let v = f32::from_le_bytes(record[off..off + 4].try_into().unwrap());
                Ok(FieldValue::Float(Some(v)))
            }
        }

        FieldType::Double => {
            if record[off + 8] == 1 {
                Ok(FieldValue::Double(None))
            } else {
                let v = f64::from_le_bytes(record[off..off + 8].try_into().unwrap());
                Ok(FieldValue::Double(Some(v)))
            }
        }

        FieldType::FixedDecimal => {
            if record[off + field.size] == 1 {
                Ok(FieldValue::Double(None))
            } else {
                let s = extract_fixed_string(record, off, field.size);
                let v: f64 = s.parse().unwrap_or(0.0);
                Ok(FieldValue::Double(Some(v)))
            }
        }

        FieldType::String => {
            if record[off + field.size] == 1 {
                Ok(FieldValue::String(None))
            } else {
                let s = extract_fixed_string(record, off, field.size);
                Ok(FieldValue::String(Some(s)))
            }
        }

        FieldType::WString => {
            let null_byte_off = off + field.size * 2;
            if record[null_byte_off] == 1 {
                Ok(FieldValue::String(None))
            } else {
                let s = extract_fixed_wstring(record, off, field.size);
                Ok(FieldValue::String(Some(s)))
            }
        }

        FieldType::VString => {
            let blob = parse_var_data(record, off);
            match blob {
                None => Ok(FieldValue::String(None)),
                Some(bytes) => {
                    let s = String::from_utf8_lossy(&bytes).to_string();
                    Ok(FieldValue::String(Some(s)))
                }
            }
        }

        FieldType::VWString => {
            let blob = parse_var_data(record, off);
            match blob {
                None => Ok(FieldValue::String(None)),
                Some(bytes) => {
                    if bytes.is_empty() {
                        Ok(FieldValue::String(Some(String::new())))
                    } else {
                        let code_units: Vec<u16> = bytes
                            .chunks_exact(2)
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect();
                        let s = String::from_utf16_lossy(&code_units);
                        Ok(FieldValue::String(Some(s)))
                    }
                }
            }
        }

        FieldType::Date => {
            if record[off + 10] == 1 {
                Ok(FieldValue::Date(None))
            } else {
                let s = extract_fixed_string(record, off, 10);
                Ok(FieldValue::Date(Some(s)))
            }
        }

        FieldType::Time => {
            if record[off + 8] == 1 {
                Ok(FieldValue::Time(None))
            } else {
                let s = extract_fixed_string(record, off, 8);
                Ok(FieldValue::Time(Some(s)))
            }
        }

        FieldType::DateTime => {
            if record[off + 19] == 1 {
                Ok(FieldValue::DateTime(None))
            } else {
                let s = extract_fixed_string(record, off, 19);
                Ok(FieldValue::DateTime(Some(s)))
            }
        }

        FieldType::Blob | FieldType::SpatialObj => {
            let blob = parse_var_data(record, off);
            Ok(FieldValue::Blob(blob))
        }
    }
}

/// Extract a null-terminated ASCII/Latin-1 string from the fixed portion.
pub fn extract_fixed_string(record: &[u8], start: usize, max_len: usize) -> String {
    let end = start + max_len;
    let slice = &record[start..end];
    // Find the first null byte
    let len = slice.iter().position(|&b| b == 0).unwrap_or(max_len);
    String::from_utf8_lossy(&slice[..len]).to_string()
}

/// Extract a null-terminated UTF-16LE string from the fixed portion.
pub fn extract_fixed_wstring(record: &[u8], start: usize, max_chars: usize) -> String {
    let byte_len = max_chars * 2;
    let end = start + byte_len;
    let slice = &record[start..end];

    // Find the first null char (two zero bytes on a 2-byte boundary)
    let mut char_count = 0;
    for chunk in slice.chunks_exact(2) {
        if chunk[0] == 0 && chunk[1] == 0 {
            break;
        }
        char_count += 1;
    }

    let code_units: Vec<u16> = slice[..char_count * 2]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&code_units)
}

/// Parse variable-length data from the record buffer.
///
/// The fixed portion contains a 4-byte value at `start`. This value encodes:
/// - `0` → empty (zero-length data)
/// - `1` → null
/// - High bit clear + bits 28-29 set → "tiny" inline blob (up to 3 bytes)
/// - High bit set → offset into the variable portion (clear high bit to get offset)
///
/// For non-tiny variable data, the variable block starts at
/// `start + (fixed_portion & 0x7FFFFFFF)`. The first byte of the block determines
/// whether it's a "small" block (low bit set → length = byte >> 1) or a "normal"
/// block (4-byte LE length, divided by 2).
pub fn parse_var_data(record: &[u8], start: usize) -> Option<Vec<u8>> {
    let fixed_portion =
        u32::from_le_bytes(record[start..start + 4].try_into().unwrap()) as usize;

    if fixed_portion == 0 {
        return Some(Vec::new());
    }
    if fixed_portion == 1 {
        return None; // null
    }

    // Check for tiny inline blob
    let bit_check_1 = fixed_portion & 0x80000000;
    let bit_check_2 = fixed_portion & 0x30000000;
    if bit_check_1 == 0 && bit_check_2 != 0 {
        // Tiny: length is in the top 4 bits (>>28), data is in the low bytes
        let length = fixed_portion >> 28;
        let mut blob = vec![0u8; length];
        blob.copy_from_slice(&record[start..start + length]);
        return Some(blob);
    }

    // Variable-length: offset into the record buffer
    let block_start = start + (fixed_portion & 0x7FFFFFFF);
    if block_start >= record.len() {
        return Some(Vec::new());
    }

    let first_byte = record[block_start];
    if first_byte & 1 == 1 {
        // Small block: length = first_byte >> 1
        let blob_len = (first_byte >> 1) as usize;
        let blob_start = block_start + 1;
        let blob_end = blob_start + blob_len;
        if blob_end > record.len() {
            return Some(Vec::new());
        }
        Some(record[blob_start..blob_end].to_vec())
    } else {
        // Normal block: 4-byte LE length (divided by 2)
        if block_start + 4 > record.len() {
            return Some(Vec::new());
        }
        let raw_len =
            u32::from_le_bytes(record[block_start..block_start + 4].try_into().unwrap()) as usize;
        let blob_len = raw_len / 2;
        let blob_start = block_start + 4;
        let blob_end = blob_start + blob_len;
        if blob_end > record.len() {
            return Some(Vec::new());
        }
        Some(record[blob_start..blob_end].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_int32_value() {
        // Int32 field at offset 0: value 42, not null
        let mut record = vec![0u8; 5];
        record[..4].copy_from_slice(&42i32.to_le_bytes());
        record[4] = 0; // not null

        let field = FieldMeta {
            name: "test".to_string(),
            field_type: FieldType::Int32,
            size: 4,
            scale: 0,
            offset: 0,
        };

        let val = extract_field(&record, &field).unwrap();
        assert_eq!(val, FieldValue::Int32(Some(42)));
    }

    #[test]
    fn extract_int32_null() {
        let mut record = vec![0u8; 5];
        record[4] = 1; // null flag

        let field = FieldMeta {
            name: "test".to_string(),
            field_type: FieldType::Int32,
            size: 4,
            scale: 0,
            offset: 0,
        };

        let val = extract_field(&record, &field).unwrap();
        assert_eq!(val, FieldValue::Int32(None));
    }

    #[test]
    fn extract_bool_values() {
        let field = FieldMeta {
            name: "flag".to_string(),
            field_type: FieldType::Bool,
            size: 1,
            scale: 0,
            offset: 0,
        };

        assert_eq!(
            extract_field(&[1], &field).unwrap(),
            FieldValue::Bool(Some(true))
        );
        assert_eq!(
            extract_field(&[0], &field).unwrap(),
            FieldValue::Bool(Some(false))
        );
        assert_eq!(
            extract_field(&[2], &field).unwrap(),
            FieldValue::Bool(None)
        );
    }

    #[test]
    fn extract_fixed_string_basic() {
        let s = extract_fixed_string(b"Hello\0\0\0\0\0", 0, 10);
        assert_eq!(s, "Hello");
    }

    #[test]
    fn var_data_null() {
        let record = 1u32.to_le_bytes();
        assert_eq!(parse_var_data(&record, 0), None);
    }

    #[test]
    fn var_data_empty() {
        let record = 0u32.to_le_bytes();
        assert_eq!(parse_var_data(&record, 0), Some(Vec::new()));
    }
}
