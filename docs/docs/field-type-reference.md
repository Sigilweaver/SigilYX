---
sidebar_position: 12
description: "Complete reference for all 17 YXDB field types, their binary encoding, and Polars/Arrow representations."
---

# Field Type Reference

Complete reference for all 17 YXDB field types, their binary encoding, and how SigilYX represents them.

## Boolean

| Property | Value |
| --- | --- |
| Category | Boolean |
| Fixed size | 1 byte |
| Encoding | `0x00` = false, `0x01` = true, `0x02` = null |
| Arrow type | `BooleanArray` |
| Polars type | `Boolean` |

No separate null indicator byte -- the value byte encodes both value and nullability.

## Byte

| Property | Value |
| --- | --- |
| Category | Integer |
| Fixed size | 2 bytes |
| Encoding | 1 byte unsigned value + 1 byte null indicator |
| Arrow type | `Int16Array` |
| Polars type | `Int16` |

Stored as `Int16` (not `UInt8`) to accommodate the full 0--255 range without ambiguity.

## Int16

| Property | Value |
| --- | --- |
| Category | Integer |
| Fixed size | 3 bytes |
| Encoding | 2 bytes signed LE + 1 byte null indicator |
| Arrow type | `Int16Array` |
| Polars type | `Int16` |

## Int32

| Property | Value |
| --- | --- |
| Category | Integer |
| Fixed size | 5 bytes |
| Encoding | 4 bytes signed LE + 1 byte null indicator |
| Arrow type | `Int32Array` |
| Polars type | `Int32` |

## Int64

| Property | Value |
| --- | --- |
| Category | Integer |
| Fixed size | 9 bytes |
| Encoding | 8 bytes signed LE + 1 byte null indicator |
| Arrow type | `Int64Array` |
| Polars type | `Int64` |

## Float

| Property | Value |
| --- | --- |
| Category | Float |
| Fixed size | 5 bytes |
| Encoding | 4 bytes IEEE 754 single-precision + 1 byte null indicator |
| Arrow type | `Float32Array` |
| Polars type | `Float32` |

## Double

| Property | Value |
| --- | --- |
| Category | Float |
| Fixed size | 9 bytes |
| Encoding | 8 bytes IEEE 754 double-precision + 1 byte null indicator |
| Arrow type | `Float64Array` |
| Polars type | `Float64` |

## FixedDecimal

| Property | Value |
| --- | --- |
| Category | Decimal |
| Fixed size | `size + 1` bytes |
| Encoding | ASCII decimal string (e.g., `"12345.67"`), null-padded, + 1 byte null indicator |
| Arrow type | `Decimal128Array` |
| Polars type | `Decimal(precision, scale)` |

The `size` attribute specifies the precision (total digits). The `scale` attribute specifies the number of decimal places.

## String

| Property | Value |
| --- | --- |
| Category | Fixed string |
| Fixed size | `size + 1` bytes |
| Encoding | ASCII / Latin-1, null-terminated, + 1 byte null indicator |
| Arrow type | `Utf8Array` |
| Polars type | `String` |

## WString

| Property | Value |
| --- | --- |
| Category | Fixed string |
| Fixed size | `size * 2 + 1` bytes |
| Encoding | UTF-16LE, null-terminated, + 1 byte null indicator |
| Arrow type | `Utf8Array` |
| Polars type | `String` |

SigilYX decodes UTF-16LE to UTF-8 on read, with an SSE2-accelerated fast path for ASCII content.

## V_String

| Property | Value |
| --- | --- |
| Category | Variable string |
| Fixed portion | 4 bytes (offset marker) |
| Encoding | ASCII / Latin-1, variable length |
| Arrow type | `LargeUtf8Array` |
| Polars type | `String` |

## V_WString

| Property | Value |
| --- | --- |
| Category | Variable string |
| Fixed portion | 4 bytes (offset marker) |
| Encoding | UTF-16LE, variable length |
| Arrow type | `LargeUtf8Array` |
| Polars type | `String` |

The most common string type for modern YXDB files. SigilYX's SIMD UTF-16 transcoding makes this type particularly fast to read.

## Date

| Property | Value |
| --- | --- |
| Category | Temporal |
| Fixed size | 11 bytes |
| Encoding | `YYYY-MM-DD` ASCII (10 chars) + 1 byte null indicator |
| Arrow type | `Date32Array` |
| Polars type | `Date` |

Stored as days since Unix epoch in the Arrow representation.

## DateTime

| Property | Value |
| --- | --- |
| Category | Temporal |
| Fixed size | 20 bytes |
| Encoding | `YYYY-MM-DD HH:MM:SS` ASCII (19 chars) + 1 byte null indicator |
| Arrow type | `TimestampArray(Microsecond)` |
| Polars type | `Datetime(Microsecond)` |

## Time

| Property | Value |
| --- | --- |
| Category | Temporal |
| Fixed size | 9 bytes |
| Encoding | `HH:MM:SS` ASCII (8 chars) + 1 byte null indicator |
| Arrow type | `Time64Array(Nanosecond)` |
| Polars type | `Time` |

## Blob

| Property | Value |
| --- | --- |
| Category | Binary |
| Fixed portion | 4 bytes (offset marker) |
| Encoding | Variable-length raw bytes |
| Arrow type | `LargeBinaryArray` |
| Polars type | `Binary` |

## SpatialObj

| Property | Value |
| --- | --- |
| Category | Binary |
| Fixed portion | 4 bytes (offset marker) |
| Encoding | Variable-length Alteryx SHP geometry |
| Arrow type | `LargeBinaryArray` |
| Polars type | `Binary` |

SigilYX can decode the proprietary SHP format to standard ISO Well-Known Binary (WKB) using `SpatialMode::Wkb`. This makes the data compatible with PostGIS, GDAL, Shapely, GeoPandas, and other geospatial tools.

## Null Handling

Most types use a trailing null indicator byte: `0x00` = valid, `0x01` = null. The exceptions are:

- **Boolean**: Uses `0x02` from the single value byte
- **Variable-length types**: A zero offset (bit 31 clear) indicates null/empty
