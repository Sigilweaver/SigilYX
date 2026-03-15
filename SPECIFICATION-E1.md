# YXDB E1 File Format Specification

*Specification derived from open-source YXDB implementations; implementation is original.*

> **Scope:** This specification covers the **E1** (original/legacy engine) YXDB format — the format produced by Alteryx Designer's original engine prior to AMP. See [SPECIFICATION-E2.md](SPECIFICATION-E2.md) for the AMP/E2 format.

---

## Overview

YXDB (Alteryx Database) is a binary file format for storing tabular data with embedded metadata. It uses LZF compression for record blocks and supports 17 distinct field types.

### File Structure

```
┌─────────────────────────────────────────┐
│           Header (512 bytes)            │
├─────────────────────────────────────────┤
│       XML Metadata (variable size)      │
├─────────────────────────────────────────┤
│         Record Block 1 (LZF)            │
├─────────────────────────────────────────┤
│         Record Block 2 (LZF)            │
├─────────────────────────────────────────┤
│              ...                        │
├─────────────────────────────────────────┤
│         Record Block N (LZF)            │
└─────────────────────────────────────────┘
```

---

## Header (512 bytes)

The header is a fixed 512-byte structure at the start of the file.

| Offset | Size | Type | Description |
|--------|------|------|-------------|
| 0 | 21 | ASCII | Magic string: `"Alteryx Database File"` (null-padded to 21 bytes) |
| 21 | 43 | — | Reserved / padding |
| 64 | 4 | u32 LE | **File ID**: `0x00440204` (no spatial index) or `0x00440205` (with spatial index) |
| 68 | 12 | — | Reserved |
| 80 | 4 | u32 LE | **Metadata size**: number of UTF-16 code units in the XML metadata (including null terminator). Byte length = value × 2. |
| 84 | 4 | — | Reserved |
| 88 | 8 | i64 LE | **Spatial index position**: file offset of the spatial index (0 if none). Only meaningful when File ID = `0x00440205`. |
| 96 | 8 | i64 LE | **Record block index position**: file offset of the RecordBlockIndex (marks the end of compressed block data). |
| 104 | 8 | u64 LE | **Record count**: total number of records in the file. |
| 112 | 4 | i32 LE | **Compression version**: `0` = uncompressed (no block framing), `1` = LZF compression with block framing. |
| 116 | 396 | — | Reserved / padding (null bytes) |

**Key fields:**
- Bytes 64–67: File ID determines whether a spatial index is present
- Bytes 80–83: Metadata size (in UTF-16 code units, multiply by 2 for byte length)
- Bytes 104–111: Record count as little-endian unsigned 64-bit integer
- Bytes 112–115: Compression version

---

## XML Metadata

Immediately following the 512-byte header is XML metadata encoded in **UTF-16LE** (Little Endian).

### Structure

```xml
<?xml version="1.0" encoding="UTF-16"?>
<RecordInfo>
  <Field name="FieldName1" type="Int32" />
  <Field name="FieldName2" type="V_WString" size="1073741823" />
  <Field name="FieldName3" type="FixedDecimal" size="19" scale="6" />
  <!-- ... more fields ... -->
</RecordInfo>
```

### Field Attributes

| Attribute | Required | Description |
|-----------|----------|-------------|
| `name` | Yes | Field name (column name) |
| `type` | Yes | One of the 17 field types (see below) |
| `size` | Depends | Maximum size/width for strings, precision for decimals |
| `scale` | Depends | Decimal places for `FixedDecimal` |
| `source` | No | Source system description (ignored) |
| `description` | No | Field description (ignored) |

---

## Field Types

YXDB supports 17 field types:

| Type Name | Category | Fixed Size | Description |
|-----------|----------|------------|-------------|
| `Boolean` | Boolean | 1 byte | 0=false, 1=true, 2=null |
| `Byte` | Integer | 2 bytes | 1 byte value + 1 null indicator |
| `Int16` | Integer | 3 bytes | 2 byte signed LE + 1 null indicator |
| `Int32` | Integer | 5 bytes | 4 byte signed LE + 1 null indicator |
| `Int64` | Integer | 9 bytes | 8 byte signed LE + 1 null indicator |
| `Float` | Float | 5 bytes | 4 byte IEEE 754 + 1 null indicator |
| `Double` | Float | 9 bytes | 8 byte IEEE 754 + 1 null indicator |
| `FixedDecimal` | Decimal | size+1 bytes | ASCII decimal string + null indicator |
| `String` | Fixed String | size+1 bytes | ASCII/Latin-1 + null terminator |
| `WString` | Fixed String | size×2+1 bytes | UTF-16LE + null indicator |
| `V_String` | Variable | 4 bytes fixed | Variable-length ASCII/Latin-1 |
| `V_WString` | Variable | 4 bytes fixed | Variable-length UTF-16LE |
| `Date` | Date/Time | 11 bytes | `"YYYY-MM-DD"` ASCII + null indicator |
| `DateTime` | Date/Time | 20 bytes | `"YYYY-MM-DD HH:MM:SS"` ASCII + null indicator |
| `Time` | Date/Time | 9 bytes | `"HH:MM:SS"` ASCII + null indicator |
| `Blob` | Variable | 4 bytes fixed | Variable-length binary data |
| `SpatialObj` | Variable | 4 bytes fixed | Variable-length spatial geometry |

### Null Indicators

Most field types include a trailing null indicator byte:
- `0x00` = value is valid
- `0x01` = value is null

For `Boolean`, the single byte encodes both value and null:
- `0x00` = false
- `0x01` = true  
- `0x02` = null

---

## Record Layout

### Fixed-Size Portion

Each record begins with a fixed-size portion containing:
1. All fixed-size fields in order
2. For variable-length fields: a 4-byte offset marker

The fixed size is calculated as the sum of all field sizes (including null indicators).

### Variable-Length Portion

Variable-length data follows the fixed portion, with each variable field's data stored contiguously.

#### Variable Field Offset (4 bytes)

In the fixed portion, variable fields store a 4-byte offset:
- If the **high bit (bit 31) is set**, the field contains data
- The **lower 31 bits** contain the byte offset from the start of the variable portion

#### Variable Data Encoding

Variable-length data uses a size-prefixed format:

**Small blocks (size ≤ 127 bytes):**
```
┌──────────┬─────────────────┐
│ 1 byte   │ N bytes         │
│ size     │ data            │
└──────────┴─────────────────┘
```

**Normal blocks (size > 127 bytes):**
```
┌──────────┬─────────────────┐
│ 4 bytes  │ N bytes         │
│ size|0x80000000 │ data     │
└──────────┴─────────────────┘
```

The 4-byte size has bit 31 set to distinguish from 1-byte headers.

---

## LZF Compression

Record blocks are compressed using the LZF algorithm.

### Block Structure

```
┌─────────────────────────────────────────┐
│ Compressed size (4 bytes, u32 LE)       │
│ (0x00000000 = uncompressed block)       │
├─────────────────────────────────────────┤
│ Compressed data OR uncompressed records │
└─────────────────────────────────────────┘
```

If `compressed_size == 0`, the block is stored uncompressed.

### LZF Algorithm

LZF is a fast, lightweight compression algorithm using:
- **Literal runs**: Copy bytes directly
- **Back-references**: Reference previously seen data

#### Byte encoding:

**Literal run (control byte < 32):**
```
Control byte: 0x00-0x1F
Meaning: Copy (control + 1) literal bytes from input
```

**Back-reference (control byte ≥ 32):**
```
Control byte: 0x20-0xFF
High 3 bits: Length encoding
Low 5 bits: High bits of offset

Next byte(s): Offset low bits

Length = (control >> 5) + 2
  - If length == 9 (0xE0-0xFF), read another byte and add to length

Offset = ((control & 0x1F) << 8) | next_byte
  - If offset high bits are 0x1F, offset spans 2 bytes
```

---

## Block Boundaries

Records are packed into blocks up to **0x40000 bytes (262,144 bytes)** of uncompressed data. When a block would exceed this limit, a new block is started.

Each block is independently compressed and can be decompressed without prior blocks.

---

## Writing YXDB Files

### Step 1: Write Header Placeholder

Write 512 bytes of zeros (or partial header). The record count at bytes 104-111 will be updated at the end.

### Step 2: Write XML Metadata

1. Build XML in UTF-16LE encoding
2. Write immediately after header

### Step 3: Write Record Blocks

1. Build records into a buffer
2. When buffer reaches ~0x40000 bytes:
   - Attempt LZF compression
   - If compressed is smaller, write: `[compressed_size:u32][compressed_data]`
   - If not, write: `[0x00000000:u32][uncompressed_data]`
3. Track total record count

### Step 4: Finalize Header

1. Seek to byte 104
2. Write final record count as i64 LE
3. Seek to byte 112
4. Write metadata size as u32 LE

---

## References

The following open-source projects were used as references:

- **[Alteryx/OpenYXDB](https://github.com/alteryx/OpenYXDB)** — C++ implementation by Alteryx
- **[NedHarding/Open_AlteryxYXDB](https://github.com/AlteryxNed/Open_AlteryxYXDB)** — C++ implementation (GPL-3.0)
- **[yxdb-go](https://github.com/tlarsendataguy-yxdb/yxdb-go)** — Go implementation by @tlarsendataguy (MIT License)
- **[yxdb-py](https://github.com/tlarsendataguy-yxdb/yxdb-py)** — Python implementation by @tlarsendataguy (MIT License)
- **[yxdb-java](https://github.com/tlarsendataguy-yxdb/yxdb-java)** — Java implementation by @tlarsendataguy (MIT License)
- **[yxdb-net](https://github.com/tlarsendataguy-yxdb/yxdb-net)** — .NET implementation by @tlarsendataguy (MIT License)
- **[yxdb-odbc](https://github.com/tlarsendataguy-yxdb/yxdb-odbc)** — ODBC driver by @tlarsendataguy

---

*This specification is provided "as-is" for interoperability purposes.*
