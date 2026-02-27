# SigilYX

*High-performance YXDB reader and writer — Rust core, Python bindings.*

[![Crates.io](https://img.shields.io/crates/v/sigilyx)](https://crates.io/crates/sigilyx)
[![PyPI](https://img.shields.io/pypi/v/sigilyx)](https://pypi.org/project/sigilyx/)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE)

[YXDB](SPECIFICATION.md) is the native binary format used by [Alteryx](https://www.alteryx.com/) Designer. SigilYX is a standalone, cross-platform library that reads and writes `.yxdb` files — 1.2–3× faster than the fastest open-source C++ readers.

## Packages

| Package | Install | Docs |
|---------|---------|------|
| **Python** (`sigilyx`) | `pip install sigilyx` | [PyPI](https://pypi.org/project/sigilyx/) · [API docs](https://sigilweaver.app/sigilyx/) |
| **Rust** (`sigilyx`) | `sigilyx = "0.1"` in `Cargo.toml` | [crates.io](https://crates.io/crates/sigilyx) · [docs.rs](https://docs.rs/sigilyx) |

## Why SigilYX?

- **Fast.** Parallel LZF decompression, SIMD UTF-16→UTF-8 transcoding, direct Arrow array construction.
- **Cross-platform.** Windows, macOS, Linux — x64 and ARM wheels, no native Alteryx install needed.
- **Full round-trip.** Read and write all 17 YXDB field types.
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

100,000 rows, 50 runs, median. SigilYX columnar reader vs all open-source YXDB readers:

| Shape | SigilYX | Best C++ | Go | .NET | vs C++ |
|-------|--------:|---------:|---:|-----:|-------:|
| Narrow (2 cols) | **2.9 ms** | 4.1 ms | 7.8 ms | 13.9 ms | **1.5×** |
| Numeric (5 cols) | **4.6 ms** | 5.4 ms | 10.8 ms | 17.7 ms | **1.2×** |
| Mixed (8 cols) | **18.9 ms** | 56.5 ms | 202.7 ms | 152.0 ms | **3.0×** |
| String-heavy (5 cols) | **42.4 ms** | 126.5 ms | 638.9 ms | 287.3 ms | **3.0×** |
| Wide (50 cols) | **66.9 ms** | 192.3 ms | 672.2 ms | 470.6 ms | **2.9×** |

Python (SigilYX) vs pure-Python yxdb-py:

| Shape | SigilYX | yxdb-py | Speedup |
|-------|--------:|--------:|--------:|
| Narrow | 3.3 ms | 508 ms | **153×** |
| Mixed | 20.5 ms | 6,922 ms | **337×** |
| String-heavy | 47.2 ms | 17,613 ms | **373×** |

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
git clone https://github.com/sigilweaver/sigilyx.git
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

Format specification derived from existing open-source implementations — see [SPECIFICATION.md](SPECIFICATION.md) for references.
