# SigilYX

*High-performance Rust library for reading and writing Alteryx `.yxdb` files.*

[![Crates.io](https://img.shields.io/crates/v/sigilyx)](https://crates.io/crates/sigilyx)
[![docs.rs](https://img.shields.io/docsrs/sigilyx)](https://docs.rs/sigilyx)
[![License: AGPL-3.0](https://img.shields.io/crates/l/sigilyx)](LICENSE)

YXDB is the native binary format used by [Alteryx](https://www.alteryx.com/) Designer. SigilYX provides a standalone, cross-platform reader and writer with native [Polars](https://pola.rs/) DataFrame integration.

**1.2–3× faster than the fastest open-source C++ readers.** See [Performance](#performance).

## Features

- **Read and Write** — full round-trip support for all 17 YXDB field types
- **Polars integration** — reads directly into `polars::DataFrame` via Arrow array construction
- **Columnar reader** — parallel LZF decompression, SIMD UTF-16→UTF-8 transcoding, memory-mapped I/O
- **Row reader** — iterate record-by-record with typed `FieldValue` variants
- **Streaming writer** — pipelined background compression via `mpsc::sync_channel`
- **Spatial support** — `SpatialObj` columns decoded to ISO WKB (compatible with Shapely, PostGIS, GDAL)
- **Projection** — skip parsing unused columns entirely
- **Batched reads** — constant-memory iteration over large files

## Installation

```toml
[dependencies]
sigilyx = "0.1"
```

## Quick Start

```rust
use sigilyx::{read_yxdb, write_yxdb, SpatialMode};

// Read a YXDB file — returns a Polars DataFrame
let df = read_yxdb("data.yxdb", SpatialMode::Wkb)?;
println!("{}", df);

// Write it back out
write_yxdb("output.yxdb", &df, &[])?;
```

## API Overview

### Reading

```rust
use sigilyx::{read_yxdb, read_yxdb_columns, SpatialMode};

// Full file
let df = read_yxdb("data.yxdb", SpatialMode::Wkb)?;

// Projection — only materialise selected columns
let df = read_yxdb_columns("data.yxdb", &["Id", "Name", "Amount"], SpatialMode::Wkb)?;
```

### Streaming / Batched Reads

```rust
use sigilyx::YxdbReader;

let mut reader = YxdbReader::open("data.yxdb")?;

// Inspect schema without reading data
for field in &reader.fields {
    println!("{}: {:?}", field.name, field.field_type);
}

// Iterate in batches (constant memory)
while let Some(batch) = reader.next_batch(100_000, None)? {
    // batch is a Polars DataFrame with up to 100k rows
    process(batch);
}
```

### Row Reader

```rust
use sigilyx::{YxdbRowReader, FieldValue};

let mut reader = YxdbRowReader::open("data.yxdb")?;

while let Some(record) = reader.next_record()? {
    for value in record {
        match value {
            FieldValue::Int64(Some(n)) => println!("int: {n}"),
            FieldValue::VWString(Some(s)) => println!("str: {s}"),
            FieldValue::Null => {}
            _ => {}
        }
    }
}
```

### Writing

```rust
use sigilyx::{write_yxdb, write_yxdb_with_schema, YxdbWriter, FieldMeta, FieldType};
use polars::prelude::*;

// Schema inferred from DataFrame column types
write_yxdb("output.yxdb", &df, &[])?;

// Explicit schema
let schema = vec![
    FieldMeta { name: "Id".into(), field_type: FieldType::Int64, size: 0, scale: 0 },
    FieldMeta { name: "Name".into(), field_type: FieldType::VWString, size: 255, scale: 0 },
];
write_yxdb_with_schema("output.yxdb", &df, &schema)?;
```

### Streaming Writer

```rust
use sigilyx::YxdbWriter;

let mut writer = YxdbWriter::new("output.yxdb", &first_batch)?;
writer.write_batch(&second_batch)?;
writer.write_batch(&third_batch)?;
writer.finish()?;
```

## Spatial Support

`SpatialObj` columns contain Alteryx's proprietary SHP-derived binary format. SigilYX handles the conversion automatically:

```rust
use sigilyx::{read_yxdb, write_yxdb, SpatialMode};

// Decode to ISO WKB on read (compatible with Shapely, GeoArrow, PostGIS, GDAL)
let df = read_yxdb("spatial.yxdb", SpatialMode::Wkb)?;

// Or keep raw SHP bytes
let df = read_yxdb("spatial.yxdb", SpatialMode::Raw)?;

// Write WKB columns back as SpatialObj
write_yxdb("output.yxdb", &df, &["geometry"])?;

// Low-level conversion
use sigilyx::{shp_to_wkb, wkb_to_shp};
let wkb = shp_to_wkb(&shp_bytes)?;
let shp = wkb_to_shp(&wkb_bytes)?;
```

## Field Types

| YXDB Type | Polars Type | Notes |
|-----------|-------------|-------|
| `Bool` | `Boolean` | |
| `Byte` | `Int16` | Unsigned byte stored as Int16 |
| `Int16` | `Int16` | |
| `Int32` | `Int32` | |
| `Int64` | `Int64` | |
| `Float` | `Float32` | |
| `Double` | `Float64` | |
| `FixedDecimal` | `Decimal` | Precision and scale preserved |
| `String` | `String` | Fixed-width, ASCII/Latin-1 |
| `WString` | `String` | Fixed-width, UTF-16 decoded |
| `V_String` | `String` | Variable-length, ASCII/Latin-1 |
| `V_WString` | `String` | Variable-length, UTF-16 decoded |
| `Date` | `Date` | Days since epoch |
| `DateTime` | `Datetime(us)` | Microsecond precision |
| `Time` | `Time` | Nanosecond precision |
| `Blob` | `Binary` | Variable-length binary |
| `SpatialObj` | `Binary` | Geometry as WKB or raw SHP bytes |

## Performance

100,000 rows, 50 runs, median. Columnar reader vs all open-source YXDB readers:

| Shape | SigilYX | Best C++ | Go | .NET | vs C++ |
|---|--:|--:|--:|--:|--:|
| Narrow (2 cols) | **2.9ms** | 4.1ms | 7.8ms | 13.9ms | **1.5×** |
| Numeric (5 cols) | **4.6ms** | 5.4ms | 10.8ms | 17.7ms | **1.2×** |
| Mixed (8 cols) | **18.9ms** | 56.5ms | 202.7ms | 152.0ms | **3.0×** |
| String-heavy (5 cols) | **42.4ms** | 126.5ms | 638.9ms | 287.3ms | **3.0×** |
| Wide (50 cols) | **66.9ms** | 192.3ms | 672.2ms | 470.6ms | **2.9×** |

See [PERFORMANCE.md](https://github.com/sigilweaver/sigilyx/blob/main/PERFORMANCE.md) for full results and methodology.

## Architecture

**Read pipeline:**
1. Memory-mapped I/O (no heap copy of raw file data)
2. Parse LZF block boundaries, decompress all blocks in parallel (Rayon)
3. Scan record boundaries (arithmetic for fixed-size fields, sequential for variable-length)
4. Build Polars `Series` in parallel — one task per column — using direct Arrow array construction (value buffer + validity bitmap, no `Vec<Option<T>>` intermediate)

**Write pipeline:**
1. Serialize records from DataFrame columns
2. Background thread compresses blocks via `mpsc::sync_channel` while the main thread serializes the next block

## License

AGPL-3.0-only. See [LICENSE](https://github.com/sigilweaver/sigilyx/blob/main/LICENSE).
