//! E2 compact record field decoding.
//!
//! E2 uses variable-length, compact encoding for all field types:
//! - Integers use a prefix byte P where (P − base) value bytes follow in LE
//! - Strings are length-prefixed UTF-8
//! - Dates/times are encoded as integer serials
//!
//! Each decode function returns `(FieldValue, bytes_consumed)`.

use crate::field::FieldType;

/// Decoded field value from an E2 record.
///
/// Mirrors the E1 FieldValue but is specific to E2 decoding.
/// We re-use the same enum from E1 for compatibility.
pub use crate::e1::record::FieldValue;

/// Type-specific null byte codes (0x40 + type_code).
/// When a field value equals one of these, it's null.
const NULL_VSTRING: u8 = 0x41;
const NULL_VWSTRING: u8 = 0x41;
const NULL_BOOL: u8 = 0x43;
const NULL_INT16: u8 = 0x45;
const NULL_BYTE: u8 = 0x47;
const NULL_DOUBLE: u8 = 0x48;
const NULL_INT32: u8 = 0x49;
const NULL_INT64: u8 = 0x4A;
const NULL_FLOAT: u8 = 0x4B;
const NULL_DOUBLE_ALT: u8 = 0x4C; // Alternate Double null observed in corpus
const NULL_DATE: u8 = 0x4D;
const NULL_DATETIME: u8 = 0x4E;
const NULL_TIME: u8 = 0x4F;

// ── UNVERIFIED null bytes ────────────────────────────────────────────
// The following null bytes are predicted from the 0x40+type_code pattern
// but have NEVER been observed in any corpus file. They may be wrong.
const NULL_FIXED_DECIMAL: u8 = 0x4C; // UNVERIFIED — type code 12, conflicts with NULL_DOUBLE_ALT
const NULL_STRING: u8 = 0x41; // UNVERIFIED — assumed same as V_String
const NULL_WSTRING: u8 = 0x41; // UNVERIFIED — assumed same as V_WString
const NULL_BLOB: u8 = 0x41; // Confirmed — same as V_String/V_WString
#[allow(dead_code)] // Documented for reference; SpatialObj has its own decoder
const NULL_SPATIAL: u8 = 0x43; // Predicted — type code 3 → 0x40+3=0x43

/// Returns `true` if the given field type has been verified against real E2
/// corpus data. Types that return `false` have speculative decoders that may
/// produce incorrect results.
pub fn is_e2_verified_type(ft: FieldType) -> bool {
    matches!(
        ft,
        FieldType::Bool
            | FieldType::Byte
            | FieldType::Int16
            | FieldType::Int32
            | FieldType::Int64
            | FieldType::Float
            | FieldType::Double
            | FieldType::Date
            | FieldType::DateTime
            | FieldType::FixedDecimal
            | FieldType::String
            | FieldType::VString
            | FieldType::VWString
            | FieldType::Blob
            | FieldType::SpatialObj
    )
}

/// OLE/Excel date epoch: 1899-12-30 as days before Unix epoch.
/// day_serial 1 = 1899-12-31, day_serial 2 = 1900-01-01, etc.
const OLE_EPOCH_OFFSET: i64 = 25569; // days from 1899-12-30 to 1970-01-01

/// Decode a single field from the E2 record data at the given offset.
///
/// Returns `Ok((value, bytes_consumed))` or an error if the data is malformed.
///
/// `has_date_flag` should be `true` if a 0x00 flag byte is expected before the
/// first Date field in the record.
pub fn decode_field(
    data: &[u8],
    offset: usize,
    field_type: FieldType,
    is_first_date_field: bool,
    has_date_flag: bool,
) -> Result<(FieldValue, usize), DecodeError> {
    if offset >= data.len() {
        return Err(DecodeError::UnexpectedEof);
    }

    // Handle the date flag byte: 0x00 before first Date field in some files
    let mut pos = offset;
    if is_first_date_field && has_date_flag {
        if data[pos] != 0x00 {
            return Err(DecodeError::InvalidDateFlag(data[pos]));
        }
        pos += 1;
        if pos >= data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
    }

    let _prefix = data[pos];

    match field_type {
        FieldType::Bool => decode_bool(data, pos),
        FieldType::Byte => decode_compact_int(data, pos, 6, NULL_BYTE, IntTarget::Byte),
        FieldType::Int16 => decode_compact_int(data, pos, 6, NULL_INT16, IntTarget::Int16),
        FieldType::Int32 => decode_compact_int(data, pos, 6, NULL_INT32, IntTarget::Int32),
        FieldType::Int64 => decode_compact_int(data, pos, 6, NULL_INT64, IntTarget::Int64),
        FieldType::Float => decode_float(data, pos),
        FieldType::Double => decode_double(data, pos),
        FieldType::VString | FieldType::VWString => {
            let null_byte = if field_type == FieldType::VString {
                NULL_VSTRING
            } else {
                NULL_VWSTRING
            };
            decode_string(data, pos, null_byte)
        }
        FieldType::Date => decode_date(data, pos),
        FieldType::DateTime => decode_datetime(data, pos),
        FieldType::Time => decode_time(data, pos),

        // ── UNVERIFIED TYPES ────────────────────────────────────────
        // The decoders below are speculative. They have NEVER been
        // validated against real E2 corpus files. The encoding is our
        // best guess based on E1 patterns and the compact encoding
        // scheme, but may be completely wrong.
        // ─────────────────────────────────────────────────────────────

        // UNVERIFIED: FixedDecimal — guessing it uses the same
        // length-prefixed UTF-8 encoding as V_String, carrying the
        // ASCII decimal representation (like E1). Null byte 0x4C.
        FieldType::FixedDecimal => decode_fixed_decimal(data, pos),

        // UNVERIFIED: Fixed-width String/WString — E2 may not even
        // support these types. Guessing they use the same variable-
        // length UTF-8 encoding as V_String since E2 is all-UTF-8.
        FieldType::String => decode_string(data, pos, NULL_STRING),
        FieldType::WString => decode_string(data, pos, NULL_WSTRING),

        // Blob — inline data via 0x80|len or 0x02+u16 length prefixes,
        // and 0x12 blob references for large values. Returns raw bytes.
        FieldType::Blob => decode_blob(data, pos),

        // SpatialObj — inline data via 0x80|len or 0x03+u16 length
        // prefixes, and 0x13 blob references for large values.
        // Binary format is ESRI Shapefile geometry records.
        FieldType::SpatialObj => decode_spatial(data, pos),
    }
    .map(|(val, end)| {
        let consumed = end - offset;
        (val, consumed)
    })
}

/// Errors during E2 field decoding.
#[derive(Debug)]
pub enum DecodeError {
    UnexpectedEof,
    InvalidDateFlag(u8),
    UnsupportedType(FieldType),
    InvalidPrefix(u8, FieldType),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::UnexpectedEof => write!(f, "unexpected end of record data"),
            DecodeError::InvalidDateFlag(b) => {
                write!(f, "expected date flag 0x00, got 0x{b:02X}")
            }
            DecodeError::UnsupportedType(t) => {
                write!(f, "E2 decoding not implemented for type {t}")
            }
            DecodeError::InvalidPrefix(p, t) => {
                write!(f, "invalid E2 prefix 0x{p:02X} for type {t}")
            }
        }
    }
}

/// Target integer type for compact int decoding.
enum IntTarget {
    Byte,
    Int16,
    Int32,
    Int64,
}

// ── Bool ────────────────────────────────────────────────────────────

fn decode_bool(data: &[u8], pos: usize) -> Result<(FieldValue, usize), DecodeError> {
    let b = data[pos];
    match b {
        0x14 => Ok((FieldValue::Bool(Some(false)), pos + 1)),
        0x15 => Ok((FieldValue::Bool(Some(true)), pos + 1)),
        NULL_BOOL => Ok((FieldValue::Bool(None), pos + 1)),
        _ => Err(DecodeError::InvalidPrefix(b, FieldType::Bool)),
    }
}

// ── Compact Integer ─────────────────────────────────────────────────

fn decode_compact_int(
    data: &[u8],
    pos: usize,
    base: u8,
    null_byte: u8,
    target: IntTarget,
) -> Result<(FieldValue, usize), DecodeError> {
    let prefix = data[pos];

    // Type-specific null byte
    if prefix == null_byte {
        return Ok((int_null(&target), pos + 1));
    }

    // Below-base null
    if prefix < base {
        return Ok((int_null(&target), pos + 1));
    }

    let n_bytes = (prefix - base) as usize;

    // Zero (prefix == base, 0 value bytes)
    if n_bytes == 0 {
        return Ok((int_zero(&target), pos + 1));
    }

    let end = pos + 1 + n_bytes;
    if end > data.len() {
        return Err(DecodeError::UnexpectedEof);
    }

    // Read value bytes (LE, zero-padded to target size)
    let val_bytes = &data[pos + 1..end];

    match target {
        IntTarget::Byte => {
            let v = val_bytes[0];
            Ok((FieldValue::Byte(Some(v)), end))
        }
        IntTarget::Int16 => {
            let mut buf = [0u8; 2];
            buf[..n_bytes.min(2)].copy_from_slice(&val_bytes[..n_bytes.min(2)]);
            let v = i16::from_le_bytes(buf);
            Ok((FieldValue::Int16(Some(v)), end))
        }
        IntTarget::Int32 => {
            let mut buf = [0u8; 4];
            buf[..n_bytes.min(4)].copy_from_slice(&val_bytes[..n_bytes.min(4)]);
            let v = i32::from_le_bytes(buf);
            Ok((FieldValue::Int32(Some(v)), end))
        }
        IntTarget::Int64 => {
            let mut buf = [0u8; 8];
            buf[..n_bytes.min(8)].copy_from_slice(&val_bytes[..n_bytes.min(8)]);
            let v = i64::from_le_bytes(buf);
            Ok((FieldValue::Int64(Some(v)), end))
        }
    }
}

fn int_null(target: &IntTarget) -> FieldValue {
    match target {
        IntTarget::Byte => FieldValue::Byte(None),
        IntTarget::Int16 => FieldValue::Int16(None),
        IntTarget::Int32 => FieldValue::Int32(None),
        IntTarget::Int64 => FieldValue::Int64(None),
    }
}

fn int_zero(target: &IntTarget) -> FieldValue {
    match target {
        IntTarget::Byte => FieldValue::Byte(Some(0)),
        IntTarget::Int16 => FieldValue::Int16(Some(0)),
        IntTarget::Int32 => FieldValue::Int32(Some(0)),
        IntTarget::Int64 => FieldValue::Int64(Some(0)),
    }
}

// ── Float ───────────────────────────────────────────────────────────

fn decode_float(data: &[u8], pos: usize) -> Result<(FieldValue, usize), DecodeError> {
    let prefix = data[pos];

    // Type-specific null
    if prefix == NULL_FLOAT {
        return Ok((FieldValue::Float(None), pos + 1));
    }

    // Below-base null (base = 7; prefixes 0x00..0x06 are null)
    if prefix < 0x07 {
        return Ok((FieldValue::Float(None), pos + 1));
    }

    // Base prefix = zero
    if prefix == 0x07 {
        return Ok((FieldValue::Float(Some(0.0)), pos + 1));
    }

    let n_bytes = (prefix - 0x07) as usize; // 1..4
    let end = pos + 1 + n_bytes;
    if end > data.len() {
        return Err(DecodeError::UnexpectedEof);
    }

    let mut buf = [0u8; 4];
    let copy_len = n_bytes.min(4);
    buf[..copy_len].copy_from_slice(&data[pos + 1..pos + 1 + copy_len]);
    let v = f32::from_le_bytes(buf);
    Ok((FieldValue::Float(Some(v)), end))
}

// ── Double ──────────────────────────────────────────────────────────

fn decode_double(data: &[u8], pos: usize) -> Result<(FieldValue, usize), DecodeError> {
    let prefix = data[pos];

    // Type-specific null bytes
    if prefix == NULL_DOUBLE || prefix == NULL_DOUBLE_ALT {
        return Ok((FieldValue::Double(None), pos + 1));
    }

    // Below zero prefix: null (prefixes 0x00..0x05)
    if prefix < 0x06 {
        return Ok((FieldValue::Double(None), pos + 1));
    }

    // Zero prefix (0x06) — special case: value is 0.0, no data bytes
    if prefix == 0x06 {
        return Ok((FieldValue::Double(Some(0.0)), pos + 1));
    }

    // Data bytes: n = prefix - 4 (prefixes 0x07..0x0C → 3..8 bytes)
    let n_bytes = (prefix - 0x04) as usize;
    let end = pos + 1 + n_bytes;
    if end > data.len() {
        return Err(DecodeError::UnexpectedEof);
    }

    let mut buf = [0u8; 8];
    let copy_len = n_bytes.min(8);
    buf[..copy_len].copy_from_slice(&data[pos + 1..pos + 1 + copy_len]);
    let v = f64::from_le_bytes(buf);
    Ok((FieldValue::Double(Some(v)), end))
}

// ── String (V_String / V_WString) ───────────────────────────────────

fn decode_string(
    data: &[u8],
    pos: usize,
    null_byte: u8,
) -> Result<(FieldValue, usize), DecodeError> {
    let prefix = data[pos];

    // Null: below-base (0x00)
    if prefix == 0x00 {
        return Ok((FieldValue::String(None), pos + 1));
    }

    // Type-specific null
    if prefix == null_byte {
        return Ok((FieldValue::String(None), pos + 1));
    }

    // Short string: prefix = 0x80 | len (len = 1..127)
    if prefix & 0x80 != 0 {
        let len = (prefix & 0x7F) as usize;
        let end = pos + 1 + len;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let s = String::from_utf8_lossy(&data[pos + 1..end]).into_owned();
        return Ok((FieldValue::String(Some(s)), end));
    }

    // Long string: prefix = 0x01, followed by u16 LE length
    if prefix == 0x01 {
        if pos + 3 > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let len = u16::from_le_bytes(data[pos + 1..pos + 3].try_into().unwrap()) as usize;
        let end = pos + 3 + len;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let s = String::from_utf8_lossy(&data[pos + 3..end]).into_owned();
        return Ok((FieldValue::String(Some(s)), end));
    }

    // Blob reference: prefix = 0x11 + 8 bytes
    // The 8 bytes encode a reference to data in a type 0x01 block.
    // We consume the bytes here but return a sentinel that the reader
    // can resolve against blob_data. The 8 bytes appear to be:
    //   u32 LE offset into decompressed blob data
    //   u32 LE length of the referenced slice
    // This is PARTIALLY VERIFIED (only 1 corpus file uses blobs).
    if prefix == 0x11 {
        let end = pos + 1 + 8;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let blob_offset = u32::from_le_bytes(data[pos + 1..pos + 5].try_into().unwrap()) as usize;
        let blob_len = u32::from_le_bytes(data[pos + 5..pos + 9].try_into().unwrap()) as usize;
        return Ok((FieldValue::BlobRef(blob_offset, blob_len), end));
    }

    Err(DecodeError::InvalidPrefix(prefix, FieldType::VString))
}

// ── FixedDecimal (UNVERIFIED) ────────────────────────────────────────
//
// WARNING: This decoder has NEVER been validated against real E2 data.
// We guess FixedDecimal uses the same length-prefixed UTF-8 encoding as
// V_String, carrying the ASCII decimal text (e.g. "123.456789") that
// E1 stores in fixed-width fields. The null byte 0x4C is predicted from
// the 0x40+type_code pattern (type_code=12) but ALSO collides with the
// alternate Double null. If this guess is wrong, decoding will produce
// garbage or errors.

fn decode_fixed_decimal(data: &[u8], pos: usize) -> Result<(FieldValue, usize), DecodeError> {
    let prefix = data[pos];

    // Type-specific null (UNVERIFIED)
    if prefix == NULL_FIXED_DECIMAL {
        return Ok((FieldValue::Decimal(None), pos + 1));
    }

    // Null: below-base (UNVERIFIED — assuming same 0x00 as V_String)
    if prefix == 0x00 {
        return Ok((FieldValue::Decimal(None), pos + 1));
    }

    // Short string: prefix = 0x80 | len
    if prefix & 0x80 != 0 {
        let len = (prefix & 0x7F) as usize;
        let end = pos + 1 + len;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let s = String::from_utf8_lossy(&data[pos + 1..end]).into_owned();
        return Ok((FieldValue::Decimal(Some(s)), end));
    }

    // Long string: prefix = 0x01 + u16 LE len
    if prefix == 0x01 {
        if pos + 3 > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let len = u16::from_le_bytes(data[pos + 1..pos + 3].try_into().unwrap()) as usize;
        let end = pos + 3 + len;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let s = String::from_utf8_lossy(&data[pos + 3..end]).into_owned();
        return Ok((FieldValue::Decimal(Some(s)), end));
    }

    Err(DecodeError::InvalidPrefix(prefix, FieldType::FixedDecimal))
}

// ── Blob ─────────────────────────────────────────────────────────────
//
// Blob fields use type-class 2: inline via 0x80|len or 0x02+u16,
// blob references via 0x12+u64 (absolute file offset to type 0x01 block).

fn decode_blob(data: &[u8], pos: usize) -> Result<(FieldValue, usize), DecodeError> {
    let prefix = data[pos];

    // Null
    if prefix == 0x00 || prefix == NULL_BLOB {
        return Ok((FieldValue::Blob(None), pos + 1));
    }

    // Short inline: prefix = 0x80 | len
    if prefix & 0x80 != 0 {
        let len = (prefix & 0x7F) as usize;
        let end = pos + 1 + len;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        return Ok((FieldValue::Blob(Some(data[pos + 1..end].to_vec())), end));
    }

    // Long inline string (type class 1): prefix = 0x01 + u16 LE len
    // (kept for backward compat with existing corpus / V_String-like paths)
    if prefix == 0x01 {
        if pos + 3 > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let len = u16::from_le_bytes(data[pos + 1..pos + 3].try_into().unwrap()) as usize;
        let end = pos + 3 + len;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        return Ok((FieldValue::Blob(Some(data[pos + 3..end].to_vec())), end));
    }

    // Long inline blob (type class 2): prefix = 0x02 + u16 LE len
    if prefix == 0x02 {
        if pos + 3 > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let len = u16::from_le_bytes(data[pos + 1..pos + 3].try_into().unwrap()) as usize;
        let end = pos + 3 + len;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        return Ok((FieldValue::Blob(Some(data[pos + 3..end].to_vec())), end));
    }

    // Blob reference (type class 1 — V_String style): prefix = 0x11 + 8 bytes
    // u32 offset + u32 length into concatenated blob_data
    if prefix == 0x11 {
        let end = pos + 1 + 8;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let blob_offset = u32::from_le_bytes(data[pos + 1..pos + 5].try_into().unwrap()) as usize;
        let blob_len = u32::from_le_bytes(data[pos + 5..pos + 9].try_into().unwrap()) as usize;
        return Ok((FieldValue::BlobRef(blob_offset, blob_len), end));
    }

    // Blob reference (type class 2 — file offset): prefix = 0x12 + u64 LE
    // u64 absolute file offset to start of type 0x01 block
    if prefix == 0x12 {
        let end = pos + 1 + 8;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let file_offset = u64::from_le_bytes(data[pos + 1..pos + 9].try_into().unwrap()) as usize;
        // Use BlobRef with a sentinel length of usize::MAX to signal
        // "file-offset reference" vs "offset+length reference"
        return Ok((FieldValue::BlobRef(file_offset, usize::MAX), end));
    }

    Err(DecodeError::InvalidPrefix(prefix, FieldType::Blob))
}

// ── SpatialObj ──────────────────────────────────────────────────────
//
// SpatialObj fields use type-class 3: inline via 0x80|len or 0x03+u16,
// blob references via 0x13+u64 (absolute file offset to type 0x01 block).
// Binary data is ESRI Shapefile geometry records.

fn decode_spatial(data: &[u8], pos: usize) -> Result<(FieldValue, usize), DecodeError> {
    let prefix = data[pos];

    // Null
    if prefix == 0x00 || prefix == NULL_SPATIAL {
        return Ok((FieldValue::Blob(None), pos + 1));
    }

    // Short inline: prefix = 0x80 | len
    if prefix & 0x80 != 0 {
        let len = (prefix & 0x7F) as usize;
        let end = pos + 1 + len;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        return Ok((FieldValue::Blob(Some(data[pos + 1..end].to_vec())), end));
    }

    // Long inline spatial (type class 3): prefix = 0x03 + u16 LE len
    if prefix == 0x03 {
        if pos + 3 > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let len = u16::from_le_bytes(data[pos + 1..pos + 3].try_into().unwrap()) as usize;
        let end = pos + 3 + len;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        return Ok((FieldValue::Blob(Some(data[pos + 3..end].to_vec())), end));
    }

    // Blob reference (type class 3 — file offset): prefix = 0x13 + u64 LE
    if prefix == 0x13 {
        let end = pos + 1 + 8;
        if end > data.len() {
            return Err(DecodeError::UnexpectedEof);
        }
        let file_offset = u64::from_le_bytes(data[pos + 1..pos + 9].try_into().unwrap()) as usize;
        return Ok((FieldValue::BlobRef(file_offset, usize::MAX), end));
    }

    Err(DecodeError::InvalidPrefix(prefix, FieldType::SpatialObj))
}

// ── Date ────────────────────────────────────────────────────────────

fn decode_date(data: &[u8], pos: usize) -> Result<(FieldValue, usize), DecodeError> {
    let prefix = data[pos];

    // Type-specific null
    if prefix == NULL_DATE {
        return Ok((FieldValue::Date(None), pos + 1));
    }

    // Below-base null (base = 0x0A)
    if prefix < 0x0A {
        return Ok((FieldValue::Date(None), pos + 1));
    }

    // Zero (base prefix)
    if prefix == 0x0A {
        // Day serial 0 = 1899-12-30
        let s = day_serial_to_date_str(0);
        return Ok((FieldValue::Date(Some(s)), pos + 1));
    }

    let n_bytes = (prefix - 0x0A) as usize; // 1..4
    let end = pos + 1 + n_bytes;
    if end > data.len() {
        return Err(DecodeError::UnexpectedEof);
    }

    let mut buf = [0u8; 4];
    let copy_len = n_bytes.min(4);
    buf[..copy_len].copy_from_slice(&data[pos + 1..pos + 1 + copy_len]);
    let day_serial = u32::from_le_bytes(buf) as i64;
    let s = day_serial_to_date_str(day_serial);
    Ok((FieldValue::Date(Some(s)), end))
}

// ── DateTime ────────────────────────────────────────────────────────

fn decode_datetime(data: &[u8], pos: usize) -> Result<(FieldValue, usize), DecodeError> {
    let prefix = data[pos];

    // Type-specific null
    if prefix == NULL_DATETIME {
        return Ok((FieldValue::DateTime(None), pos + 1));
    }

    // Below-base null (base = 8)
    if prefix < 0x08 {
        return Ok((FieldValue::DateTime(None), pos + 1));
    }

    // Zero (base prefix)
    if prefix == 0x08 {
        let s = datetime_packed_to_str(0);
        return Ok((FieldValue::DateTime(Some(s)), pos + 1));
    }

    let n_bytes = (prefix - 0x08) as usize; // 1..6
    let end = pos + 1 + n_bytes;
    if end > data.len() {
        return Err(DecodeError::UnexpectedEof);
    }

    // Read up to 6 bytes into a u64
    let mut buf = [0u8; 8];
    let copy_len = n_bytes.min(6);
    buf[..copy_len].copy_from_slice(&data[pos + 1..pos + 1 + copy_len]);
    let raw = u64::from_le_bytes(buf);
    let s = datetime_packed_to_str(raw);
    Ok((FieldValue::DateTime(Some(s)), end))
}

// ── Time ────────────────────────────────────────────────────────────

fn decode_time(data: &[u8], pos: usize) -> Result<(FieldValue, usize), DecodeError> {
    let prefix = data[pos];

    // Type-specific null
    if prefix == NULL_TIME {
        return Ok((FieldValue::Time(None), pos + 1));
    }

    // Below-base null (predicted base = 0x0C)
    if prefix < 0x0C {
        return Ok((FieldValue::Time(None), pos + 1));
    }

    // Zero (base prefix)
    if prefix == 0x0C {
        return Ok((FieldValue::Time(Some("00:00:00".to_string())), pos + 1));
    }

    let n_bytes = (prefix - 0x0C) as usize;
    let end = pos + 1 + n_bytes;
    if end > data.len() {
        return Err(DecodeError::UnexpectedEof);
    }

    let mut buf = [0u8; 4];
    let copy_len = n_bytes.min(4);
    buf[..copy_len].copy_from_slice(&data[pos + 1..pos + 1 + copy_len]);
    let centiseconds = u32::from_le_bytes(buf) as u64;
    let s = centiseconds_to_time_str(centiseconds);
    Ok((FieldValue::Time(Some(s)), end))
}

// ── Date/Time Helpers ───────────────────────────────────────────────

/// Convert an OLE day serial number to "YYYY-MM-DD" string.
fn day_serial_to_date_str(day_serial: i64) -> String {
    // OLE epoch is 1899-12-30. Convert to Unix days:
    // Unix day 0 = 1970-01-01, OLE day 0 = 1899-12-30
    // OLE day_serial N → Unix day = N - OLE_EPOCH_OFFSET
    let unix_days = day_serial - OLE_EPOCH_OFFSET;
    let (y, m, d) = days_to_civil(unix_days as i32);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Convert a packed u48 (day_serial << 24 | centiseconds) to "YYYY-MM-DD HH:MM:SS".
fn datetime_packed_to_str(raw: u64) -> String {
    let centiseconds = raw & 0xFFFFFF;
    let day_serial = ((raw >> 24) & 0xFFFFFF) as i64;

    let unix_days = day_serial - OLE_EPOCH_OFFSET;
    let (y, m, d) = days_to_civil(unix_days as i32);

    let total_seconds = centiseconds / 100;
    let h = total_seconds / 3600;
    let min = (total_seconds % 3600) / 60;
    let s = total_seconds % 60;

    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, h, min, s)
}

/// Convert centiseconds since midnight to "HH:MM:SS".
fn centiseconds_to_time_str(cs: u64) -> String {
    let total_seconds = cs / 100;
    let h = total_seconds / 3600;
    let min = (total_seconds % 3600) / 60;
    let s = total_seconds % 60;
    format!("{:02}:{:02}:{:02}", h, min, s)
}

/// Convert Unix days to (year,month,day) using Hinnant's algorithm.
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
    fn decode_bool_values() {
        assert_eq!(
            decode_bool(&[0x14], 0).unwrap(),
            (FieldValue::Bool(Some(false)), 1)
        );
        assert_eq!(
            decode_bool(&[0x15], 0).unwrap(),
            (FieldValue::Bool(Some(true)), 1)
        );
        assert_eq!(
            decode_bool(&[NULL_BOOL], 0).unwrap(),
            (FieldValue::Bool(None), 1)
        );
    }

    #[test]
    fn decode_int32_zero() {
        // prefix 0x06 = base, 0 value bytes → value = 0
        assert_eq!(
            decode_compact_int(&[0x06], 0, 6, NULL_INT32, IntTarget::Int32).unwrap(),
            (FieldValue::Int32(Some(0)), 1)
        );
    }

    #[test]
    fn decode_int32_one_byte() {
        // prefix 0x07 = 1 byte, value = 42
        assert_eq!(
            decode_compact_int(&[0x07, 42], 0, 6, NULL_INT32, IntTarget::Int32).unwrap(),
            (FieldValue::Int32(Some(42)), 2)
        );
    }

    #[test]
    fn decode_int32_two_bytes() {
        // prefix 0x08 = 2 bytes, value = 9999 = 0x270F
        assert_eq!(
            decode_compact_int(&[0x08, 0x0F, 0x27], 0, 6, NULL_INT32, IntTarget::Int32).unwrap(),
            (FieldValue::Int32(Some(9999)), 3)
        );
    }

    #[test]
    fn decode_int32_null_below_base() {
        assert_eq!(
            decode_compact_int(&[0x05], 0, 6, NULL_INT32, IntTarget::Int32).unwrap(),
            (FieldValue::Int32(None), 1)
        );
    }

    #[test]
    fn decode_int32_null_type_specific() {
        assert_eq!(
            decode_compact_int(&[NULL_INT32], 0, 6, NULL_INT32, IntTarget::Int32).unwrap(),
            (FieldValue::Int32(None), 1)
        );
    }

    #[test]
    fn decode_double_zero() {
        assert_eq!(
            decode_double(&[0x06], 0).unwrap(),
            (FieldValue::Double(Some(0.0)), 1)
        );
    }

    #[test]
    fn decode_double_full() {
        // prefix 0x0C = 8 bytes, value = 1.0
        let mut data = vec![0x0C];
        data.extend_from_slice(&1.0f64.to_le_bytes());
        let (val, end) = decode_double(&data, 0).unwrap();
        assert_eq!(val, FieldValue::Double(Some(1.0)));
        assert_eq!(end, 9);
    }

    #[test]
    fn decode_double_null() {
        assert_eq!(
            decode_double(&[NULL_DOUBLE], 0).unwrap(),
            (FieldValue::Double(None), 1)
        );
        assert_eq!(
            decode_double(&[NULL_DOUBLE_ALT], 0).unwrap(),
            (FieldValue::Double(None), 1)
        );
    }

    #[test]
    fn decode_string_short() {
        // 0x83 = 0x80 | 3, followed by "AND"
        let data = [0x83, b'A', b'N', b'D'];
        let (val, end) = decode_string(&data, 0, NULL_VSTRING).unwrap();
        assert_eq!(val, FieldValue::String(Some("AND".to_string())));
        assert_eq!(end, 4);
    }

    #[test]
    fn decode_string_null() {
        assert_eq!(
            decode_string(&[0x00], 0, NULL_VSTRING).unwrap(),
            (FieldValue::String(None), 1)
        );
        assert_eq!(
            decode_string(&[NULL_VSTRING], 0, NULL_VSTRING).unwrap(),
            (FieldValue::String(None), 1)
        );
    }

    #[test]
    fn decode_string_long() {
        // 0x01 + u16 LE len + data
        let text = "A".repeat(200);
        let mut data = vec![0x01];
        data.extend_from_slice(&200u16.to_le_bytes());
        data.extend_from_slice(text.as_bytes());
        let (val, end) = decode_string(&data, 0, NULL_VSTRING).unwrap();
        assert_eq!(val, FieldValue::String(Some(text)));
        assert_eq!(end, 3 + 200);
    }

    #[test]
    fn decode_date_serial() {
        // 2016-09-05 = day serial 42618 = 0xA67A
        // prefix 0x0D = 3 bytes, LE bytes: 7A A6 00
        let data = [0x0D, 0x7A, 0xA6, 0x00];
        let (val, end) = decode_date(&data, 0).unwrap();
        assert_eq!(val, FieldValue::Date(Some("2016-09-05".to_string())));
        assert_eq!(end, 4);
    }

    #[test]
    fn decode_datetime_example() {
        // 2022-12-17 21:52:09 → day serial 44912 = 0x00AF70
        // centisecond 7872900 = 0x782184
        // packed = 0x00AF70782184
        // LE bytes = 84 21 78 70 AF 00
        let data = [0x0E, 0x84, 0x21, 0x78, 0x70, 0xAF, 0x00];
        let (val, end) = decode_datetime(&data, 0).unwrap();
        assert_eq!(
            val,
            FieldValue::DateTime(Some("2022-12-17 21:52:09".to_string()))
        );
        assert_eq!(end, 7);
    }

    #[test]
    fn decode_float_example() {
        // Full 4-byte float: prefix 0x0B = base 7 + 4 bytes
        let data = [0x0B, 0xEC, 0xF1, 0x33, 0x44];
        let (val, end) = decode_float(&data, 0).unwrap();
        let expected = f32::from_le_bytes([0xEC, 0xF1, 0x33, 0x44]);
        match val {
            FieldValue::Float(Some(v)) => assert_eq!(v, expected),
            other => panic!("expected float, got {other:?}"),
        }
        assert_eq!(end, 5);
    }

    #[test]
    fn decode_float_null() {
        assert_eq!(
            decode_float(&[NULL_FLOAT], 0).unwrap(),
            (FieldValue::Float(None), 1)
        );
    }

    #[test]
    fn day_serial_to_date_known_values() {
        // OLE day serial 1 = 1899-12-31
        assert_eq!(day_serial_to_date_str(1), "1899-12-31");
        // OLE day serial 2 = 1900-01-01
        assert_eq!(day_serial_to_date_str(2), "1900-01-01");
        // 2016-09-05 = OLE day 42618
        assert_eq!(day_serial_to_date_str(42618), "2016-09-05");
    }
}
