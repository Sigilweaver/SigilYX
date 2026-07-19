# SigilYX

*Rust reader and writer for Alteryx `.yxdb` files.*

[![Crates.io](https://img.shields.io/crates/v/sigilyx)](https://crates.io/crates/sigilyx)
[![docs.rs](https://img.shields.io/docsrs/sigilyx)](https://docs.rs/sigilyx)
[![License: Apache-2.0](https://img.shields.io/crates/l/sigilyx)](LICENSE)

YXDB is the native binary format used by [Alteryx](https://www.alteryx.com/) Designer. SigilYX is a standalone, cross-platform reader and writer with native [Polars](https://pola.rs/) DataFrame integration. No native Alteryx Designer installation is required.

> **Format scope:** SigilYX has full read/write support for the **E1** (original engine) YXDB layout. **Experimental** read support for **E2** (AMP engine) is included - 13 field types have been verified against real E2 files; 4 rare types (Blob, SpatialObj, Time, WString) have speculative decoders behind an opt-in flag. E2 writing is not yet supported. See [SPECIFICATION-E2.md](https://github.com/Sigilweaver/SigilYX/blob/main/SPECIFICATION-E2.md) for details.

## Features

- **Read and write** - full round-trip for all 17 E1 field types; E2 read support for 13 verified types
- **Polars integration** - reads directly into `polars::DataFrame` via Arrow array construction
- **Columnar reader** - memory-mapped I/O, parallel block decompression
- **Row reader** - iterate record-by-record with typed `FieldValue` variants
- **Streaming writer** - pipelined background compression
- **Spatial support** - `SpatialObj` columns decoded to ISO WKB (compatible with Shapely, PostGIS, GDAL)
- **Projection** - skip parsing unused columns entirely
- **Batched reads** - constant-memory iteration over large files

## Installation

```toml
[dependencies]
sigilyx = "0.3"
```

## Quick Start

```rust
use sigilyx::{read_yxdb, write_yxdb, SpatialMode};

// Read a YXDB file - returns a Polars DataFrame
let df = read_yxdb("data.yxdb", SpatialMode::Wkb)?;
println!("{}", df);

// Write it back out
write_yxdb("output.yxdb", &df, &[])?;
```

See the [repository README](https://github.com/Sigilweaver/SigilYX#readme) for the full API tour, the field-type mapping, and Python bindings (`pip install sigilyx`).

## License

[Apache License 2.0](https://github.com/Sigilweaver/SigilYX/blob/main/LICENSE).
