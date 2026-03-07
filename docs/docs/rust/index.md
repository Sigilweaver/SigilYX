---
sidebar_position: 1
---

# Rust Guide

SigilYX is a Rust library first. The Python bindings are a layer on top of the Rust core. If you're building Rust applications that need to read or write YXDB files, you can use the `sigilyx` crate directly.

## What You Get

- **Read** YXDB files into Polars DataFrames
- **Write** Polars DataFrames to YXDB files
- **Streaming** batched reads with configurable batch size
- **Column projection** to only decode the columns you need
- **Row-level iteration** for custom processing pipelines
- **SpatialObj** support with SHP-to-WKB conversion
- **All 17 YXDB field types** fully supported

## Quick Example

```rust
use sigilyx::{read_yxdb, write_yxdb, SpatialMode};

fn main() -> sigilyx::Result<()> {
    // Read
    let df = read_yxdb("data.yxdb", SpatialMode::Wkb)?;
    println!("{}", df);

    // Write
    write_yxdb("output.yxdb", &df, &[])?;
    Ok(())
}
```

## Sections

- [Getting Started](/rust/getting-started) - Installation, dependencies, and first program
- [Reading](/rust/reading) - All read APIs: eager, batched, projected, row-level
- [Writing](/rust/writing) - Write DataFrames and streaming writes
- [Field Types](/rust/field-types) - YXDB field types and their Rust/Arrow representations
