---
sidebar_position: 4
---

# Field Types

YXDB supports 17 field types. This page documents the Rust API for working with them. For the complete binary encoding reference, see the [Field Type Reference](/field-type-reference).

## Summary

| YXDB Type | Category | Rust/Arrow Type | Polars Type |
| --- | --- | --- | --- |
| `Boolean` | Boolean | `BooleanArray` | `Boolean` |
| `Byte` | Integer | `Int16Array` | `Int16` |
| `Int16` | Integer | `Int16Array` | `Int16` |
| `Int32` | Integer | `Int32Array` | `Int32` |
| `Int64` | Integer | `Int64Array` | `Int64` |
| `Float` | Float | `Float32Array` | `Float32` |
| `Double` | Float | `Float64Array` | `Float64` |
| `FixedDecimal` | Decimal | `Decimal128Array` | `Decimal` |
| `String` | Fixed string | `Utf8Array` | `String` |
| `WString` | Fixed string | `Utf8Array` | `String` |
| `V_String` | Variable string | `LargeUtf8Array` | `String` |
| `V_WString` | Variable string | `LargeUtf8Array` | `String` |
| `Date` | Temporal | `Date32Array` | `Date` |
| `DateTime` | Temporal | `TimestampArray(us)` | `Datetime(us)` |
| `Time` | Temporal | `Time64Array(ns)` | `Time` |
| `Blob` | Binary | `LargeBinaryArray` | `Binary` |
| `SpatialObj` | Binary | `LargeBinaryArray` | `Binary` |

## The `FieldType` Enum

```rust
pub enum FieldType {
    Boolean,
    Byte,
    Int16,
    Int32,
    Int64,
    Float,
    Double,
    FixedDecimal,
    String,
    WString,
    V_String,
    V_WString,
    Date,
    DateTime,
    Time,
    Blob,
    SpatialObj,
}
```

## The `FieldMeta` Struct

```rust
pub struct FieldMeta {
    pub name: String,
    pub field_type: FieldType,
    pub size: u32,
    pub scale: u32,
}
```

- `size` - Width for fixed-size fields, maximum length for variable-length fields. For `FixedDecimal`, this is the precision.
- `scale` - Number of decimal places. Only meaningful for `FixedDecimal`.

## Type Details

### Boolean

Binary representation: 1 byte, where `0x00` = false, `0x01` = true, `0x02` = null.

### Integer Types (Byte, Int16, Int32, Int64)

Little-endian signed integers followed by a 1-byte null indicator (`0x00` = valid, `0x01` = null). `Byte` is an unsigned 8-bit value stored in an `Int16` to accommodate the full 0-255 range.

### Float Types (Float, Double)

IEEE 754 floating-point values (4 or 8 bytes) followed by a 1-byte null indicator.

### FixedDecimal

ASCII string representation of a decimal number (e.g., `"12345.67"`), null-padded to `size` bytes, followed by a null indicator. SigilYX parses this into a Polars `Decimal` column preserving the original precision and scale.

### String Types

| Type | Encoding | Length |
| --- | --- | --- |
| `String` | ASCII / Latin-1 | Fixed `size` bytes + null indicator |
| `WString` | UTF-16LE | Fixed `size * 2` bytes + null indicator |
| `V_String` | ASCII / Latin-1 | Variable length (4-byte offset in fixed portion) |
| `V_WString` | UTF-16LE | Variable length (4-byte offset in fixed portion) |

Fixed strings are padded with null bytes. Variable strings use the offset/length encoding described in the [specification](/specification).

SigilYX converts all strings to UTF-8 on read. For `WString` and `V_WString`, an SSE2-accelerated fast path handles the common ASCII-subset case.

### Temporal Types

| Type | Format | Arrow Type |
| --- | --- | --- |
| `Date` | `YYYY-MM-DD` (10 chars) | `Date32` (days since epoch) |
| `DateTime` | `YYYY-MM-DD HH:MM:SS` (19 chars) | `Timestamp(Microsecond)` |
| `Time` | `HH:MM:SS` (8 chars) | `Time64(Nanosecond)` |

Temporal values are stored as ASCII strings in the YXDB file and parsed to native Arrow temporal types on read.

### Binary Types (Blob, SpatialObj)

Variable-length binary data using the same offset/length encoding as variable strings. `SpatialObj` contains Alteryx SHP geometry data, which SigilYX can optionally decode to ISO WKB format.
