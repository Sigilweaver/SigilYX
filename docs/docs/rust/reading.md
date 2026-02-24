---
sidebar_position: 2
---

# Reading YXDB Files in Rust

SigilYX provides several read APIs depending on your needs.

## Eager Read (Full File)

Read the entire file into a Polars DataFrame:

```rust
use sigilyx::{read_yxdb, SpatialMode};

let df = read_yxdb("data.yxdb", SpatialMode::Wkb)?;
println!("{}", df);
```

The `SpatialMode` argument controls how `SpatialObj` columns are handled:

| Mode | Behavior |
| --- | --- |
| `SpatialMode::Wkb` | Decode SHP geometry to ISO WKB (compatible with PostGIS, GDAL, Shapely) |
| `SpatialMode::Raw` | Keep raw SHP bytes as-is |
| `SpatialMode::GeoArrow` | Same as WKB at the Rust level; GeoArrow metadata is applied in the Python layer |

## Column Projection

Read only specific columns:

```rust
use sigilyx::{read_yxdb_columns, SpatialMode};

let df = read_yxdb_columns("data.yxdb", &["id", "name", "amount"], SpatialMode::Raw)?;
```

Columns not in the list are never decoded. This is significantly faster for wide files.

## Low-Level Reader

For more control, use `YxdbReader` directly:

```rust
use sigilyx::YxdbReader;

let reader = YxdbReader::open("data.yxdb")?;

// Inspect metadata
println!("Records: {}", reader.record_count);
for field in &reader.fields {
    println!("  {} : {:?} (size={})", field.name, field.field_type, field.size);
}

// Read into DataFrame
let df = reader.into_dataframe()?;
```

## Batched Read

Read in batches for constant-memory processing:

```rust
use sigilyx::YxdbReader;

let mut reader = YxdbReader::open("large_file.yxdb")?;

while let Some(batch) = reader.next_batch(100_000, None)? {
    println!("Batch shape: {:?}", batch.shape());
    // Process batch...
}
```

With column projection:

```rust
let mut reader = YxdbReader::open("large_file.yxdb")?;
let columns = Some(&["id", "name"][..]);

while let Some(batch) = reader.next_batch(50_000, columns)? {
    process(batch);
}
```

## Row-Level Read

For row-by-row processing:

```rust
use sigilyx::{YxdbRowReader, FieldValue};

let reader = YxdbRowReader::open("data.yxdb")?;

for record in reader {
    let record = record?;
    for (field, value) in record.iter() {
        match value {
            FieldValue::Int64(v) => print!("{v}\t"),
            FieldValue::String(s) => print!("{s}\t"),
            FieldValue::Null => print!("NULL\t"),
            _ => print!("{value:?}\t"),
        }
    }
    println!();
}
```

The row reader is slower than the columnar reader (see [Performance](/performance)) but gives you per-record control without materialization.

## Arrow IPC Interop

Read a YXDB file and get the result as Arrow IPC bytes (useful for sending to another process or language):

```rust
use sigilyx::{read_yxdb_to_ipc, SpatialMode};

let ipc_bytes: Vec<u8> = read_yxdb_to_ipc("data.yxdb", SpatialMode::Wkb)?;
// Send ipc_bytes to Python, Java, etc.
```

Batched IPC:

```rust
use sigilyx::{read_yxdb_to_ipc_batches, SpatialMode};

let batches: Vec<Vec<u8>> = read_yxdb_to_ipc_batches(
    "data.yxdb",
    100_000,
    SpatialMode::Raw,
)?;
```
