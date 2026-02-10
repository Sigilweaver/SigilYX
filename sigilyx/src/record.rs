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
    /// FixedDecimal stored as a formatted string to avoid precision loss.
    Decimal(Option<String>),
    String(Option<String>),
    Date(Option<String>),      // "YYYY-MM-DD"
    Time(Option<String>),      // "HH:MM:SS"
    DateTime(Option<String>),  // "YYYY-MM-DD HH:MM:SS"
    Blob(Option<Vec<u8>>),
}

/// Extract a field's value by column index.
///
/// This is a convenience wrapper around [`extract_field`] with bounds checking.
#[inline]
pub fn extract_field_index(
    record: &[u8],
    fields: &[FieldMeta],
    index: usize,
) -> Result<FieldValue> {
    let field = fields.get(index).ok_or_else(|| {
        crate::error::YxdbError::ConversionError(format!(
            "field index {} out of range (0..{})",
            index,
            fields.len()
        ))
    })?;
    extract_field(record, field)
}

/// Extract all field values from a record in one pass.
///
/// Returns a `Vec<FieldValue>` with one entry per field. This is more
/// efficient than calling [`extract_field_index`] in a loop when you
/// need all values (e.g. for Python FFI where each call crosses the
/// language boundary).
#[inline]
pub fn extract_all_fields(
    record: &[u8],
    fields: &[FieldMeta],
) -> Result<Vec<FieldValue>> {
    let mut values = Vec::with_capacity(fields.len());
    for field in fields {
        values.push(extract_field(record, field)?);
    }
    Ok(values)
}

/// Extract a field's value from a record buffer.
///
/// `record` is the full record buffer (fixed portion + variable portion).
/// `field` contains the offset and type information.
pub fn extract_field(record: &[u8], field: &FieldMeta) -> Result<FieldValue> {
    let off = field.offset;
    let required = off + field.field_type.fixed_bytes(field.size);
    if required > record.len() {
        return Err(crate::error::YxdbError::ConversionError(format!(
            "record too short: need {} bytes for field '{}' at offset {}, but record is {} bytes",
            required, field.name, off, record.len()
        )));
    }

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
                Ok(FieldValue::Decimal(None))
            } else {
                let slice = &record[off..off + field.size];
                let len = slice.iter().position(|&b| b == 0).unwrap_or(field.size);
                let s = match std::str::from_utf8(&slice[..len]) {
                    Ok(s) => s.to_owned(),
                    Err(_) => std::string::String::from_utf8_lossy(&slice[..len]).into_owned(),
                };
                Ok(FieldValue::Decimal(Some(s)))
            }
        }

        FieldType::String => {
            if record[off + field.size] == 1 {
                Ok(FieldValue::String(None))
            } else {
                let slice = &record[off..off + field.size];
                let len = slice.iter().position(|&b| b == 0).unwrap_or(field.size);
                let s = match std::str::from_utf8(&slice[..len]) {
                    Ok(s) => s.to_owned(),
                    Err(_) => String::from_utf8_lossy(&slice[..len]).into_owned(),
                };
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
            match locate_var_data(record, off) {
                None => Ok(FieldValue::String(None)),
                Some(bytes) if bytes.is_empty() => Ok(FieldValue::String(Some(String::new()))),
                Some(bytes) => {
                    let s = match std::str::from_utf8(bytes) {
                        Ok(s) => s.to_owned(),
                        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
                    };
                    Ok(FieldValue::String(Some(s)))
                }
            }
        }

        FieldType::VWString => {
            match locate_var_data(record, off) {
                None => Ok(FieldValue::String(None)),
                Some(bytes) if bytes.is_empty() => Ok(FieldValue::String(Some(String::new()))),
                Some(bytes) => {
                    let code_units: Vec<u16> = bytes
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    let s = String::from_utf16_lossy(&code_units);
                    Ok(FieldValue::String(Some(s)))
                }
            }
        }

        FieldType::Date => {
            if record[off + 10] == 1 {
                Ok(FieldValue::Date(None))
            } else {
                let slice = &record[off..off + 10];
                let len = slice.iter().position(|&b| b == 0).unwrap_or(10);
                let s = match std::str::from_utf8(&slice[..len]) {
                    Ok(s) => s.to_owned(),
                    Err(_) => String::from_utf8_lossy(&slice[..len]).into_owned(),
                };
                Ok(FieldValue::Date(Some(s)))
            }
        }

        FieldType::Time => {
            if record[off + 8] == 1 {
                Ok(FieldValue::Time(None))
            } else {
                let slice = &record[off..off + 8];
                let len = slice.iter().position(|&b| b == 0).unwrap_or(8);
                let s = match std::str::from_utf8(&slice[..len]) {
                    Ok(s) => s.to_owned(),
                    Err(_) => String::from_utf8_lossy(&slice[..len]).into_owned(),
                };
                Ok(FieldValue::Time(Some(s)))
            }
        }

        FieldType::DateTime => {
            if record[off + 19] == 1 {
                Ok(FieldValue::DateTime(None))
            } else {
                let slice = &record[off..off + 19];
                let len = slice.iter().position(|&b| b == 0).unwrap_or(19);
                let s = match std::str::from_utf8(&slice[..len]) {
                    Ok(s) => s.to_owned(),
                    Err(_) => String::from_utf8_lossy(&slice[..len]).into_owned(),
                };
                Ok(FieldValue::DateTime(Some(s)))
            }
        }

        FieldType::Blob | FieldType::SpatialObj => {
            // Blobs still use parse_var_data since FieldValue owns the Vec
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

/// Parse variable-length data from the record buffer (owned variant).
///
/// Delegates to [`locate_var_data`] and copies the result into a `Vec<u8>`.
/// Use `locate_var_data` on hot paths to avoid heap allocation.
///
/// See [`locate_var_data`] for the encoding format documentation.
pub fn parse_var_data(record: &[u8], start: usize) -> Option<Vec<u8>> {
    locate_var_data(record, start).map(|s| s.to_vec())
}

/// Zero-copy variable-length data extraction from a record buffer.
///
/// Returns a borrowed slice into the record buffer instead of an owned `Vec<u8>`,
/// avoiding heap allocation on the hot path.
///
/// The fixed portion contains a 4-byte value at `start`. This value encodes:
/// - `0` → empty (zero-length data)
/// - `1` → null
/// - High bit clear + bits 28-29 set → "tiny" inline blob (up to 7 bytes)
/// - High bit set → offset into the variable portion (clear high bit to get offset)
///
/// For non-tiny variable data, the variable block starts at
/// `start + (fixed_portion & 0x7FFFFFFF)`. The first byte of the block determines
/// whether it's a "small" block (low bit set → length = byte >> 1) or a "normal"
/// block (4-byte LE length, divided by 2).
#[inline]
pub fn locate_var_data<'a>(record: &'a [u8], start: usize) -> Option<&'a [u8]> {
    let fixed_portion =
        u32::from_le_bytes(record[start..start + 4].try_into().unwrap()) as usize;

    if fixed_portion == 0 {
        return Some(&[]); // empty, not null
    }
    if fixed_portion == 1 {
        return None; // null
    }

    // Check for tiny inline blob
    let bit_check_1 = fixed_portion & 0x80000000;
    let bit_check_2 = fixed_portion & 0x30000000;
    if bit_check_1 == 0 && bit_check_2 != 0 {
        let length = fixed_portion >> 28;
        return Some(&record[start..start + length]);
    }

    // Variable-length: offset into the record buffer
    let block_start = start + (fixed_portion & 0x7FFFFFFF);
    if block_start >= record.len() {
        return Some(&[]);
    }

    let first_byte = record[block_start];
    if first_byte & 1 == 1 {
        // Small block: length = first_byte >> 1
        let blob_len = (first_byte >> 1) as usize;
        let blob_start = block_start + 1;
        let blob_end = blob_start + blob_len;
        if blob_end > record.len() {
            return Some(&[]);
        }
        Some(&record[blob_start..blob_end])
    } else {
        // Normal block: 4-byte LE length (divided by 2)
        if block_start + 4 > record.len() {
            return Some(&[]);
        }
        let raw_len =
            u32::from_le_bytes(record[block_start..block_start + 4].try_into().unwrap()) as usize;
        let blob_len = raw_len / 2;
        let blob_start = block_start + 4;
        let blob_end = blob_start + blob_len;
        if blob_end > record.len() {
            return Some(&[]);
        }
        Some(&record[blob_start..blob_end])
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
    fn extract_fixed_string_full_no_null() {
        // String fills entire buffer with no null terminator
        let s = extract_fixed_string(b"ABCDE", 0, 5);
        assert_eq!(s, "ABCDE");
    }

    #[test]
    fn extract_fixed_string_empty() {
        let s = extract_fixed_string(b"\0\0\0\0\0", 0, 5);
        assert_eq!(s, "");
    }

    #[test]
    fn extract_fixed_string_with_offset() {
        let buf = b"XXXHello\0\0";
        let s = extract_fixed_string(buf, 3, 7);
        assert_eq!(s, "Hello");
    }

    #[test]
    fn extract_fixed_wstring_basic() {
        // "AB" in UTF-16LE: [0x41, 0x00, 0x42, 0x00], then null pad
        let record = vec![0x41, 0x00, 0x42, 0x00, 0x00, 0x00, 0x00, 0x00];
        let s = extract_fixed_wstring(&record, 0, 4);
        assert_eq!(s, "AB");
    }

    #[test]
    fn extract_fixed_wstring_empty() {
        // All null code units
        let record = vec![0x00; 8];
        let s = extract_fixed_wstring(&record, 0, 4);
        assert_eq!(s, "");
    }

    #[test]
    fn extract_fixed_wstring_unicode() {
        // U+00FC (ü) = [0xFC, 0x00] in UTF-16LE
        let record = vec![0xFC, 0x00, 0x00, 0x00, 0x00, 0x00];
        let s = extract_fixed_wstring(&record, 0, 3);
        assert_eq!(s, "ü");
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

    #[test]
    fn var_data_small_block() {
        // Small block: first byte has low bit set, length = byte >> 1
        // Put the pointer at offset 0: high bit set + offset to byte 4
        // 0x80000004 = high bit set, offset 4
        let mut record = Vec::new();
        record.extend_from_slice(&0x80000004u32.to_le_bytes()); // fixed portion
        // The block starts at start + (0x80000004 & 0x7FFFFFFF) = 0 + 4 = byte 4
        // Small block header: length 3 → (3 << 1) | 1 = 7
        record.push(7);
        record.extend_from_slice(b"ABC");
        let data = locate_var_data(&record, 0);
        assert_eq!(data, Some(&b"ABC"[..]));
    }

    #[test]
    fn var_data_normal_block() {
        // Normal block: first byte has low bit clear, 4-byte LE length (raw_len / 2 = actual length)
        let mut record = Vec::new();
        record.extend_from_slice(&0x80000004u32.to_le_bytes()); // fixed portion
        // Block at byte 4: 4-byte header, length in bytes = raw_len / 2
        let actual_len = 5u32;
        let raw_len = actual_len * 2;
        record.extend_from_slice(&raw_len.to_le_bytes());
        record.extend_from_slice(b"Hello");
        let data = locate_var_data(&record, 0);
        assert_eq!(data, Some(&b"Hello"[..]));
    }

    #[test]
    fn extract_all_fields_basic() {
        // Two fields: Int32 and Bool
        let mut record = vec![0u8; 6]; // 4 + 1 (null) + 1 (bool)
        record[0..4].copy_from_slice(&99i32.to_le_bytes());
        record[4] = 0; // not null
        record[5] = 1; // true

        let fields = vec![
            FieldMeta { name: "num".into(), field_type: FieldType::Int32, size: 4, scale: 0, offset: 0 },
            FieldMeta { name: "flag".into(), field_type: FieldType::Bool, size: 1, scale: 0, offset: 5 },
        ];
        let vals = extract_all_fields(&record, &fields).unwrap();
        assert_eq!(vals.len(), 2);
        assert_eq!(vals[0], FieldValue::Int32(Some(99)));
        assert_eq!(vals[1], FieldValue::Bool(Some(true)));
    }

    #[test]
    fn extract_field_index_out_of_range() {
        let record = vec![0u8; 5];
        let fields = vec![
            FieldMeta { name: "x".into(), field_type: FieldType::Int32, size: 4, scale: 0, offset: 0 },
        ];
        assert!(extract_field_index(&record, &fields, 1).is_err());
    }

    #[test]
    fn extract_field_record_too_short() {
        let record = vec![0u8; 2]; // too short for Int32 (needs 5)
        let field = FieldMeta { name: "x".into(), field_type: FieldType::Int32, size: 4, scale: 0, offset: 0 };
        assert!(extract_field(&record, &field).is_err());
    }

    #[test]
    fn extract_byte_values() {
        let field = FieldMeta { name: "b".into(), field_type: FieldType::Byte, size: 1, scale: 0, offset: 0 };
        // Non-null value 255
        assert_eq!(extract_field(&[255, 0], &field).unwrap(), FieldValue::Byte(Some(255)));
        // Non-null value 0
        assert_eq!(extract_field(&[0, 0], &field).unwrap(), FieldValue::Byte(Some(0)));
        // Null
        assert_eq!(extract_field(&[0, 1], &field).unwrap(), FieldValue::Byte(None));
    }

    #[test]
    fn extract_int16_values() {
        let field = FieldMeta { name: "i".into(), field_type: FieldType::Int16, size: 2, scale: 0, offset: 0 };
        // i16::MIN = -32768
        let mut rec = i16::MIN.to_le_bytes().to_vec();
        rec.push(0); // not null
        assert_eq!(extract_field(&rec, &field).unwrap(), FieldValue::Int16(Some(i16::MIN)));
        // Null
        let mut rec = [0u8; 3];
        rec[2] = 1;
        assert_eq!(extract_field(&rec, &field).unwrap(), FieldValue::Int16(None));
    }

    #[test]
    fn extract_double_values() {
        let field = FieldMeta { name: "d".into(), field_type: FieldType::Double, size: 8, scale: 0, offset: 0 };
        // Infinity
        let mut rec = f64::INFINITY.to_le_bytes().to_vec();
        rec.push(0);
        assert_eq!(extract_field(&rec, &field).unwrap(), FieldValue::Double(Some(f64::INFINITY)));
        // NaN
        let mut rec = f64::NAN.to_le_bytes().to_vec();
        rec.push(0);
        match extract_field(&rec, &field).unwrap() {
            FieldValue::Double(Some(v)) => assert!(v.is_nan()),
            other => panic!("expected NaN, got {other:?}"),
        }
        // Null
        let mut rec = [0u8; 9];
        rec[8] = 1;
        assert_eq!(extract_field(&rec, &field).unwrap(), FieldValue::Double(None));
    }

    #[test]
    fn extract_float_values() {
        let field = FieldMeta { name: "f".into(), field_type: FieldType::Float, size: 4, scale: 0, offset: 0 };
        let mut rec = 3.14f32.to_le_bytes().to_vec();
        rec.push(0);
        match extract_field(&rec, &field).unwrap() {
            FieldValue::Float(Some(v)) => assert!((v - 3.14).abs() < 0.001),
            other => panic!("expected float, got {other:?}"),
        }
    }

    #[test]
    fn extract_int64_boundary() {
        let field = FieldMeta { name: "i".into(), field_type: FieldType::Int64, size: 8, scale: 0, offset: 0 };
        let mut rec = i64::MAX.to_le_bytes().to_vec();
        rec.push(0);
        assert_eq!(extract_field(&rec, &field).unwrap(), FieldValue::Int64(Some(i64::MAX)));
        let mut rec = i64::MIN.to_le_bytes().to_vec();
        rec.push(0);
        assert_eq!(extract_field(&rec, &field).unwrap(), FieldValue::Int64(Some(i64::MIN)));
    }
}
