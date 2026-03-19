# SigilYX

*High-performance YXDB reader and writer — Rust core, Python bindings.*

[![Crates.io](https://img.shields.io/crates/v/sigilyx)](https://crates.io/crates/sigilyx)
[![PyPI](https://img.shields.io/pypi/v/sigilyx)](https://pypi.org/project/sigilyx/)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE)

[YXDB](SPECIFICATION-E1.md) is the native binary format used by [Alteryx](https://www.alteryx.com/) Designer. SigilYX is a standalone, cross-platform library that reads and writes `.yxdb` files — 1.2–3× faster than the fastest open-source C++ readers.

> **Format scope:** SigilYX has full read/write support for the **E1** (original engine) YXDB layout. **Experimental** read support for **E2** (AMP engine) is included — 13 field types have been verified against real E2 files; 4 rare types (Blob, SpatialObj, Time, WString) have speculative decoders behind an opt-in flag. E2 writing is not yet supported. See [SPECIFICATION-E1.md](SPECIFICATION-E1.md) and [SPECIFICATION-E2.md](SPECIFICATION-E2.md) for details.

## Packages

| Package | Install | Docs |
|---------|---------|------|
| **Python** (`sigilyx`) | `pip install sigilyx` | [PyPI](https://pypi.org/project/sigilyx/) · [API docs](https://sigilweaver.app/sigilyx/) |
| **Rust** (`sigilyx`) | `sigilyx = "0.1"` in `Cargo.toml` | [crates.io](https://crates.io/crates/sigilyx) · [docs.rs](https://docs.rs/sigilyx) |

## Why SigilYX?

- **Fast.** Parallel LZF decompression, SIMD UTF-16→UTF-8 transcoding, direct Arrow array construction.
- **Cross-platform.** Windows, macOS, Linux — x64 and ARM wheels, no native Alteryx install needed.
- **Full round-trip.** Read and write all 17 E1 field types. E2 read support for 13 verified types.
- **Multiple output formats.** Polars, PyArrow, or Pandas from the same call.
- **Streaming.** Batched reads with constant memory; lazy scans with Polars LazyFrames.
- **Spatial support.** `SpatialObj` columns decoded to ISO WKB (compatible with Shapely, PostGIS, GDAL).

## Quick Look

<table>
<tr>
<th>Python</th>
<th>Rust</th>
</tr>
<tr>
<td>

```python
import polars as pl
import sigilyx  # registers pl.read_yxdb() etc.

df = pl.read_yxdb("data.yxdb")
df.yxdb.write("output.yxdb")
```

</td>
<td>

```rust
use sigilyx::{read_yxdb, write_yxdb, SpatialMode};

let df = read_yxdb("data.yxdb", SpatialMode::Wkb)?;
write_yxdb("output.yxdb", &df, &[])?;
```

</td>
</tr>
</table>

## Performance

100,000 rows, 100 runs, median. SigilYX columnar reader vs all open-source YXDB readers:

| Shape | SigilYX | Best C++ | Go | .NET | vs best |
|-------|--------:|---------:|---:|-----:|--------:|
| Narrow (2 cols) | **2.2 ms** | 2.2 ms | 4.5 ms | 8.7 ms | **1.0×** |
| Numeric (5 cols) | **4.2 ms** | 4.3 ms | 7.2 ms | 11.6 ms | **1.0×** |
| Mixed (8 cols) | **21.5 ms** | 39.9 ms | 130.3 ms | 108.4 ms | **1.9×** |
| String-heavy (5 cols) | **52.0 ms** | 85.3 ms | 344.6 ms | 204.6 ms | **1.6×** |
| Wide (50 cols) | **71.0 ms** | 139.6 ms | 439.0 ms | 336.6 ms | **2.0×** |

Python (SigilYX) vs pure-Python yxdb-py:

| Shape | SigilYX | yxdb-py | Speedup |
|-------|--------:|--------:|--------:|
| Narrow | 2.8 ms | 309 ms | **111×** |
| Mixed | 22.2 ms | 4,333 ms | **195×** |
| String-heavy | 52.2 ms | 10,659 ms | **204×** |

See [PERFORMANCE.md](PERFORMANCE.md) for full results and methodology.

## Project Structure

```
sigilyx/
├── sigilyx/            # Rust core library (published to crates.io)
├── sigilyx-python/     # PyO3 + pyo3-polars bindings
├── python/sigilyx/     # Python wrapper (__init__.py — Polars/PyArrow/Pandas API)
├── benchmarks/         # Cross-language benchmark suite (Rust, Go, C#, C++)
├── tests/              # Python test suite
├── PERFORMANCE.md      # Benchmark results
└── SPECIFICATION.md    # YXDB format specification
```

<details>
<summary><strong>Building from source</strong></summary>

Requires [Rust](https://rustup.rs/) and Python 3.9+.

```bash
git clone https://github.com/Sigilweaver/sigilyx.git
cd sigilyx
python -m venv .venv
.venv\Scripts\activate      # Windows
# source .venv/bin/activate  # macOS / Linux
pip install maturin polars pyarrow pandas
maturin develop --release
```

</details>

<details>
<summary><strong>Running tests</strong></summary>

```bash
# Rust tests
cargo test --workspace

# Python tests
pytest tests/ -v
```

See [benchmarks/README.md](benchmarks/README.md) for the full cross-language benchmark setup.

</details>

## License

[GNU Affero General Public License v3.0](LICENSE) (AGPL-3.0-only).

Format specification derived from open-source implementations; implementation is original. See [SPECIFICATION.md](SPECIFICATION.md) for references.
