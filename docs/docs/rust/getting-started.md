---
sidebar_position: 1
description: "Get started with the SigilYX Rust crate for reading and writing YXDB files."
---

# Getting Started with Rust

## Add the Dependency

Add `sigilyx` to your `Cargo.toml`:

```toml
[dependencies]
sigilyx = { git = "https://github.com/sigilweaver/sigilyx.git" }
```

SigilYX depends on Polars (for DataFrame output) and links against a vendored C LZF library (compiled automatically via `build.rs`).

## Requirements

- **Rust**: 1.75+ (2021 edition)
- **C compiler**: Required for the vendored LZF decompression library (cc crate handles this automatically)
- **Platforms**: Windows, macOS, Linux (x64 and ARM)

## First Program

```rust
use sigilyx::{read_yxdb, SpatialMode};

fn main() -> sigilyx::Result<()> {
    let df = read_yxdb("data.yxdb", SpatialMode::Wkb)?;

    println!("Shape: {:?}", df.shape());
    println!("{}", df.head(Some(10)));

    Ok(())
}
```

Build and run:

```bash
cargo run --release
```

The `--release` flag enables optimizations including LTO and single codegen unit, which significantly improve read performance.

## Key Types

| Type | Description |
| --- | --- |
| `YxdbReader` | Low-level reader; gives access to fields, record count, and batch iteration |
| `YxdbRowReader` | Row-by-row iterator returning `FieldValue` per field |
| `YxdbWriter` | Streaming writer for building YXDB files incrementally |
| `FieldMeta` | Metadata about a single column (name, type, size, scale) |
| `FieldType` | Enum of all 17 YXDB field types |
| `FieldValue` | Sum type for row-level values (Int32, String, etc.) |
| `SpatialMode` | Controls SpatialObj decoding: `Wkb`, `Raw`, or `GeoArrow` |

## Error Handling

All fallible operations return `sigilyx::Result<T>`, which is an alias for `std::result::Result<T, YxdbError>`.

```rust
use sigilyx::YxdbError;

match read_yxdb("missing.yxdb", SpatialMode::Raw) {
    Ok(df) => println!("{}", df),
    Err(YxdbError::IoError(e)) => eprintln!("File error: {e}"),
    Err(YxdbError::InvalidHeader(msg)) => eprintln!("Bad header: {msg}"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

## Crate Features

The `sigilyx` crate enables the following Polars features by default:

- `lazy` -- LazyFrame support
- `dtype-date`, `dtype-datetime`, `dtype-time` -- Temporal types
- `dtype-decimal` -- Fixed-point decimals
- `ipc` -- Arrow IPC serialization (used for cross-language interop)
