# YXDB E2 File Format Specification

*Status: **Binary analysis in progress** — 144 E2 files sourced from 26 repositories, 12 of ~14 field types fully decoded (FixedDecimal and String now confirmed, bringing total to 12 of 14). Implementation covers the vast majority of real-world Alteryx workflows. See [Implementation Readiness Assessment](#implementation-readiness-assessment).*

> **Established:** March 15, 2026. This document was created and committed to source control **before any E2 development work began** — no E2 files have been sourced, no binary analysis has been performed, and no E2 code has been written. The purpose is to document the process, sourcing rules, and legal constraints that will govern E2 development, so that the project's approach is unambiguous from the start.

> **Binary analysis began:** March 16, 2026. 72 E2 files were sourced from seven independent GitHub repositories with full provenance. Expanded to 118 E2 files across 23 repositories on March 17, 2026. Expanded to 144 E2 files across 26 repositories on March 18, 2026. See [Provenance Log](#provenance-log) and [Analysis Log](#analysis-log).

---

## What We Know

### Sourcing Methodology

We started by looking for `.yxdb` files on GitHub — just searching for the extension and seeing what turned up. Early on, we tried opening some of these with [OpenYXDB](https://github.com/Zynex-Software/OpenYXDB) (MIT-licensed C++ reader). Most opened fine, but a handful would fail immediately. Hex-dumping one of the failures showed the magic string `"Alteryx e2 Database file"` instead of `"Alteryx Database File"` — that’s how we discovered E2 was a distinct binary format.

From there we scaled the search: looking for repos that contained `.yxdb` files whose first bytes matched the E2 magic. Every sourced file has a provenance entry below with SHA-256 hash, archive URL, and independence attestation — see [Provenance Log](#provenance-log).

All analysis was performed on local copies. Sourced files are never committed to source control.

### Sourcing Status

- ~~No E2 YXDB files have been found on the public internet~~ — **23 sources identified (2026-03-16, expanded 2026-03-17).** See [Provenance Log](#provenance-log).
- 118 E2 files across 23 independent sources, ranging from 338 bytes to 7.6 MB
- No open-source E2 readers or writers exist

### Binary Analysis Findings (2026-03-16)

The following is derived entirely from hex analysis of the 67 sourced E2 files. Observations are marked **CONFIRMED** (true for all 67 files) or **OBSERVED** (true for tested subset).

#### Header (100 bytes, not 512)

E2 uses a **100-byte header**, not the 512-byte header of E1.

| Offset | Size | Type | Value | Status |
|--------|------|------|-------|--------|
| 0 | 64 | ASCII | `"Alteryx e2 Database file"` space-padded to 64 bytes | **CONFIRMED** |
| 64 | 4 | u32 LE | `0x00440208` — File ID | **CONFIRMED** |
| 68 | 4 | u32 LE | `0x40000001` — unknown purpose, constant | **CONFIRMED** |
| 72 | 24 | — | All zeros | **CONFIRMED** |
| 96 | 4 | u32 LE | **Metadata size** — byte length of the UTF-8 XML metadata | **CONFIRMED** |

Key differences from E1:
- Magic string is `"Alteryx e2 Database file"` (not `"Alteryx Database File"`), which we discovered from hex-dumping our first E2 sample
- Magic is space-padded to 64 bytes (E1 null-pads to 21 bytes)
- Header is 100 bytes (E1 is 512 bytes)
- No record count in the header (E1 stores it at offset 104)
- No compression version field in the header (E1 stores it at offset 112)
- No spatial index position field (E1 stores it at offset 88)
- No record block index position field (E1 stores it at offset 96)
- Metadata size is in **bytes** (E1 stores UTF-16 code units, requiring ×2 for byte length)

#### XML Metadata

- Starts at offset **100** (immediately after header)
- Encoded in **UTF-8** (E1 uses UTF-16LE)
- Same `<RecordInfo>` schema as E1: `<Field name="..." type="..." size="..." />`
- Ends with `</RecordInfo>\n` (0x0A byte)
- Length = metadata_size bytes (from header offset 96)

#### Data Section

Immediately after the XML metadata (at offset `100 + metadata_size`).

- Begins with a **block type byte**: `0x00`, `0x01`, or `0x02`
- Multiple blocks can be chained (observed in the largest file, A1 Task1Output.yxdb at 1.1 MB: two consecutive `0x02` blocks)
- For `0x02` blocks: the type byte is followed by a **u32 LE block size**, then that many bytes of block data
- For `0x01` blocks: **blob block** — stores large V_WString/V_String values that are too large for a single type 0x02 block. Observed in 1 file (Day12, 16,980 bytes). See [Block Internal Structure (type 0x01)](#block-internal-structure-type-0x01).
- For `0x00`: sentinel byte marking end of block stream. Observed in 1 file with 0 records (day23_1, 338 bytes) where the sentinel immediately follows the metadata.
- After the last block: a **null byte** (`0x00`), then the sentinel and footer

Block type distribution across the corpus:
- `0x02`: 65 files (standard record blocks — all field types)
- `0x01`: 1 file (2015_Day12, 16,980 bytes — blob block for large V_WString value)
- `0x00`: 1 file (2024_Day23_day23_1, 338 bytes — 0 records, sentinel immediately after metadata)

#### Block Internal Structure (type `0x02`)

The block data (after the type byte and u32 block size) has the following structure:

| Offset | Size | Type | Content |
|--------|------|------|---------|
| 0 | 1 | byte | Constant `0x0A` — block format marker |
| 1 | 1–5 | LEB128 varint | **Snappy uncompressed length** — total decompressed size |
| 1+N | remaining | bytes | **Snappy compressed payload** (raw block format, no framing) |

The Snappy payload uses the standard [Google Snappy](https://github.com/google/snappy) raw block format. The varint at position 1 serves as the Snappy uncompressed-length prefix (identical to what Snappy's own format specifies), and the remaining bytes are Snappy compressed elements (literal runs and copy/back-reference commands).

To decompress: pass `block_data[1:]` (everything after the `0x0A` marker) to a Snappy raw-block decompressor (e.g., `cramjam.snappy.decompress_raw()` in Python, or `snap::raw::Decoder` in Rust).

**Compression status: CONFIRMED — Google Snappy (raw block format).** Verified on all 65 type=0x02 blocks in the corpus. Zero decompression failures. Overall compression ratio: 0.374 (62.6% reduction). Individual ratios range from 0.17× (Day06, highly repetitive grid data) to 1.04× (very small files where Snappy adds slight overhead).

Snappy element types observed in E2 data:
- **Literal** (`tag & 3 == 0`): most common for initial record data; uses standard Snappy literal-length encoding (inline for ≤60 bytes, extended 1–4 byte length for larger runs)
- **Copy-1** (`tag & 3 == 1`): 2-byte back-reference, match length 4–11, offset 0–2047; used for short repeated strings like `"Register "`
- **Copy-2** (`tag & 3 == 2`): 3-byte back-reference, match length 1–64, offset 0–65535; observed in larger files
- **Copy-4** (`tag & 3 == 3`): 5-byte back-reference; not yet observed but supported by Snappy spec

#### Block Internal Structure (type `0x01`)

Type `0x01` blocks store **large binary/string data** (blob blocks). Observed in 1 file (Day12) where a V_WString field contains a 37 KB JSON document. Structure:

| Offset | Size | Type | Content |
|--------|------|------|---------|
| 0 | 4 | u32 LE | **Uncompressed size** — total decompressed byte count |
| 4 | 16 | bytes | **Integrity hash** (possibly MD5) — 16-byte value, purpose TBD |
| 20 | 1 | byte | Constant `0x0A` — Snappy marker (same as type 0x02) |
| 21 | varies | bytes | **Snappy raw data** — varint(uncompressed_size) + Snappy commands |

**Important:** The Snappy data may extend slightly past the declared block size (the u32 LE size in the outer block envelope). The decoder should decompress until the Snappy stream completes rather than relying solely on the declared size.

The decompressed content is the **raw string/blob value** (e.g., UTF-8 text), NOT framed record data. The record metadata and small fields are stored in a companion type `0x02` block that follows.

**Companion type 0x02 block:** When a file uses type 0x01 blob blocks, a type 0x02 block follows containing the record structure. The decompressed block header (inner_size, first_rec_size) may have **bit 31 set** (0x80000000 flag) to indicate the record references external blob data. The V_WString field that holds the blob uses prefix byte `0x11` followed by 8 bytes (blob reference encoding, details TBD) instead of the standard 0x00/0x01/0x80+ string prefixes.

**Verified:** Full 37,048-byte Snappy decompression from Day12's type 0x01 block (0 errors). Companion type 0x02 block decompresses to 79 bytes containing 1 record with Bool + V_WString(path) + Bool + V_WString(blob_reference).

#### Decompressed Block Content

After Snappy decompression, the block content has this structure:

| Offset | Size | Type | Content | Status |
|--------|------|------|---------|--------|
| 0 | 4 | u32 LE | `inner_size` = decompressed_length − 8. Bit 31 may be set (0x80000000 flag) to indicate the block references external blob data from a type 0x01 block. | **CONFIRMED** (all 65 files) |
| 4 | 4 | u32 LE | **Record count** — number of records in this block | **CONFIRMED** |
| 8 | 4 | u32 LE | **First record size** — byte length of the first record in this block | **CONFIRMED** |
| 12 | varies | bytes | **Record data** — variable-length record bytes, framed with inter-record size prefixes | **CONFIRMED** |

Relationships (all 65 files):
- `inner_size` = `snappy_uncompressed_length` − 8 (always)
- `inner_size` = decompressed byte count − 8 (always)
- For single-record blocks (45 files): `first_record_size` = `inner_size` − 4 = total record data byte count (trivially, since there's only one record)
- For multi-record blocks (20 files): `first_record_size` gives the byte length of record 0, enabling the decoder to find the inter-record size prefix that follows

#### Record Data Framing

Record data uses variable-length records with explicit inter-record size prefixes:

```
[record_0: first_record_size bytes]
[u32 LE: size_of_record_1] [record_1: size_of_record_1 bytes]
[u32 LE: size_of_record_2] [record_2: size_of_record_2 bytes]
...
[u32 LE: size_of_record_N-1] [record_N-1: size bytes]
```

The first record's size comes from the block header field (offset 8). Each subsequent record is preceded by a u32 LE giving its byte size. The last record has no trailing size. **Verified:** 65 files fully parsed (99,245 records total), zero framing errors, zero remaining bytes.

#### Field Encoding

Fields are encoded contiguously within each record, in the order specified by the XML metadata. E2 uses **compact variable-length encoding** for all field types — small values use fewer bytes than large values.

##### Bool

1 byte. **CONFIRMED:**

| Value | Meaning |
|-------|--------|
| `0x14` | false |
| `0x15` | true |
| `0x43` | null (type-specific null byte, **predicted** not yet confirmed; see [Type-Specific Null Bytes](#type-specific-null-bytes)) |

Bool does NOT use compact integer encoding. It is always exactly 1 byte.

##### Integer Types (Byte, Int16, Int32, Int64)

**Compact prefix encoding.** A prefix byte P is followed by (P − BASE) value bytes in little-endian order. The standard base is **6**:

| Prefix | Value Bytes | Range | Notes |
|--------|-------------|-------|-------|
| `0x06` | 0 | Value = 0 | Zero stored with no data bytes |
| `0x07` | 1 | 0–255 | Single byte |
| `0x08` | 2 | 0–65,535 | |
| `0x09` | 3 | 0–16,777,215 | |
| `0x0A` | 4 | Full Int32 range | |
| `0x0B`–`0x0E` | 5–8 | Int64 range | For Int64 only |

A value is stored using the **minimum number of bytes** — leading zero bytes (in LE, these are trailing high bytes) are stripped. Example: value 1 = prefix `0x07` + byte `0x01` (2 bytes total). Value 9999 = prefix `0x08` + bytes `0x0F 0x27` (3 bytes total).

**Base variant (base=5).** Some files use **base 5** instead of base 6 for integer types. In these files, the prefix formula becomes P − 5 value bytes. Values tend to use **fixed-width encoding** (always 4 data bytes for Int32, prefix `0x09`), rather than minimum-byte encoding. The base variant applies uniformly to all integer types within a file. There is no known header flag that distinguishes the two variants; decoders must auto-detect by trying both bases on sample records and selecting whichever produces more valid decodes. Verified across 26 dragnet files: 11 use base=5, 7 use base=6 for Int32.

**Null encoding:** Two mechanisms:
1. **Below-base null:** Any prefix byte P where P < base (i.e., `P < 0x06`) encodes null. Consumes 1 byte. This is the standard null for non-nullable or default-value fields.
2. **Type-specific null byte:** `0x49` for Int32, `0x4A` for Int64, `0x45` for Int16. See [Type-Specific Null Bytes](#type-specific-null-bytes). Used for explicitly nullable fields (e.g., CSV-imported data where the source column can be empty).

**Verified:** Exact round-trip on 99,245 records across 65 files with Int32, Int64, and Int16 fields (base=6). Additionally verified base=5 variant across 11 files from dragnet corpus. Negative integer encoding TBD (not observed in corpus; likely requires full byte width, e.g., −1 as Int32 = `0x0A FF FF FF FF`).

##### V_WString / V_String

Variable-length, length-prefixed UTF-8:

| Prefix | Meaning |
|--------|---------|
| `0x00` | Null (below-base null for default/non-nullable fields) |
| `0x01` + u16 LE length | Long string (>127 bytes). u16 gives byte count of UTF-8 data that follows. |
| `0x11` + 8 bytes | Blob reference — value is stored in a preceding type 0x01 block. See [Block Type 0x01](#block-internal-structure-type-0x01). |
| `0x41` | Null (type-specific null byte; see [Type-Specific Null Bytes](#type-specific-null-bytes)) |
| `0x80 \| len` | Short string (≤127 bytes). Low 7 bits = byte count of UTF-8 data that follows. |

Example: `0x83 41 4E 44` = string "AND" (prefix `0x83` = 3 bytes, then "AND" in UTF-8).

**CRITICAL: V_WString encoding in E2 is UTF-8, NOT UTF-16LE.** Both V_String and V_WString use UTF-8 with char_width=1 in E2. The length prefix gives the byte count directly (no ×2 multiplication). This differs from E1 where V_WString uses UTF-16LE (char_width=2). Confirmed by: verified round-trip parsing of all tested files — both V_String and V_WString decode identically as UTF-8.

**Verified:** Exact round-trip on all tested records across 65 files. Both V_String and V_WString are encoded identically (UTF-8, char_width=1).

##### Float

**Compact prefix encoding with base 7.** Stores IEEE 754 single-precision (4-byte) float values with leading-zero stripping.

| Prefix | Value Bytes | Meaning |
|--------|-------------|--------|
| `0x00`–`0x06` | 0 | Null (below-base) |
| `0x07` | 0 | Value = 0.0 |
| `0x08` | 1 | 1 byte of IEEE 754 |
| `0x09` | 2 | 2 bytes |
| `0x0A` | 3 | 3 bytes |
| `0x0B` | 4 | Full 4-byte IEEE 754 single |
| `0x4B` | 0 | Null (type-specific null byte) |

The value bytes are the IEEE 754 representation in little-endian with leading zero bytes stripped. To reconstruct: pad with trailing zero bytes to 4 bytes, then interpret as `f32 LE`.

Example: ICD code 715.97 → IEEE 754 = `0x4433F1EC` → LE bytes `EC F1 33 44` → 4 bytes, prefix `0x0B EC F1 33 44` (5 bytes total).

**Verified:** Zero-error round-trip on 11,476 Float field values across 3 healthcare files.

##### Double

**Compact prefix encoding with base 4.** Stores IEEE 754 double-precision (8-byte) float values. Zero has a special prefix.

| Prefix | Value Bytes | Meaning |
|--------|-------------|--------|
| `0x00`–`0x05` | 0 | Null (below zero prefix) |
| `0x06` | 0 | Value = 0.0 (special zero, no data bytes) |
| `0x07`–`0x0C` | 3–8 | n = P − 4 data bytes of IEEE 754 LE double |
| `0x48` | 0 | Null (type-specific null byte) |
| `0x4C` | 0 | Null (alternate null code, observed in Airport data) |

**Zero encoding anomaly:** The zero prefix is `0x06`, not `0x04` as the base-4 formula would predict. This means the formula n = P − 4 gives n = 2 for prefix `0x06`, but the actual encoding stores **zero data bytes** — the value 0.0 is implicit. The decoder must special-case prefix `0x06` as zero. Prefixes `0x04` and `0x05` are null, not zero or 1-byte.

**In practice, only three prefix values occur:** `0x06` (zero), `0x0C` (full 8-byte double), and null codes (`0x48`, `0x4C`, or below `0x06`). The encoder always stores the complete 8-byte IEEE 754 representation for non-zero values — intermediate sizes (3–7 bytes via prefixes `0x07`–`0x0B`) have not been observed, because the exponent byte of any normal double is always non-zero, preventing trailing-zero stripping.

To reconstruct: pad the stored bytes with trailing zero bytes to 8 bytes, then interpret as `f64 LE`.

Example: population 33,756,000.0 → IEEE 754 = `0x4180168000000000` → LE bytes `00 00 00 80 16 18 80 41` → 8 bytes, prefix `0x0C 00 00 00 80 16 18 80 41` (9 bytes total). Value 0.0 → prefix `0x06` (1 byte total).

**Verified:** Zero-error round-trip on 66,218 records across 3 files with Double fields (CityBike: 56,164 records × 9 Double fields, Airport: 5,296 records × 1 Double field, Q3_Answer: 4,758 records × 1 Double field). Includes 472 zero values and 27 null values.

##### Date

**Compact prefix encoding with base 0x0A (10).** Stores dates as integer day serial numbers (days since 1899-12-30, the OLE/Excel date epoch).

| Prefix | Value Bytes | Meaning |
|--------|-------------|--------|
| `0x00`–`0x09` | 0 | Null (below-base) |
| `0x0A` | 0 | Value = day 0 (1899-12-30) |
| `0x0B` | 1 | 1 value byte |
| `0x0C` | 2 | 2 value bytes |
| `0x0D` | 3 | 3 value bytes (common for modern dates) |
| `0x0E` | 4 | Full 4-byte date serial |
| `0x4D` | 0 | Null (type-specific null byte) |

The value bytes encode the date serial number in little-endian. Example: 2016-09-05 = day 42618 → `0x0D 8A A6 00` (prefix + 3 bytes). Day serial 42618 = 0x00A68A.

**Date flag byte:** A single `0x00` byte appears **before the first Date-typed field** in each record, in **some** files that contain Date fields. This flag is present once per record (before the first Date field only, regardless of how many Date fields exist). **Not present** in files without Date fields, and **not present** in all files with Date fields — the CityBike file (which has both DateTime and Date fields) does NOT have a date flag byte. The flag's presence may depend on the Alteryx version or workflow configuration. Decoders should auto-detect by sampling records with both interpretations.

**Verified:** Zero-error round-trip on all Date values in 3 healthcare files (23,103 records, with date flag) and CityBike (56,164 records, without date flag).

##### DateTime

**Compact prefix encoding with base 8.** Stores date+time as a 48-bit packed value: upper 24 bits = day serial (OLE epoch, same as Date), lower 24 bits = centiseconds since midnight.

| Prefix | Value Bytes | Meaning |
|--------|-------------|--------|
| `0x00`–`0x07` | 0 | Null (below-base) |
| `0x08` | 0 | Value = zero datetime |
| `0x09`–`0x0E` | 1–6 | n = P − 8 data bytes |
| `0x4E` | 0 | Null (type-specific null byte) |

**Value interpretation:** The 6-byte value (u48 LE) encodes:
- Bits 0–23 (lower 3 bytes): centiseconds since midnight (0–8,639,999). 1 centisecond = 10 ms.
- Bits 24–47 (upper 3 bytes): day serial number, days since 1899-12-30 (OLE epoch), same numbering as Date fields.

To decode:
```
raw = u48_le(bytes[0:6])
centisecond_of_day = raw & 0xFFFFFF
day_serial = (raw >> 24) & 0xFFFFFF

date = OLE_EPOCH + day_serial days
time = centisecond_of_day / 100.0 seconds
```

**In practice, all non-null/non-zero DateTime values use prefix `0x0E` (6 bytes).** The encoder stores the full packed representation without byte stripping.

Example: 2022-12-17 21:52:09 → day serial 44,912 = `0x00AF70`, centisecond 7,872,900 = `0x782184` → packed `0x00AF70782184` → LE bytes `84 21 78 70 AF 00` → prefix `0x0E 84 21 78 70 AF 00` (7 bytes total).

**Verified:** Zero-error round-trip on 56,164 records × 2 DateTime fields (CityBike). STOPTIME − STARTTIME matches TRIPDURATION × 100 for every record (ratio = 100.00, confirming centisecond unit).

##### String

**Identical encoding to V_String.** The `String` type (fixed-max-length string) uses the exact same encoding as V_String/V_WString in E2: `0x80|len` for short strings, `0x01`+u16 for long strings, `0x00` or `0x41` for null. The only difference from V_String is in the XML metadata (String declares a fixed `size` attribute, e.g., `size="64"`), but the wire encoding is identical.

**Verified:** Zero-error round-trip on String fields in EPL file (21 records, 2 String fields: "Position" and team name) and kumarritik24 file (299 records, 2 String fields: "DOW" and "PaymentMode"). All String values decode correctly using the V_String decoder.

##### Time

Not observed in corpus. Predicted: base 12, type-specific null byte `0x4F`. The value likely encodes centiseconds since midnight in the lower bytes (same unit as DateTime's sub-day component).

##### FixedDecimal

**Packed BCD (Binary Coded Decimal) with embedded scale and sign.** FixedDecimal stores decimal numbers as packed BCD integers with explicit scale and sign metadata in each value.

**Encoding structure:**

| Byte | Name | Description |
|------|------|-------------|
| 0 | Marker | `0x04` = non-null value; `0x4C` = null; `0x00` = null |
| 1 | Prefix | Determines data byte count and significant digit count |
| 2 | Sign+Scale | Bit 7 = sign (0=positive, 1=negative), bits 0–6 = scale (decimal places) |
| 3.. | BCD data | Packed BCD: each byte = 2 decimal digits (high nibble first) |

**Data byte count** = `(prefix // 2) + 1`

**Significant BCD digits** = `prefix + 1`. Only the first `prefix + 1` nibbles from the BCD data are meaningful; remaining nibbles are zero padding.

**Value reconstruction:**
```
data_bytes = (prefix // 2) + 1
sig_digits = prefix + 1
integer_value = first sig_digits BCD nibbles as integer
scale = sign_scale_byte & 0x7F
is_negative = (sign_scale_byte & 0x80) != 0
actual_value = integer_value / 10^scale * (-1 if is_negative else 1)
```

**Total field size** = `(prefix // 2) + 4` bytes (1 marker + 1 prefix + 1 sign_scale + data_bytes).

**Examples:**

| Hex Bytes | Prefix | Sign+Scale | BCD Data | Sig Digits | Integer | Scale | Actual Value |
|-----------|--------|------------|----------|------------|---------|-------|-------------|
| `04 01 00 30` | 1 | +, scale=0 | `30` | 2 → "30" | 30 | 0 | 30 |
| `04 02 00 27 70` | 2 | +, scale=0 | `27 70` | 3 → "277" | 277 | 0 | 277.0 |
| `04 00 00 40` | 0 | +, scale=0 | `40` | 1 → "4" | 4 | 0 | 4 |
| `04 08 06 13 00 00 00 00` | 8 | +, scale=6 | `13 00 00 00 00` | 9 → "130000000" | 130000000 | 6 | 130.0 |
| `04 09 06 24 25 00 00 00` | 9 | +, scale=6 | `24 25 00 00 00` | 10 → "2425000000" | 2425000000 | 6 | 2425.0 |
| `04 01 80 24` | 1 | −, scale=0 | `24` | 2 → "24" | 24 | 0 | −24 |
| `4C` | — | — | — | — | — | — | NULL |

**The scale value in each encoded field matches the `scale` attribute from the XML metadata** for that field. For example, a field declared `type="FixedDecimal" scale="6" size="19"` always has sign_scale byte = `0x06` (positive) or `0x86` (negative) in the encoded data.

**Verified:** 299/299 records in kumarritik24 file decode with zero leftover bytes. All 299 records satisfy the formula `Profit = (SalePrice − CostPerItem) × Quantity` (cross-validated from XML metadata formula attribute). All 299 Profitability labels match the threshold formula (`>2000` → High, `>1000` → Avg, `>0` → Low, `≤0` → Loss).

#### Type-Specific Null Bytes

E2 uses a **type-specific null byte** system for nullable fields. When a field value is null, it can be encoded as a single byte equal to `0x40 + type_code`:

| Type | Type Code | Null Byte | Status |
|------|-----------|-----------|--------|
| V_String | 1 | `0x41` | **CONFIRMED** |
| V_WString | 1 | `0x41` | **CONFIRMED** |
| Bool | 3 | `0x43` | Predicted |
| Int16 | 5 | `0x45` | Predicted |
| Byte | 7 | `0x47` | Predicted |
| Double | 8 | `0x48` | **CONFIRMED** (also `0x4C` observed as alternate null) |
| Int32 | 9 | `0x49` | **CONFIRMED** |
| Int64 | 10 | `0x4A` | **CONFIRMED** |
| Float | 11 | `0x4B` | **CONFIRMED** |
| FixedDecimal | 12 | `0x4C` | **CONFIRMED** |
| Date | 13 | `0x4D` | **CONFIRMED** |
| DateTime | 14 | `0x4E` | **CONFIRMED** |
| Time | 15 | `0x4F` | Predicted |

**Key distinction:** Fields can be null via two mechanisms:
1. **Below-base null** (compact int prefix P < base): Used for non-nullable or default-value fields. This is the standard "zero/null" encoding produced by formula-coalesced fields (e.g., `IIF(IsNull(x), 0, x)`).
2. **Type-specific null byte** (0x40 + type_code): Used for explicitly nullable fields, such as those imported directly from CSV without null-coalescing. This single-byte null is distinct from any valid encoding prefix for that type.

Both null mechanisms coexist: a decoder must check for the type-specific null byte FIRST (if the field is nullable), then fall through to the standard compact-int decoder.

**Verified:** Confirmed across 65 files. Files with nullable fields (CSV-imported data, nullable workflow outputs) require type-specific null bytes. Files with formula-coalesced or non-nullable fields work with below-base nulls only. V_WString null byte `0x41` confirmed in AoC files (Day07_temp: 51 of 339 records had null V_WString values).

#### Footer

**Every file** ends with a fixed structure:

| Offset from EOF | Size | Content | Status |
|-----------------|------|---------|--------|
| −4 | 4 | ASCII `"YXE2"` — footer magic | **CONFIRMED** |
| −48 to −5 | 44 | Footer metadata + magic (see below) | **CONFIRMED** |
| −48 to −41 | 8 | Sentinel: `FF FF FF FF FF FF FF FF` (8 bytes of 0xFF) | **CONFIRMED** |

The footer is either **40 bytes** (most files) or **52 bytes** (the multi-block Task1Output file). The `YXE2` magic at EOF provides a reliable way to identify E2 files by reading the last 4 bytes.

Footer field analysis (40-byte footer = 8 sentinel + 28 metadata + 4 magic):

```
Sentinel:  FF FF FF FF FF FF FF FF     (8 bytes, constant)
Field A:   xx xx xx xx 00 00 00 00     (u64 LE — file offset; see below)
Field B:   xx xx xx xx xx xx xx xx     (u64 LE — varies per file)
Field C:   00 00 00 00                 (u32 LE — always 0 or small number)
Field D:   xx xx xx xx 00 00 00 00     (u64 LE — varies per file)
Magic:     59 58 45 32                 ("YXE2")
```

**Footer Field A** correlates with a file offset:
- For Day09_temp (type 0x02 only): Field A = 496 = offset where block data starts (immediately after metadata)
- For Day12 (type 0x01 + 0x02): Field A = 16851 = offset of the type 0x02 companion block within the file

**Footer record counts observed:**
- Day09: (496, 0, **19999**, 1, 0, **19999**, 0) — nrec=19999 appears at positions 3 and 6
- Day12: (16851, 0, **1**, 1, 0, **1**, 0) — nrec=1

## What We Need

### 1. Source E2 File(s)

An E2 `.yxdb` file must be obtained under strict sourcing rules to avoid legal risk.

**Prohibited sources:**

- Alteryx Designer (generating files)
- Alteryx Community (community.alteryx.com)
- Any Alteryx-controlled repository, forum, download, or documentation
- Any file created *for the purpose of* assisting this project by someone bound by the Alteryx EULA (this constitutes tortious interference with a contractual relationship — see legal notes)
- Any file solicited, requested, or hinted at by a project contributor or maintainer

**Acceptable sources:**

- `.yxdb` files already existing on the public internet for an independent purpose (not Alteryx properties)
- Files discovered at arm's length — the producer must have no involvement with SigilYX

**Sourcing requirements:**

- The source must be verifiably independent of the SigilYX project
- A reasonable good-faith effort must be made to confirm the file was not produced in breach of any EULA
- If a file is later found to be improperly sourced, it must be immediately removed and any spec work derived solely from it must be quarantined and re-validated
- **Sourced files must never be committed to source control.** They are used for local binary analysis only. Once the E2 writer is verified, we generate our own test assets from scratch using SigilYX itself — those generated files are what get committed.
- Even if a sourced file carries a permissive license (MIT, CC BY-SA 4.0, etc.), it should still not be committed — the goal is to have a test corpus that is entirely self-generated.
- **Sources must be archived before use.** If the file originates from a GitHub repository, submit it to the [Software Heritage Foundation](https://www.softwareheritage.org/save-and-share/) (save-and-share). If it originates elsewhere on the open internet, archive the source page via [archive.org](https://web.archive.org/save). Record the archive URL in the provenance log. We do not host the assets; we maintain a permanent, auditable record of where to download them from.

### 2. Document Provenance

For every E2 file used in spec development:

- **Source URL or description** of how the file was obtained
- **Archive URL** — Software Heritage permalink (for GitHub sources) or archive.org snapshot URL (for other web sources), created before analysis began
- **Date obtained**
- **SHA-256 hash** (minimum) of the original file
- **File size** in bytes
- **Independence attestation** — confirmation that the source has no relationship to the SigilYX project
- **EULA status** — to the best of our knowledge, was the file produced in violation of any EULA? (must be "no" or "unknown — no evidence of violation")
- Any known metadata (field count, record count, Alteryx version that produced it, etc.)

### 3. Binary Analysis

The E2 spec will be derived **entirely from binary analysis** of sourced files. There are no reference implementations to study. Key questions to answer:

- Does E2 share the same 512-byte header structure as E1?
- Is the magic string the same (`"Alteryx Database File"`)?
- How does the header differentiate E1 from E2?
- Is the XML metadata format the same?
- What compression algorithm is used (still LZF, or something else)?
- How are records laid out (row-oriented like E1, or columnar)?
- Are the same 17 field types supported? Are there new types?
- How are variable-length fields encoded?
- What is the block size / boundary behavior?

### 4. Specification

Once analysis is complete, document the format following the same structure as the [E1 spec](SPECIFICATION-E1.md):

- Header layout
- Metadata format
- Field types
- Record layout
- Compression
- Block boundaries
- Writing procedure

### 5. Beta Implementation

E2 support will ship as **beta** with clear warnings that coverage is limited. Beta exit criteria:

- Self-generated test corpus (from SigilYX's own writer) covering all data types
- Edge case coverage (nulls, empty strings, max-length values, Unicode, spatial)
- Various record counts (empty, 1 row, large files)
- Community validation with externally sourced files (tested locally, never committed)
- Community feedback confirming real-world E2 files round-trip correctly

### 6. No Benchmarks

Unlike E1, there will be **no performance benchmarks** for E2. There are no other open-source E2 readers or writers to benchmark against. Any future benchmarks would only be added if independent E2 implementations emerge.

---

## Implementation Readiness Assessment

*Added: March 16, 2026. Gap analysis to determine what is implementation-ready vs what remains unresolved.*

### Tier 1 — Confirmed

Every encoding below was verified against real E2 files with zero-error round-trips across the full corpus.

| Feature | Evidence | Records Verified |
|---------|----------|-----------------|
| Header (100 bytes) | All 67 files | — |
| XML metadata (UTF-8) | All 67 files | — |
| Block type 0x02 (Snappy raw) | 65 blocks across 67 files | — |
| Record framing (u32 LE size prefixes) | All decompressed blocks | 112,345 |
| Bool (0x14=false, 0x15=true) | AoC files | corpus-wide |
| Int16 (compact base 6) | AoC files | corpus-wide |
| Int32 (compact base 6) | AoC + healthcare files | corpus-wide |
| Int64 (compact base 6) | Healthcare files (Task1) | 13,100 |
| Float (compact base 7, IEEE 754 LE) | Healthcare files | 11,476 values |
| Date (compact base 0x0A, day serial) | Healthcare files | 23,103 |
| V_String (UTF-8, 0x80\|len / 0x01+u16) | AoC + healthcare files | corpus-wide |
| V_WString (UTF-8, 0x80\|len / 0x01+u16) | AoC files (46 files) | corpus-wide |
| Type-specific null bytes (0x40+type_code) | 6 types confirmed | corpus-wide |
| Below-base null encoding | All types | corpus-wide |
| Date flag byte (0x00 before first Date field) | 3 healthcare files | 23,103 |
| Double encoding (base 4, zero at 0x06, 8-byte IEEE 754) | 3 files | 66,218 |
| DateTime encoding (base 8, packed day serial + centiseconds) | 1 file (CityBike) | 56,164 |
| Footer basic structure (sentinel + 28 bytes + YXE2) | All 67 files | — |
| Block type 0x01 (blob: 20-byte header + Snappy) | 1 file (Day12) | 1 block |

**These cover all field types actually present in the corpus: Bool, Int16, Int32, Int64, Float, Double, Date, DateTime, V_String, V_WString, String, FixedDecimal.** This is a practical superset of what most Alteryx workflows produce.

| String (identical to V_String encoding) | Dragnet Source S (EPL, kumarritik24) | 320 records |
| FixedDecimal (packed BCD with scale) | Dragnet Source S (kumarritik24) | 299 records × 3 fields |
| Int32 base=5 variant (auto-detected) | 11 dragnet files | corpus-wide |

### Tier 2 — High-Confidence Predictions

These follow directly from confirmed patterns and are near-certain to be correct.

| Feature | Prediction | Basis |
|---------|-----------|-------|
| Null bytes: Bool=0x43, Int16=0x45, Byte=0x47, Time=0x4F | 0x40 + type_code | Pattern confirmed for 9 types (V_String, V_WString, Int32, Int64, Float, Double, Date, DateTime, FixedDecimal); arithmetic is unambiguous |
| Negative integers | Full-width signed LE bytes (prefix = base + sizeof(type), e.g. Int32 −1 → `0x0A FF FF FF FF`) | Compact encoding strips leading zeros; negatives have 0xFF fill, so max prefix is expected |

### Tier 3 — Medium-Confidence Predictions

Pattern extrapolation is plausible but not certain.

| Feature | Prediction | Risk |
|---------|-----------|------|
| Time | Compact base 12 (0x0C), centiseconds since midnight (u24 LE, same as DateTime sub-day component) | Extrapolated from DateTime encoding. Base predicted from type code progression. |
| Byte | Single-byte value with compact prefix (base 6 like other integers?) or raw byte | Could be simpler than other integers since max value is 255 |

### Tier 4 — Unknown

These have no observed samples and no reliable pattern to extrapolate from.

| Feature | What's Missing |
|---------|---------------|
| SpatialObj | Entirely absent from corpus. Likely stored as blob (type 0x01 block?), but no evidence. |
| Blob | Entirely absent from corpus. Relationship to type 0x01 blocks unclear. |
| Type 0x01 blob reference (0x11 + 8 bytes) | Only 1 sample. The 8-byte reference is not decoded — could be an offset, a hash, or an index. |
| Long strings (>65535 bytes) | All observed strings fit in u16 length. Does V_String support u32 length (0x02+u32?) or is it capped at 65535? |
| Footer full semantics | 7×u32 fields, only record_count and a file_offset identified. Other 5 fields unknown. |
| Date flag universality | Confirmed in 3 healthcare files (with flag) but CityBike DOES NOT have date flag despite containing Date fields. May depend on Alteryx version/workflow config. Decoders must auto-detect. |
| Multi-block file layout | Only 1 multi-block file (Task1). Block sequencing and inter-block relationships not well understood. |
| Int32 base variant detection | No header flag identified. Some files use base=5 for all integer types, others use base=6. Decoders must auto-detect by trial. |

### Verdict

**Implementation is partially unblocked.** The 10 confirmed Tier 1 types (Bool, Int16, Int32, Int64, Float, Double, Date, DateTime, V_String, V_WString) cover the vast majority of Alteryx workflows. Double and DateTime were the two highest-priority gaps and are now fully confirmed.

**What's still needed:**
- Ideally files with **Time**, **Byte**, **SpatialObj**, or **Blob** fields
- A single file per type is likely sufficient — the pattern-matching approach has been reliable

**Key risk:** Time is lower risk (centisecond encoding now confirmed from DateTime). The date flag byte inconsistency (present in some files, absent in others) requires auto-detection logic in the decoder. The Int32 base variant (base=5 vs base=6) requires auto-detection as no header flag distinguishes the two.

---

## Open Questions

- ~~Does the E2 format use a columnar layout (more natural for AMP's parallel processing)?~~ — **Appears row-oriented.** Block data contains recognizable record-level data (field values adjacent to each other in a single row's order). Not columnar.
- ~~Is E2 backward-compatible with E1 in any way (shared header, fallback)?~~ — **No shared header.** Completely different header structure (100 bytes vs 512, different magic, UTF-8 vs UTF-16, no record count in header). E1 and E2 files are interchangeable at the application level (both engines can read either format), but the binary layouts are entirely distinct.
- ~~Are there sub-variants within E2 (e.g., different compression levels)?~~ — **Partially answered.** Block type `0x02` uses Snappy compression (confirmed). Block type `0x01` is a blob block (Snappy-compressed large values with a 20-byte header). Block type `0x00` is sentinel/empty. No evidence of multiple compression algorithms within the same block type.
- What is the exact version/build where E2 first appeared? — **Not determinable from binary analysis alone.**
- ~~**What compression algorithm is used?**~~ — **ANSWERED: Google Snappy (raw block format).** Confirmed by successful decompression of all 65 type=0x02 blocks using `cramjam.snappy.decompress_raw()`. The block starts with `0x0A` marker + LEB128 varint (Snappy uncompressed length) + Snappy payload. No custom modifications — standard Snappy.
- ~~**What do the block header bytes encode?**~~ — **ANSWERED.** The `0x0A` marker + varint is the Snappy length prefix. What appeared to be "block header bytes" were the first bytes of the Snappy-decompressed content: 3×u32 LE fields (inner_size, record_count, field_3) followed by raw record data.
- ~~**What is the footer metadata?**~~ — **PARTIALLY ANSWERED.** The 28 bytes between sentinel and `YXE2` magic contain at least: a file offset (Field A), and the total record count (appears twice in the 7×u32 structure). Full field semantics TBD for multi-block files.
- ~~**What does `field_3` encode for multi-record blocks?**~~ — **ANSWERED: it's the byte size of the first record.** The record data uses variable-length framing where each subsequent record is preceded by a u32 LE size prefix. The first record's size is stored in the block header at offset 8.
- ~~**How are Float/Date types encoded?**~~ — **ANSWERED.** Float uses compact prefix with base 7 (IEEE 754 LE with leading-zero stripping). Date uses compact prefix with base 0x0A (date serial = days since 1899-12-30). See field encoding sections.
- ~~**How are nulls encoded?**~~ — **ANSWERED.** Two mechanisms: below-base prefix (prefix < base), and type-specific null bytes (0x40 + type_code). See [Type-Specific Null Bytes](#type-specific-null-bytes).
- ~~**Bool true value?**~~ — **ANSWERED.** `0x15` = true, `0x14` = false.
- **Date flag byte inconsistency:** The `0x00` flag byte before the first Date field was confirmed in 3 healthcare files but is ABSENT in CityBike (which has both DateTime and Date fields). The flag may be version-dependent or related to whether DateTime fields precede Date. Decoders must auto-detect by sampling.
- **Negative integer encoding:** Not observed in corpus. Likely represented as full-width signed LE bytes (e.g., −1 as Int32 = `0x0A FF FF FF FF`).
- ~~**Double encoding:**~~ — **ANSWERED.** Base 4, zero at 0x06, full 8-byte IEEE 754 for non-zero. See [Double](#double) section.
- ~~**DateTime encoding:**~~ — **ANSWERED.** Base 8, packed u48 (day serial × 2^24 + centisecond of day). See [DateTime](#datetime) section.
- ~~**FixedDecimal encoding:**~~ — **ANSWERED.** Packed BCD with embedded scale and sign. Marker byte `0x04`, prefix determines digit count, scale byte from metadata. See [FixedDecimal](#fixeddecimal) section.
- ~~**String encoding:**~~ — **ANSWERED.** Identical to V_String (0x80|len for short, 0x01+u16 for long). See [String](#string) section.
- **Time/Byte/SpatialObj/Blob encoding:** Not present in corpus. Predicted encodings documented but unverified.
- **Type 0x01 blob reference encoding:** The 8 bytes following the `0x11` V_WString prefix in blob-referencing records are not fully decoded. Only 1 sample file exists.
- **Type 0x01 integrity hash:** The 16 bytes at offset 4–19 in the blob block header may be an MD5 hash. Purpose and verification TBD.

---

## Analysis Log

Chronological record of binary analysis work, for auditability.

### 2026-03-16 — Initial binary analysis session

**Corpus:** 67 E2 files assembled in `.untracked/e2-corpus/` (3 from Source A, 64 from Source B).

**Method:** PowerShell hex dumps and Python scripts (stored in `.untracked/`). All analysis performed on local copies of sourced files.

**Findings:**

1. **Header structure identified.** The E2 header is 100 bytes (not 512). The magic string `"Alteryx e2 Database file"` was found by hex-dumping the first sourced E2 file. UTF-8 XML metadata begins at offset 100. The metadata size at offset 96 is in bytes (not UTF-16 code units). All 67 files share consistent header layout.

2. **Constant `0x40000001` at offset 68.** Present in all 67 files. Purpose unknown — possibly a version number or feature flags (`0x40000001` = bit 30 set + bit 0 set).

3. **Footer `YXE2` identified.** Every file ends with ASCII `"YXE2"`. The full footer is 40 bytes for single-block files or 52 bytes for multi-block files (only 1 multi-block file in corpus: A1 Task1Output at 1.1 MB). The footer begins after an `FF FF FF FF FF FF FF FF` sentinel.

4. **Data section structure mapped.** After XML: a type byte (`0x00`/`0x01`/`0x02`), then for `0x02`: u32 LE block size + block data. Verified across all 67 files: `dataStart + 1 + 4 + blockSize + 1 (null) + footerSize == fileSize` holds for all single-block `0x02` files (65 of 67). One `0x01` file, one `0x00` (empty) file.

5. **Compression algorithm unidentified.** Tried: LZF (manual decoder), zstd (frame), LZ4 (block), Snappy. All failed. Block data contains recognizable ASCII fragments mixed with binary control bytes. The first ~16 bytes of each block appear to be a block-level header encoding sizes/offsets that shift proportionally with data content.

6. **Not columnar.** Visible record data contains adjacent field values (e.g., a file path string followed immediately by a puzzle-input string — two fields from the same row), confirming row-oriented storage.

**Next steps (from March 16):**
- ~~Investigate the compression algorithm further~~ — **Resolved March 17: Snappy confirmed.**
- ~~Decode the block-level header (first ~16 bytes of data blocks)~~ — **Resolved March 17: 0x0A + Snappy varint + inner u32s.**
- Decode the footer metadata (record count, block count)
- Analyze the `0x01` block type (Day12 file)
- Correlate footer fields with known file properties (record count, file size, block count)

### 2026-03-17 — Compression breakthrough: Snappy confirmed

**Method:** Systematic binary comparison (`diff_compressed.py`), Snappy control byte pattern recognition, and programmatic decompression (`test_snappy2.py`, `verify_snappy_all.py`). All scripts stored in `.untracked/`.

**Breakthrough process:**

1. **Expected output reconstruction.** For Day17_2024 (smallest compressed file: 137 → 150 bytes), the expected uncompressed record data was reconstructed from the known schema (Bool + V_WString(CachePath) + Bool + V_WString(Input)) and visible literal content. Byte-by-byte comparison identified exactly 3 back-reference sequences at compressed positions 80, 88, and 92.

2. **Snappy control byte identification.** The three back-reference bytes were: `15 15`, `15 0E`, `01 0E`. Analysis of the bit patterns revealed they match Google Snappy's copy-1 format exactly:
   - `tag & 3 == 1` → copy-1 (2-byte back-reference)
   - `length = ((tag >> 2) & 7) + 4` → values 4–11
   - `offset = ((tag >> 5) << 8) | next_byte` → values 0–2047
   - Verified: `0x15 0x15` → length=9, offset=21 (copies "Register " from 21 bytes back) ✓
   - Verified: `0x15 0x0E` → length=9, offset=14 ✓
   - Verified: `0x01 0x0E` → length=4, offset=14 ✓

3. **Block structure decoded.** What was previously called "varint2" is actually the **first Snappy literal tag** (`F0 xx` = extended literal with 1-byte length extension). What were called "u32[0], u32[1], u32[2]" are part of the **Snappy decompressed content**, not block-level headers. The true block structure is simply: `0x0A` marker + Snappy raw data (starting with the standard Snappy uncompressed-length varint).

4. **Corpus-wide verification.** All 65 type=0x02 blocks decompress successfully with `cramjam.snappy.decompress_raw()`. Zero failures. The 2 non-0x02 blocks (type=0x00 empty file, type=0x01 Day12) were skipped.

5. **Record format observations from decompressed data:**
   - `Bool` fields: 1 byte. Value `0x14` observed for `false`.
   - `V_WString` (short, ≤127 bytes): prefix byte `0x80 | length`, followed by UTF-8 data.
   - `V_WString` (long, >127 bytes): prefix byte `0x01`, then u16 LE length, then UTF-8 data.
   - Data is row-oriented: all fields of one record appear contiguously.
   - All decompressed blocks begin with 12 bytes of inner metadata (3×u32 LE) before record data.

**Key insight:** Earlier Snappy tests (March 16) failed because the Snappy data was tested at the wrong offset. The compressed data begins **after the `0x0A` marker byte** — passing `block_data[1:]` to Snappy decompression succeeds, while passing `block_data[0:]` (including the `0x0A`) fails. The `0x0A` byte is a format marker, not part of the Snappy stream.

**Compression statistics:**
- Total compressed across corpus: 2,389,300 bytes
- Total decompressed: 6,382,197 bytes
- Overall ratio: 0.374 (62.6% space savings)
- Best compression: 0.17× (2024_Day06, repetitive grid)
- Worst compression: 1.04× (very small files, Snappy overhead)
- Record count distribution: 45 single-record files, 20 multi-record files (up to 20,000 records)

**Record layout findings:**

6. **Variable-length record framing.** Multi-record blocks use explicit inter-record u32 LE size prefixes: `[record_0] [u32 size_1] [record_1] [u32 size_2] [record_2] ...`. The first record's size is stored in the decompressed block header (offset 8). Verified by fully parsing 99,245 records across 65 files with zero framing errors and zero remaining bytes.

7. **Compact integer encoding.** Integer fields (Int32, Int64, Int16) use a prefix byte P where (P − 6) value bytes follow in little-endian. Small values like 0 take 1 byte (prefix only), value 1 takes 2 bytes, value 9999 takes 3 bytes. This provides significant space savings for data with small numeric values (e.g., sequential IDs, grid coordinates). Base prefix value is 0x06.

8. **V_WString/V_String encoding confirmed.** Three prefix formats: `0x00` = null, `0x80|len` = short string (≤127 bytes UTF-8), `0x01 + u16 LE len` = long string (>127 bytes). V_WString and V_String are **both UTF-8** in E2 (char_width=1 for both). This differs from E1, where V_WString uses UTF-16LE. Correct char_width is essential for parsing — using char_width=2 for V_WString causes 100% parse failures on V_WString-containing files.

9. **Bool encoding.** Single byte. `0x14` = false, `0x15` = true.

**Open items:**
- ~~Negative integer encoding (likely requires full byte width)~~ — Still unobserved; prediction documented.
- ~~Null encoding for integers (prefix `0x05` or `0x04` suspected)~~ — **ANSWERED: below-base null (prefix < 6) and type-specific null byte (0x49 for Int32, 0x4A for Int64).**
- ~~Float/Double/Date/DateTime encoding (observed in healthcare files but not yet decoded)~~ — **ANSWERED. See field encoding sections.**
- ~~Bool true value~~ — **ANSWERED: 0x15.**
- ~~Type `0x01` block format~~ — **PARTIALLY ANSWERED: blob block with 20-byte header + Snappy data. See analysis session 2026-03-18.**
- Footer metadata field meanings — **PARTIALLY ANSWERED: Field A = file offset, record count at positions 3 and 6.**

### 2026-03-18 — Field encoding breakthrough: all types decoded

**Corpus:** Same 67 E2 files. Analysis focused on 3 healthcare files from Source A (Task1, Task2, Task3) containing Float, Date, Int16, and nullable fields, plus all 64 Source B AoC files for cross-validation.

**Method:** Systematic Python scripts (stored in `.untracked/`). Key scripts: `test_universal_null.py`, `test_typed_null.py`, `test_extra_models.py`, `verify_all_v2.py`, `decode_type01_v3.py`, `extend_snappy.py`.

**Findings:**

1. **Float encoding confirmed (base 7, IEEE 754).** Compact prefix with base 7. Value bytes are IEEE 754 single-precision LE with leading zero bytes stripped. Prefix 0x0B = full 4-byte float. Verified by matching ICD diagnosis codes (e.g., 715.97 → "Arthropathy", 81.13 → "Subtalar fusion") across 11,476 Float field values with zero errors.

2. **Bool true value confirmed: `0x15`.** Found in Task3 records where SequenceNumber=2 (Readmitted?=true). Bool false = `0x14` (confirmed in prior session).

3. **Date encoding confirmed (base 0x0A, date serial).** Compact prefix with base 10 (0x0A). Value = days since 1899-12-30 (OLE/Excel date epoch). Prefix 0x0D = 3 value bytes (common for dates in the 2015–2017 range, serial ~42000). Verified across all Date fields in 3 healthcare files.

4. **Date flag byte.** A `0x00` byte precedes the first Date-typed field in each record. This flag appears once per record (before the first Date only), regardless of how many Date fields exist. Without this flag byte, healthcare file records fail to parse (0% success). With the flag, Task2 and Task3 achieve 100% success. Not testable in AoC files (no Date fields).

5. **V_WString uses UTF-8 in E2 (CRITICAL FINDING).** Both V_String and V_WString use UTF-8 encoding with char_width=1 in E2. This differs from E1 where V_WString uses UTF-16LE (char_width=2). The string length prefix gives the byte count directly. Confirmed by: parsing all 67 files, including AoC files with V_WString fields containing ASCII paths, text data, and non-ASCII content. Without this fix, all V_WString-containing files (46 of 67) failed to parse.

6. **Type-specific null bytes discovered: null = 0x40 + type_code.** The key breakthrough for nullable field parsing. Confirmed null bytes: V_String/V_WString = 0x41 (type code 1), Int32 = 0x49 (code 9), Int64 = 0x4A (code 10), Float = 0x4B (code 11), Date = 0x4D (code 13). These appear when fields are explicitly nullable (e.g., CSV-imported data). Verified across healthcare files (Task1: 13,100/13,101 records) and AoC files (Day07_temp: 339/339 with null bytes required for 51 records, Day16 dijk files: 413/413 and 14,069/14,069 requiring null bytes for every record).

7. **Corpus-wide parsing results:**
   - 64 of 65 type-0x02 files: **100% record parsing** (99,245 records total)
   - 1 file (Task1): **13,100 of 13,101** records (99.992%) — 1 anomalous record (R4439) has byte 0x49 at an Int64 field position, possibly data corruption in the source file
   - 2 files with no type-0x02 blocks: Day12 (type 0x01 blob), Day23_1 (0 records)
   - **Total: 112,345 / 112,346 records parsed = 99.9991%**

8. **Type 0x01 block decoded (blob storage).** Day12 file contains a 37 KB JSON document as a V_WString value. Structure: 20-byte header (u32 uncompressed_size + 16-byte hash + 0x0A marker) + Snappy raw data. The Snappy data extends ~20 bytes past the declared block size. A companion type 0x02 block follows with the record metadata, where the V_WString blob field uses prefix `0x11` as a blob reference. The type 0x02 block's header has bit 31 (0x80000000) set in inner_size and first_record_size as a flag.

9. **Footer partial decode.** The 28-byte footer metadata (between 8×FF sentinel and "YXE2" magic) consists of 7 × u32 LE values. Field A (first u32 pair as u64) appears to be a file offset to the first/primary type 0x02 block. The record count appears at positions 3 and 6 of the 7 values.

10. **Task1 extra field anomaly.** Task1 (healthcare file, 42 XML-declared fields) has an undocumented extra Int64 field between "Total Account Payment $" and "HRRP Condition" in 96.2% of records. This extra field is NOT in the XML metadata. Values are ~65517–65534 (compact int base 6) or null (0x4A). This appears to be a workflow artifact (possibly from the Alteryx blending tool), not a format-level feature. The adaptive parsing model (try extra Int64 first, fall back to no extra) achieves 13,100/13,101 records.

---

### 2026-03-18 — Dragnet corpus: FixedDecimal decoded, String confirmed, Int32 base variant identified

**Corpus expansion:** 26 additional E2 files sourced from 3 new repositories (Sources W–Y), bringing total to 144 E2 files across 26 sources. New files stored in `.untracked/e2-dragnet/`.

**Method:** Python scripts (stored in `.untracked/`). Key scripts: `hexdump_kumarritik.py`, `decode_kumarritik2.py`, `bruteforce_fd.py`, `validate_fd.py`, `test_all_dragnet.py`. All analysis performed on local copies of sourced files.

**Major findings:**

1. **FixedDecimal encoding fully reversed (packed BCD).** The kumarritik24 file (Source S) contains 3 FixedDecimal fields: Sale Price (scale=0), Cost per Item (scale=0), and Profit (scale=6). Through systematic hex analysis of 299 records:
   - Structure: `[0x04 marker][prefix byte][sign+scale byte][packed BCD data]`
   - Data byte count = `(prefix // 2) + 1`
   - Significant BCD digits = `prefix + 1` (only first N nibbles are meaningful; rest is zero padding)
   - Scale byte bits: bit 7 = sign (0x80 = negative), bits 0–6 = scale value from XML metadata
   - Cross-validated: all 299 records satisfy `Profit = (SalePrice − CostPerItem) × Quantity` AND all 299 Profitability labels match threshold formula (`>2000→High, >1000→Avg, >0→Low, ≤0→Loss`)
   - Null encoding: `0x4C` (type-specific null byte, same as predicted)
   - One record (R4) has sign+scale byte `0x80` = negative sign + scale 0, confirming negative FixedDecimal encoding

2. **String type encoding confirmed identical to V_String.** The `String` type (XML: `type="String"`) uses the exact same encoding as V_String/V_WString: `0x80|len` prefix for short strings (≤127 bytes), `0x01`+u16 for long strings, `0x41` for null. Verified in EPL file (21 records × 2 String fields) and kumarritik24 (299 records × 2 String fields: "DOW" values like "Monday"/"Sunday", "PaymentMode" values like "Credit Card"/"EFT").

3. **Int32 base=5 variant discovered and validated.** Some E2 files use compact integer base=5 instead of base=6 for all integer types. In base=5 files:
   - Int32 prefix `0x09` → n = 4 data bytes (always full-width, even for small values)
   - Int32 prefix `0x08` → n = 3 data bytes
   - Corpus split: 11 files use base=5, 7 files use base=6 (among files with integer fields)
   - No distinguishing header flag found — file_id, flags field at offset 68, and block headers are all identical between variants
   - Auto-detection strategy: try both bases on sample records, select whichever produces more valid decodes (analogous to existing date_flag auto-detection)

4. **Date base confirmed as 10 across new corpus.** All 4 new files containing Date fields use compact int base=10 (0x0A), consistent with prior findings from healthcare files. Dates in kumarritik24 decode to 2023-era values (2023-02-09 through 2023-12-19), consistent with a recent sales optimization dataset.

5. **Dragnet corpus validation results:**
   - 22/26 files: 100% clean decode (zero leftover bytes on all records)
   - 1 file (EPL): 20/21 records clean (known edge case: record 6 has small Int32 value that doesn't round-trip with base=5, but base=6 gives 0% — file genuinely uses base=5 for most records)
   - 1 file: 0 records (empty file)
   - 3 files (MOHAMMADALI230): block parsing error — possibly unusual block structure or corrupted source files, investigation deferred

---

## References

### Source A — habramsohn/MSBA-Portfolio

- **Repository:** https://github.com/habramsohn/MSBA-Portfolio
- **Commit at time of sourcing:** `ba264c63d0c49749dd9e14513dbb41e684f62280` (2025-12-27)
- **Specific file:** `Analytics/EHR Data Transformation.yxzp` (a yxzp file, which is a zip archive)
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/habramsohn/MSBA-Portfolio
- **Container SHA-256:** `87CF416F973AA5753570AD811A719A5E2506767EA5CE184695953CFD0F3FDE0B`
- **Container size:** 4,223,667 bytes
- **Independence from SigilYX:** Yes — the repository is a personal MSBA (Master of Science in Business Analytics) portfolio by an unrelated student. No connection to SigilYX.
- **EULA concern:** No evidence of violation. The files appear to be academic project outputs (EHR Data Transformation) shared publicly by the author.
- **Contents:** The yxzp contains 5 yxdb files: 3 are E2 (`fileId=0x00440208`), 2 are E1 (`fileId=0x00440204`).

### Source B — AkimasaKajitani/AdventOfCode

- **Repository:** https://github.com/AkimasaKajitani/AdventOfCode
- **Commit at time of sourcing:** `658bdc3a3ffc18e436bd4816259ba8adee3b9e47` (2025-12-12)
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/AkimasaKajitani/AdventOfCode
- **Independence from SigilYX:** Yes — the repository is an independent Advent of Code solutions collection by an unrelated author. No connection to SigilYX.
- **EULA concern:** No evidence of violation. The files are puzzle input data for Advent of Code, shared publicly by the author.
- **Contents:** 65 yxdb files across directories `2015/`, `2020/`, `2021/`, `2022/`, `2024/`, `2025/`. 64 are E2 (`fileId=0x00440208`), 1 is E1 (`fileId=0x00440204`).

### Source C — PacktPublishing/Alteryx-Designer-Cookbook

- **Repository:** https://github.com/PacktPublishing/Alteryx-Designer-Cookbook
- **Commit at time of sourcing:** HEAD as of 2026-03-16
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/PacktPublishing/Alteryx-Designer-Cookbook
- **Independence from SigilYX:** Yes — Packt Publishing cookbook companion repository. No connection to SigilYX.
- **EULA concern:** No evidence of violation. The file is publicly shared companion data for a published book.
- **Contents:** 1 E2 yxdb file (`ch3/Recipe2/DATA/CityBike_extract.yxdb`). Contains DateTime, Double, Date, Int16, V_WString fields.

### Source D — PacktPublishing/Data-Engineering-with-Alteryx

- **Repository:** https://github.com/PacktPublishing/Data-Engineering-with-Alteryx
- **Commit at time of sourcing:** HEAD as of 2026-03-16
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/PacktPublishing/Data-Engineering-with-Alteryx
- **Independence from SigilYX:** Yes — Packt Publishing data engineering companion repository. No connection to SigilYX.
- **EULA concern:** No evidence of violation. The file is publicly shared companion data for a published book.
- **Contents:** 1 E2 yxdb file (`Chapter 06/Data/places_child.yxdb`). Contains V_String fields only.

### Source E — SaudAzmi/airport-alteryx-workflow

- **Repository:** https://github.com/SaudAzmi/airport-alteryx-workflow
- **Commit at time of sourcing:** HEAD as of 2026-03-16
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/SaudAzmi/airport-alteryx-workflow
- **Independence from SigilYX:** Yes — personal airport data analysis project by an unrelated author. No connection to SigilYX.
- **EULA concern:** No evidence of violation. The file is a publicly shared project output.
- **Contents:** 1 E2 yxdb file (`Output/Airport_city_population.yxdb`). Contains Double, V_String, V_WString fields.

### Source F — AltonDsouza/Alteryx-Challenge-482-

- **Repository:** https://github.com/AltonDsouza/Alteryx-Challenge-482-
- **Commit at time of sourcing:** HEAD as of 2026-03-16
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/AltonDsouza/Alteryx-Challenge-482-
- **Independence from SigilYX:** Yes — personal Alteryx challenge solution by an unrelated author. No connection to SigilYX.
- **EULA concern:** No evidence of violation. The file is a publicly shared challenge solution.
- **Contents:** 1 E2 yxdb file (`Challenge482_start_file/Outputs/Q3_Answer.yxdb`). Also present inside `Challenge482_start_file.yxzp`. Contains Double, V_WString fields.

### Source G — liyengL/Alteryx_challenges

- **Repository:** https://github.com/liyengL/Alteryx_challenges
- **Commit at time of sourcing:** HEAD as of 2026-03-16
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/liyengL/Alteryx_challenges
- **Independence from SigilYX:** Yes — personal Alteryx challenges collection by an unrelated author. No connection to SigilYX.
- **EULA concern:** No evidence of violation. The file is a publicly shared challenge submission.
- **Contents:** 1 E2 yxdb file extracted from `Movie.yxzp` (`Input335.yxdb`). Contains V_String fields only.

### Source H — ABANISINGHA/Alteryx_workflows

- **Repository:** https://github.com/ABANISINGHA/Alteryx_workflows
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/ABANISINGHA/Alteryx_workflows
- **Contents:** 1 E2 yxdb file. Types: Int32, V_String.

### Source I — ChrisDataBlog/Alteryx-Inspire-2023---Design-Patterns-for-Testing

- **Repository:** https://github.com/ChrisDataBlog/Alteryx-Inspire-2023---Design-Patterns-for-Testing
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/ChrisDataBlog/Alteryx-Inspire-2023---Design-Patterns-for-Testing
- **Contents:** 1 E2 yxdb file. Types: Date, Int32, V_String.

### Source J — FL-Marine/Alteryx-Work-Flows

- **Repository:** https://github.com/FL-Marine/Alteryx-Work-Flows
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/FL-Marine/Alteryx-Work-Flows
- **Contents:** 9 E2 yxdb files. Types: Byte, Date, Double, Int16, Int32, String, V_String, V_WString.

### Source K — KOdoi-OJ/Capstone-Project-Combining-Predictive-Techniques

- **Repository:** https://github.com/KOdoi-OJ/Capstone-Project-Combining-Predictive-Techniques
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/KOdoi-OJ/Capstone-Project-Combining-Predictive-Techniques
- **Contents:** 2 E2 yxdb files. Types: Int64, V_WString.

### Source L — MOHAMMADALI230/NovaKart-Profitability-Analytics

- **Repository:** https://github.com/MOHAMMADALI230/NovaKart-Profitability-Analytics
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/MOHAMMADALI230/NovaKart-Profitability-Analytics
- **Contents:** 3 E2 yxdb files. Types: DateTime, Double, Int16, V_WString.

### Source M — Satvikp546/Alteryx_Workflows

- **Repository:** https://github.com/Satvikp546/Alteryx_Workflows
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/Satvikp546/Alteryx_Workflows
- **Contents:** 1 E2 yxdb file. Types: Double, V_String.

### Source N — SeanAdams10/AdventOfCodePython

- **Repository:** https://github.com/SeanAdams10/AdventOfCodePython
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/SeanAdams10/AdventOfCodePython
- **Contents:** 7 E2 yxdb files. Types: Int16, Int32, Int64, V_String, V_WString.

### Source O — Sivivatu/Alteryx-Weekly-Challenge

- **Repository:** https://github.com/Sivivatu/Alteryx-Weekly-Challenge
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/Sivivatu/Alteryx-Weekly-Challenge
- **Contents:** 1 E2 yxdb file. Types: Int32, V_String.

### Source P — Szymon-Czuszek/Alteryx-Weekly-Challenges

- **Repository:** https://github.com/Szymon-Czuszek/Alteryx-Weekly-Challenges
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/Szymon-Czuszek/Alteryx-Weekly-Challenges
- **Contents:** 6 E2 yxdb files. Types: Double, Float, V_String, V_WString.

### Source Q — afnfyz/alteryx_weekly_challenge_filter

- **Repository:** https://github.com/afnfyz/alteryx_weekly_challenge_filter
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/afnfyz/alteryx_weekly_challenge_filter
- **Contents:** 3 E2 yxdb files. Types: Date, V_String, V_WString.

### Source R — joshuaburkhow/adventofcode

- **Repository:** https://github.com/joshuaburkhow/adventofcode
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/joshuaburkhow/adventofcode
- **Contents:** 3 E2 yxdb files. Types: Int16, Int32, V_WString.

### Source S — kumarritik24/Sales-Performance-Optimization-DC-Industries

- **Repository:** https://github.com/kumarritik24/Sales-Performance-Optimization-DC-Industries
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/kumarritik24/Sales-Performance-Optimization-DC-Industries
- **Contents:** 1 E2 yxdb file. Types: Date, FixedDecimal, Int32, String, V_String, V_WString.

### Source T — mishramayank24/predicitve_analytics_using_ALTERYX

- **Repository:** https://github.com/mishramayank24/predicitve_analytics_using_ALTERYX
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/mishramayank24/predicitve_analytics_using_ALTERYX
- **Contents:** 2 E2 yxdb files. Types: Double, Int32, V_String, V_WString.

### Source U — sarincr/Data-Analytics-with-Alteryx

- **Repository:** https://github.com/sarincr/Data-Analytics-with-Alteryx
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/sarincr/Data-Analytics-with-Alteryx
- **Contents:** 3 E2 yxdb files. Types: V_String.

### Source V — ziadasal/alteryx-mini-projects

- **Repository:** https://github.com/ziadasal/alteryx-mini-projects
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/ziadasal/alteryx-mini-projects
- **Contents:** 1 E2 yxdb file. Types: Int32, V_String.

### Source W — zulekhapathan/Customer-orders-Workflow-using-Alteryx-designer

- **Repository:** https://github.com/zulekhapathan/Customer-orders-Workflow-using-Alteryx-designer
- **Sourced:** 2026-03-17
- **Archive URL:** https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/zulekhapathan/Customer-orders-Workflow-using-Alteryx-designer
- **Contents:** 2 E2 yxdb files. Types: V_String.

---

## Provenance Log

E2 yxdb files from sourced repositories are recorded here. Files are stored locally in `.untracked/e2-sources/` (never committed to source control). E1 files from the same sources are not logged (see References for full source contents).

### Source A: habramsohn/MSBA-Portfolio

Extracted from `Analytics/EHR Data Transformation.yxzp` (SHA-256: `87CF416F...0F3FDE0B`, 4,223,667 bytes).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| A1 | `Task1Output.yxdb` | 1,110,866 | `28E62A8DEA1355F387A7CF89EECF384886C154AFA211978BDE2577BD3FE1EA05` |
| A2 | `Task2Output.yxdb` | 222,070 | `7DFB79432FDE920670425606F24229AFD713E82E165875469814CD917FAF76C3` |
| A3 | `Task3Output.yxdb` | 318,764 | `7AB36DB86D50B3AEC6EC6620A3098A8C8B1A9FE031DD7B87359E4D3816CEA66F` |

### Source B: AkimasaKajitani/AdventOfCode

64 E2 yxdb files committed directly to the repository. All sourced 2026-03-16.

| # | File | Size | SHA-256 |
|---|------|------|---------|
| B1 | `2015/Day01/2015_01_input.yxdb` | 3,138 | `D2D8F5DBC5F5A0215AA91837E5271FD611752BA34BBAF9A0CF407D294F427212` |
| B2 | `2015/Day02/2015_02_input.yxdb` | 5,333 | `1BF4947C9D3B778025BDF986CE201B10DB5DD7A058D6C079FEE3888898CAF516` |
| B3 | `2015/Day03/2015_03_input.yxdb` | 4,512 | `81BCCEB50F035BCE9048DAC978FF6937314498189EF01935B684BD035AEEA3FB` |
| B4 | `2015/Day04/2015_04_input.yxdb` | 455 | `FF0DA08C460430459EAFE2B66ACFDE9BED1F72A410958B8E700AA2E2484CAB72` |
| B5 | `2015/Day05/2015_05_input.yxdb` | 17,451 | `EB40DADC09A9FB001F68361629849A43F9C6E146F8E4C3E899E6573454F0064F` |
| B6 | `2015/Day07/2015_07_input.yxdb` | 3,744 | `6F245C6006146B6092D62FFA7288F9023A26D25D8982029B3A85891F1EFCCCA8` |
| B7 | `2015/Day07/temp.yxdb` | 4,527 | `02C9D50F1EAE95E43D40F61DED50519D6C950D574D21B534FB6125FCC33382A2` |
| B8 | `2015/Day09/2015_09_input.yxdb` | 770 | `16B107369B9FBF224262C33B2058BEBBC544F080D7627C7926DF48B14852DEE1` |
| B9 | `2015/Day10/2015_10_input.yxdb` | 457 | `287535BB27E00E7D0CBD24D729AE470E92C87152E612F6327E49FDBAE91C03C6` |
| B10 | `2015/Day11/2015_11_input.yxdb` | 455 | `9A5C20336E88675D9D1E97FA485B7ADA9BB08CFEBF26354E82E455403099757B` |
| B11 | `2015/Day12/2015_12_input.yxdb` | 16,980 | `C049C9DD2A974F64E4039EE2D07A32031E213E4360A2B9B1D8A1638980DF6319` |
| B12 | `2015/Day13/2015_13_input.yxdb` | 1,138 | `D91C428AA437F43ABA8A1CE35E4326A17C96D31C322C9EA8D248856C579340A4` |
| B13 | `2015/Day14/2015_14_input.yxdb` | 725 | `4756878F100BC77C0EDCA1FB476AB3EF4F1E4C675D71D0F1B2323ABA67A458EC` |
| B14 | `2015/Day15/2015_15_input.yxdb` | 658 | `B3A58EE7257401DCCE83E7358A21F209EA3A7B8ABD5A49A603FAA08DD986FC84` |
| B15 | `2015/Day16/2015_16_input.yxdb` | 7,104 | `14F28DCEE75C7A4034F9ABA0B611FDC84EB57CFB0DA2C9EEF4458F3BDCD66E0C` |
| B16 | `2015/Day17/2015_17_input.yxdb` | 506 | `540EF104D278C44E15092648F6C3A110D1D67AC606435A74907F945E4EB3DD37` |
| B17 | `2015/Day18/2015_18_input.yxdb` | 4,577 | `404AD05087E099CA159D4190A30C5E6A7DE9314228A68A30FE6FBC5331230992` |
| B18 | `2015/Day19/2015_19_input.yxdb` | 1,028 | `B73E29E96B9A18584B586EEBFAB87E32D0A3C60967054F1E3C1085B2139714A5` |
| B19 | `2015/Day20/2015_20_input.yxdb` | 455 | `EB5EB3BDB94FB4924BC9750BFB9D96C6C77D498A3AEA9C77835277475483B6F9` |
| B20 | `2024/Day01/2024_01_input.yxdb` | 10,744 | `A2D882C35B0F192644BA98D686F38B85FD037673192E2B9C38D736422F28D731` |
| B21 | `2024/Day02/2024_02_input.yxdb` | 11,627 | `5C5FE77A32CA58BE9316AD1482E847128971C5E4F9F8E7697667E6E7C29745F9` |
| B22 | `2024/Day03/2024_03_input.yxdb` | 13,196 | `69627FC6EFF02E0B9833A39B0C267BD21C8E72DED18F03B793982048CA3A3DE0` |
| B23 | `2024/Day04/2024_04_input.yxdb` | 10,097 | `64BBD6F6BA868F8834BBB696A48F392D37B8BECCF7ED0F8F18FC085699D7916B` |
| B24 | `2024/Day05/2024_05_input.yxdb` | 9,949 | `AAA9431B97D7EAFF96730F4D115BE3B522CDDE8B0E0584955E249F5D014695EA` |
| B25 | `2024/Day06/2024_06_input.yxdb` | 3,310 | `29E529806D58E85909D6914B7A794BEE3530325B9E51590874078665348AF8E8` |
| B26 | `2024/Day07/2024_07_input.yxdb` | 20,261 | `E06A6CA47B29CAF57994F3B306B448FD5634D0FDC5517C988A64EEA2939288FB` |
| B27 | `2024/Day08/2024_08_input.yxdb` | 1,131 | `12FDB6C209CC869A9A725FE80FB6947C06D441C40F3C89D8C0B12D6AF841B607` |
| B28 | `2024/Day09/2024_09_input.yxdb` | 18,100 | `7915381A197D34AB36A171BDC3193B05FE5B9BC5F18D55C4BE7CA02DA2FC2FFA` |
| B29 | `2024/Day09/temp.yxdb` | 210,153 | `BAEEC5EFF8C7702B0D944151F06D75B27391196FB1C84AFCE73F41CB5D81117D` |
| B30 | `2024/Day10/2024_10_input.yxdb` | 3,019 | `96DD999AB7B624F1C51684F9F51EE8D5E3F66D3313AC98F81FD23DF0A9407A95` |
| B31 | `2024/Day10/temp1.yxdb` | 3,979 | `047C97BDC17E74181D00847C30E9ED4786D3ED18FD2EB0F81D6B6335A7C1F495` |
| B32 | `2024/Day10/temp2.yxdb` | 19,984 | `7ECA4C3CB1EEB0AD8E174989DD795D4F7B4127A828A549A8F45CC87162EC320E` |
| B33 | `2024/Day11/2024_11_input.yxdb` | 595 | `0427426B2627648311D7860744AA01BA484A0F78C9061AE749C5FFE518ECAB44` |
| B34 | `2024/Day12/2024_12_input.yxdb` | 8,931 | `41586F7612A7D27B84375E6162536CF5A483855EA0CC200AAEF2B12C60D4EADC` |
| B35 | `2024/Day13/2024_13_input.yxdb` | 8,332 | `58FFA514CD66E6562C309E515A8D019B4BE557299E303DA85697A4E4BE60E764` |
| B36 | `2024/Day14/2024_14_input.yxdb` | 5,799 | `083CBEFE11509F0A08B174FC51CBED8D10F5BDC0C5E5BD71F0BA5624EC919FB3` |
| B37 | `2024/Day15/2024_15_input.yxdb` | 11,116 | `4F1B1DEA78920C999176D63266789654717663FE88083D53284FA87779EF7D65` |
| B38 | `2024/Day15/ForP2Macro1.yxdb` | 81,096 | `3A30F216B753E3ADE2F15AF6F802D6567327335EF14BDE0CD1BA50F6A368CF5E` |
| B39 | `2024/Day15/ForP2macro_2.yxdb` | 14,173 | `B1D89869879C269654C0407EBBDA06B01F3453DE8D5A7E4407A30982E8FB8F12` |
| B40 | `2024/Day16/2024D16P1_dijk.yxdb` | 2,829 | `56F908509118B791DBF1BCA568FE445EA4386DD1B3D46102A56E295236AA90D1` |
| B41 | `2024/Day16/2024D16P1_dijk_p2.yxdb` | 60,801 | `E9943C3FDA1B4E3F696FE2FAD76DBCFE9A5E8784C7996A9E23BAD1980EA26171` |
| B42 | `2024/Day16/2024D16P1_dijk_p2_e.yxdb` | 1,958 | `37EF36B660A65DE4897D4C8C398E51E22D8DDF913CA798AAB7F565E03BCB1DAE` |
| B43 | `2024/Day16/2024_16_input.yxdb` | 7,389 | `185D1AA9C29C0FFD32D9396010D23CBCC2076E2645ED393FF2BDB6ACA345C177` |
| B44 | `2024/Day16/P2_SearchNodes.yxdb` | 6,080 | `F7EAF44AEF0B3269F07CB5C0BE05E6D6EA7CAC17DD5AB2E413E857E6413C7238` |
| B45 | `2024/Day16/P2_edgelist.yxdb` | 92,770 | `5DF1743CFCC2E4D67FE6949D7DF01F82BE71B56BF2ADE05951DD38A25EC99A94` |
| B46 | `2024/Day17/2024_17_input.yxdb` | 525 | `CAA39753763E3987A993E6B4084416666300B612AAE67DA26FFFA42511472B0C` |
| B47 | `2024/Day18/2024_18_input.yxdb` | 13,308 | `FE52C8565C9368F8F54DD07DDDA4805965CB11BD16625A432011D960CF403EBE` |
| B48 | `2024/Day19/2024_19_input.yxdb` | 13,916 | `B834D2100D6DE6BCA047FC368AFA95CB4E1388B4C418A40813E0786E88967925` |
| B49 | `2024/Day20/2024_20_input.yxdb` | 6,944 | `005D32F0A1C768CBAE8759BD783E3C272D645A4277AA30C5980C824FB9A1C604` |
| B50 | `2024/Day21/D21P2_ite_input.yxdb` | 620 | `20EAD3D7394F1C960F225CF403495D418ABBA020044180ACF6B30A4ECDF4C56F` |
| B51 | `2024/Day21/P1_input.yxdb` | 535 | `C3991569F65F1558E138B9ECDCD269D9A29EB07A18A5FC6654EF93346A40AEF6` |
| B52 | `2024/Day22/2024_22_input.yxdb` | 18,085 | `18B15BA5BF3648A5E0A07B3F4D0D0C0A4EBB4576B98874BB4F3975FFBBAA5B45` |
| B53 | `2024/Day23/2024_23_input.yxdb` | 19,595 | `6932356A4490E77039DEF43FDFFBECAF1802B0AD3FE09219C2E1589EF5544FAB` |
| B54 | `2024/Day23/day23_1.yxdb` | 338 | `02B28823218FF7637CBCEC3FE6145527BBCA18725AB88DC0989A6653932C5F17` |
| B55 | `2024/Day23/day23_2.yxdb` | 39,164 | `DD016F78E6D2EACAB67CC973EE0F59470AF5A4ACFBECA78E8EDE6D5E0437F363` |
| B56 | `2024/Day24/2024_24_input.yxdb` | 3,870 | `1E42843ED5A561046D83AE429D089E53570BDC5DE8B6C5645C10A0BD8D444D3D` |
| B57 | `2024/Day25/2024_25_input.yxdb` | 7,792 | `389CC48715DCD494DBD5EE5C7F5285B4AAD59F79B6A993C36D50C8209A3E95D1` |
| B58 | `2025/Day01/2025_01_input.yxdb` | 10,556 | `315CABEA2E0C1433726248C644E270AFC0C9D554FFDC3AB0FC0A7F229C62D847` |
| B59 | `2025/Day02/2025_02_input.yxdb` | 893 | `3461E5469CC3F5223DF52466F88C2DD1EB20CF3B06A9EF200AB4B2A39DC20006` |
| B60 | `2025/Day03/2025_03_input.yxdb` | 14,250 | `CCCCFF2C5E2AABE250BF8D21EC329F9B79F1E76F91087BEAF04249D62FE8BF4C` |
| B61 | `2025/Day05/2025_05_input.yxdb` | 19,239 | `C3284B36A210B9AE2D0DD5948C77FB127937D9A7E778DDDBD792957F56914FF1` |
| B62 | `2025/Day06/2025_06_input_actual.yxdb` | 12,389 | `8AD6F216EF4B2460C16AFCBD047EF173270DBC89FB223E603CFF916133AE2A92` |
| B63 | `2025/Day07/2025_07_input_actual.yxdb` | 3,066 | `B85DFE922028B0759AA039BA4DA3E1A2D717B757AE75A0E9FFFB9F1ADDDE5A65` |
| B64 | `2025/Day08/2025_08_input_actual.yxdb` | 16,809 | `4D2489FA47306F64C80D187D4DC4609E0C7F54D2DEB2A3E48CC5503D490FBDEB` |

### Source C: PacktPublishing/Alteryx-Designer-Cookbook

1 E2 yxdb file committed directly to the repository. Sourced 2026-03-16. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/PacktPublishing/Alteryx-Designer-Cookbook).

| # | File | Size | SHA-256 |
|---|------|------|---------|  
| C1 | `ch3/Recipe2/DATA/CityBike_extract.yxdb` | 3,744,623 | `8A578CF741075E25DF184BCD3EB5BFAE3EFFA1F5E8835E95A01AE240212B41D2` |

### Source D: PacktPublishing/Data-Engineering-with-Alteryx

1 E2 yxdb file committed directly to the repository. Sourced 2026-03-16. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/PacktPublishing/Data-Engineering-with-Alteryx).

| # | File | Size | SHA-256 |
|---|------|------|---------|  
| D1 | `Chapter 06/Data/places_child.yxdb` | 1,204 | `4B69D667A6D262B6BD992A90BF8500C9C56C3A047F71F9DEC3D5EFB0C4E4E474` |

### Source E: SaudAzmi/airport-alteryx-workflow

1 E2 yxdb file committed directly to the repository. Sourced 2026-03-16. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/SaudAzmi/airport-alteryx-workflow).

| # | File | Size | SHA-256 |
|---|------|------|---------|  
| E1 | `Output/Airport_city_population.yxdb` | 159,410 | `CFB9789ABBEE56D3DEA244F58C8DCE6FCB9992175017F34E6EB1A8E7A9E85BA3` |

### Source F: AltonDsouza/Alteryx-Challenge-482-

1 E2 yxdb file extracted from `Challenge482_start_file.yxzp` and also committed directly at `Challenge482_start_file/Outputs/Q3_Answer.yxdb`. Sourced 2026-03-16. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/AltonDsouza/Alteryx-Challenge-482-).

| # | File | Size | SHA-256 |
|---|------|------|---------|  
| F1 | `Challenge482_start_file/Outputs/Q3_Answer.yxdb` | 63,078 | `156376EE57DDC063571B5AF41DC6000DB4433596F0934C3C7C33BF884503246D` |

### Source G: liyengL/Alteryx_challenges

1 E2 yxdb file extracted from `Movie.yxzp`. Sourced 2026-03-16. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/liyengL/Alteryx_challenges).

| # | File | Size | SHA-256 |
|---|------|------|---------|  
| G1 | `Movie.yxzp` → `Input335.yxdb` | 45,403 | `A674BD1E545FA834B18B46E983F55DBF7B841D15017D44EC15EAC65D7255BBA4` |

### Source H: ABANISINGHA/Alteryx_workflows

1 E2 yxdb file sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/ABANISINGHA/Alteryx_workflows).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| H1 | `Output_Superstore_workflow_5.yxdb` | 11,796 | `40263690192DCB948E3403CC22399805BE448F45F86B6E3250429BEF4ED01F12` |

### Source I: ChrisDataBlog/Alteryx-Inspire-2023---Design-Patterns-for-Testing

1 E2 yxdb file sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/ChrisDataBlog/Alteryx-Inspire-2023---Design-Patterns-for-Testing).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| I1 | `Model Schema Example.yxdb` | 442 | `9D54553DE8E34BFB501233CE419DDB13281F0D4BE6D39994ABF07F8F75486D45` |

### Source J: FL-Marine/Alteryx-Work-Flows

9 E2 yxdb files sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/FL-Marine/Alteryx-Work-Flows).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| J1 | `DYNAMIC date.yxdb` | 30,379 | `88967DEF92B49E2A7A9F34F8AEE76079A2DC405F8AA43916BF14D99E7A53AFC2` |
| J2 | `EPL_2018_alteryx.yxdb` | 2,265 | `95876ACD7F6252C8D80760DBAD3A2F018B58A58ACA515C493C44369E02722CD2` |
| J3 | `Forward Positions.yxdb` | 5,750 | `C2DD7E04F3F3F55D0B2FE3401057DB07F4096F9171786F37AF6C5CB28BE6BFC3` |
| J4 | `Results_YXDB_Output.yxdb` | 52,055 | `509752A5113603749768E443CAF9C35432C444618E66717E088EEB045136CC96` |
| J5 | `Section 3 WF.yxdb` | 21,856 | `4C2D1C7808474532A5863868DCCCF2DF546D1D114918F57CA3133FD3F52FA29B` |
| J6 | `Section 5 WF.yxdb` | 88,472 | `ACB2CC69C41098BE0BC805A12143A7908BCF50EB9631CD6ED4BD4F863FD1AD20` |
| J7 | `Top 3 Players.yxdb` | 2,102 | `5AE0E644C1F834F15E26E20EF84F4711892D0948C663BAB4B13751FFF653437A` |
| J8 | `Union_Tool_Output.yxdb` | 4,428 | `C827410D278C6752212B0C9B9D5671684B01F57180AD66293BA847236D7A663F` |
| J9 | `basic_customer_first_purchase.yxdb` | 352 | `5C411CF4CC2594300EE087F83F9B89AE5667B8A1CCADC5A48A6F403D0CACD9BF` |

### Source K: KOdoi-OJ/Capstone-Project-Combining-Predictive-Techniques

2 E2 yxdb files sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/KOdoi-OJ/Capstone-Project-Combining-Predictive-Techniques).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| K1 | `Task 2 - New Stores with Final Clusters.yxdb` | 1,191 | `57A7619BE2B44CB7A7CACBAFD964852F780F7E9694B00D48BFECD61B6A3DB3D1` |
| K2 | `Task 2 - Number of New Stores per cluster.yxdb` | 514 | `054A6A93E80899BA3EE529A74DB336FA7063D932803BEF74167FBC10A57391A7` |

### Source L: MOHAMMADALI230/NovaKart-Profitability-Analytics

3 E2 yxdb files sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/MOHAMMADALI230/NovaKart-Profitability-Analytics).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| L1 | `Customers_LTV_Segmented.yxdb` | 41,243 | `75F56D31D753B576BED9987C74B2CB46518C6F86D0EB890A01B8ED65D23089C4` |
| L2 | `NovaKart_Metric.yxdb` | 593,729 | `78E6F9954878D6F2B4E77024B648FA49F03E2B0F8D079ADB5BEFF8CACDD0AA59` |
| L3 | `Orders_Joined_All.yxdb` | 1,246,096 | `B61104A9895B76BC02EF21D1C8F6846077082F13323A9A19A46A6CFF4819D7D9` |

### Source M: Satvikp546/Alteryx_Workflows

1 E2 yxdb file sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/Satvikp546/Alteryx_Workflows).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| M1 | `NYC Neighborhood Summary.yxdb` | 7,941 | `C47D2DB646CC237B60FCF31EE0064CCFE00E84042C050EEC6FFFAEB2BA8D29D4` |

### Source N: SeanAdams10/AdventOfCodePython

7 E2 yxdb files sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/SeanAdams10/AdventOfCodePython).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| N1 | `inf_fixer_data.yxdb` | 1,441 | `17D6A9205FB846A4E29E911D083C21B8421FFCC35743D39A31EF1E3ABBC0BE4F` |
| N2 | `itf_checker_rules.yxdb` | 5,229 | `4577AB0B59B4F1B052052BF334A8DED3D26F692F1966AF08C9998B460BB6E50A` |
| N3 | `itf_fixer_data.yxdb` | 6,426 | `A00779E2E06D4213C64E925A4AB8C7E34BA7AE44362BCEA5ED8A1DE5C3F0A89A` |
| N4 | `itf_walk_path-AdamsAMD.yxdb` | 883 | `84787B23DF0CB3D29E5B7713A198C81BE15E5F49D2357B8144982D257466F5ED` |
| N5 | `tmpChecker.yxdb` | 564 | `C5FF454EE3805C2048C9D7F6B68845BF95D2D6971F14B335179C8D6BD38995FC` |
| N6 | `tmpGrid.yxdb` | 4,100,774 | `1099621AA7F5BA0FE48A1377D3146DC5ECD21F07476E0C71A21E404C8A7D9FCF` |
| N7 | `tmpIterator.yxdb` | 636 | `FFFF479A709DEEEBC72AB34D1BE32A0683170ADD690B493609DC6DD75C759A4F` |

### Source O: Sivivatu/Alteryx-Weekly-Challenge

1 E2 yxdb file sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/Sivivatu/Alteryx-Weekly-Challenge).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| O1 | `Challenge 275_List of Primes.yxdb` | 8,062 | `F141D573E2B1067EA069B0C6CBC43C54A2CFACCCEBB265197C855841FC40A839` |

### Source P: Szymon-Czuszek/Alteryx-Weekly-Challenges

6 E2 yxdb files sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/Szymon-Czuszek/Alteryx-Weekly-Challenges).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| P1 | `RATIO.yxdb` | 396 | `D80B328270B34289F361CB199D6F0C545B2F503D7DB653E4B460DD75C79017D6` |
| P2 | `Region.yxdb` | 436 | `8F63E157853F84DB7255E56CB6DA45B5062BF2065241E8EC68291BF439395BE5` |
| P3 | `Tire_Size_Data_2.yxdb` | 1,289 | `F2B25BA716E3DD824000B3E2291A95F3B31BA2E769ABE7EB3117C692A49D03E2` |
| P4 | `Type.yxdb` | 570 | `7B75E18714785B5570C1153F0929AE180849BF291FF3C03462DA7C7F2FB32776` |
| P5 | `Wheel_size.yxdb` | 462 | `32764F3265955730514A8D87F177CEC2D0AE34E3738C6D8E3B8BC655CC830B18` |
| P6 | `Width_MM.yxdb` | 1,073 | `A6574825A2A4EDC46311A0EC358BBC0CC63C02CA940E5618041A65B137BEBE5E` |

### Source Q: afnfyz/alteryx_weekly_challenge_filter

3 E2 yxdb files sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/afnfyz/alteryx_weekly_challenge_filter).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| Q1 | `alteryx_challenges_filter_app_tempfiles_completed_challenges.yxdb` | 4,632 | `A81BC23879605A80702ECE6CB2C8F760212442176C8490CE391C409CD8F2BFFF` |
| Q2 | `alteryx_challenges_filter_app_tempfiles_templist.yxdb` | 11,385 | `C3788ECB5C5E284A3A8A1B5BC780910E167DC868D971BE5159821460C6BE10CF` |
| Q3 | `alteryx_challenges_filter_app_tempfiles_tempoutput.yxdb` | 24,339 | `F0EE048237383EEB47E448BFE487C5F58EE739A15493FF48DB12FC29AE976573` |

### Source R: joshuaburkhow/adventofcode

3 E2 yxdb files sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/joshuaburkhow/adventofcode).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| R1 | `Day9part1.yxdb` | 4,405 | `A8EEEE98A608FD1E181C3662C02C78884A05E807D02B580ACACC90E052F03556` |
| R2 | `Day9part2.yxdb` | 5,252 | `140005F71950F0587BED4DE043F2C2DC0EA4F601442A15FFAD3DD82B3258DCE5` |
| R3 | `tmpIterator - Copy.yxdb` | 648 | `ECE2FE70519B4F8C79915437B396751A3AAF606EB083B8135F4E8EB454E39972` |

### Source S: kumarritik24/Sales-Performance-Optimization-DC-Industries

1 E2 yxdb file sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/kumarritik24/Sales-Performance-Optimization-DC-Industries).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| S1 | `Order Data Complete_backup.yxdb` | 47,203 | `C07530FE89C5B1EE572EB66DD57998247176DD79070EEFBA3FE869B6270D7C1F` |

### Source T: mishramayank24/predicitve_analytics_using_ALTERYX

2 E2 yxdb files sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/mishramayank24/predicitve_analytics_using_ALTERYX).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| T1 | `cluster task1 output.yxdb` | 5,348 | `0AD2C3ED864FD2014F602D4BDE75483F7FEE3F036B3DD14554FFAF97ABC3E9C0` |
| T2 | `new store cluster.yxdb` | 621 | `CBA941F4C634071A81AB9C41F36374AE5A19C66075E299FD139124B4A09900CC` |

### Source U: sarincr/Data-Analytics-with-Alteryx

3 E2 yxdb files sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/sarincr/Data-Analytics-with-Alteryx).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| U1 | `MultiOut.yxdb` | 7,564,258 | `E9F99982CA88C67AF53B53DDCEE2EAAFF5994F47470AF3E5D4A772C47026F0DD` |
| U2 | `Out1.yxdb` | 1,536,884 | `56C8235C4DC118ABCF89AC144814DBC6DC9EC0D7E8A2EBF29F1F68B6BC67A927` |
| U3 | `Out2.yxdb` | 3,794,773 | `04E773A4212388CBFF882102D0B77E16C1C38C47F895EA4CE2CA24226C00A7C5` |

### Source V: ziadasal/alteryx-mini-projects

1 E2 yxdb file sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/ziadasal/alteryx-mini-projects).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| V1 | `Output_Task1.yxdb` | 1,023,883 | `27504E50AFF8FC3A70C04689DE4E5C97493A1F2DC60A601C7FAF1749C7F16740` |

### Source W: zulekhapathan/Customer-orders-Workflow-using-Alteryx-designer

2 E2 yxdb files sourced 2026-03-17. Archived on [Software Heritage](https://archive.softwareheritage.org/browse/origin/directory/?origin_url=https://github.com/zulekhapathan/Customer-orders-Workflow-using-Alteryx-designer).

| # | File | Size | SHA-256 |
|---|------|------|---------|
| W1 | `order_items.yxdb` | 29,227 | `80749E132037AB6449B1EA51F5F4E52D1AE3013AB2DC2C0B661B5133E07D8D1E` |
| W2 | `orders.yxdb` | 23,149 | `EF80391E108626D4B2329D7E01EF594308A37A9990F1AB59B30E68D6AB2AE100` |

### Summary

**118 E2 files** across 23 sources (Sources A–G: 72 files, Sources H–W: 46 files).

| Source | Files | Software Heritage |
|--------|-------|-------------------|
| A: habramsohn/MSBA-Portfolio | 3 | Archived |
| B: AkimasaKajitani/AdventOfCode | 64 | Archived |
| C: PacktPublishing/Alteryx-Designer-Cookbook | 1 | Archived |
| D: PacktPublishing/Data-Engineering-with-Alteryx | 1 | Archived |
| E: SaudAzmi/airport-alteryx-workflow | 1 | Archived |
| F: AltonDsouza/Alteryx-Challenge-482- | 1 | Deleted |
| G: liyengL/Alteryx_challenges | 1 | Archived |
| H: ABANISINGHA/Alteryx_workflows | 1 | Archived |
| I: ChrisDataBlog/Alteryx-Inspire-2023---Design-Patterns-for-Testing | 1 | Archived |
| J: FL-Marine/Alteryx-Work-Flows | 9 | Archived |
| K: KOdoi-OJ/Capstone-Project-Combining-Predictive-Techniques | 2 | Archived |
| L: MOHAMMADALI230/NovaKart-Profitability-Analytics | 3 | Archived |
| M: Satvikp546/Alteryx_Workflows | 1 | Archived |
| N: SeanAdams10/AdventOfCodePython | 7 | Archived |
| O: Sivivatu/Alteryx-Weekly-Challenge | 1 | Archived |
| P: Szymon-Czuszek/Alteryx-Weekly-Challenges | 6 | Archived |
| Q: afnfyz/alteryx_weekly_challenge_filter | 3 | Archived |
| R: joshuaburkhow/adventofcode | 3 | Archived |
| S: kumarritik24/Sales-Performance-Optimization-DC-Industries | 1 | Archived |
| T: mishramayank24/predicitve_analytics_using_ALTERYX | 2 | Archived |
| U: sarincr/Data-Analytics-with-Alteryx | 3 | Archived |
| V: ziadasal/alteryx-mini-projects | 1 | Archived |
| W: zulekhapathan/Customer-orders-Workflow-using-Alteryx-designer | 2 | Archived |

---

*Binary analysis is in progress. All sourcing rules, legal constraints, and process requirements were established before any E2 work began. See [Analysis Log](#analysis-log) for the chronological record of findings.*
