# SigilYX

*YXDB reader and writer for Rust, with Python bindings.*

[![CI](https://github.com/Sigilweaver/SigilYX/actions/workflows/ci.yml/badge.svg)](https://github.com/Sigilweaver/SigilYX/actions/workflows/ci.yml)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.20470609.svg)](https://doi.org/10.5281/zenodo.20470609)
[![crates.io](https://img.shields.io/crates/v/sigilyx.svg)](https://crates.io/crates/sigilyx)
[![PyPI](https://img.shields.io/pypi/v/sigilyx.svg)](https://pypi.org/project/sigilyx/)
[![docs.rs](https://img.shields.io/docsrs/sigilyx)](https://docs.rs/sigilyx)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust MSRV](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

[YXDB](SPECIFICATION-E1.md) is the native binary format used by [Alteryx](https://www.alteryx.com/) Designer. SigilYX is a standalone, cross-platform library that reads and writes `.yxdb` files. The core is written in Rust; Python bindings are built on top with [PyO3](https://pyo3.rs/) and integrate with [Polars](https://pola.rs/), [PyArrow](https://arrow.apache.org/docs/python/), and [Pandas](https://pandas.pydata.org/).

> **Format scope:** Full read/write support for the **E1** (original engine) YXDB layout. **Experimental** read support for **E2** (AMP engine) is included: 13 field types have been verified against real E2 files; 4 rarer types (Blob, SpatialObj, Time, WString) have speculative decoders behind an opt-in flag. E2 writing is not yet supported. See [SPECIFICATION-E1.md](SPECIFICATION-E1.md) and [SPECIFICATION-E2.md](SPECIFICATION-E2.md).

## Packages

| Package | Install | Docs |
|---------|---------|------|
| **Python** (`sigilyx`) | `pip install sigilyx` | [PyPI](https://pypi.org/project/sigilyx/) - [API docs](https://sigilweaver.app/sigilyx/) |
| **Rust** (`sigilyx`) | `sigilyx = "0.2"` in `Cargo.toml` | [crates.io](https://crates.io/crates/sigilyx) - [docs.rs](https://docs.rs/sigilyx) |

## What's in the box

- **E1 + E2 format support** - read both original (E1) and AMP-engine (E2) layouts; full E1 write support.
- **All 17 E1 field types** - including `FixedDecimal`, `WString`, `Blob`, and `SpatialObj`.
- **Multiple output formats** - Polars, PyArrow, or Pandas from the same call.
- **Streaming** - batched reads with constant memory, and Polars `LazyFrame` scans with projection and row-limit pushdown.
- **Spatial** - `SpatialObj` columns decoded to ISO WKB (compatible with Shapely, PostGIS, GDAL).
- **Cross-platform** - Linux, macOS, and Windows wheels for x64 and ARM; no native Alteryx install required.

## Quick look

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

## Project layout

```
sigilyx/
  sigilyx/            # Rust core library (published to crates.io)
  sigilyx-python/     # PyO3 + pyo3-polars bindings
  python/sigilyx/     # Python wrapper (__init__.py - Polars/PyArrow/Pandas API)
  tests/              # Python test suite
  docs/               # Docusaurus site (sigilweaver.app/sigilyx/)
  SPECIFICATION-E1.md # YXDB E1 format spec
  SPECIFICATION-E2.md # YXDB E2 format spec (experimental)
```

<details>
<summary><strong>Building from source</strong></summary>

Requires [Rust](https://rustup.rs/) (>= 1.75) and Python 3.9+.

```bash
git clone https://github.com/Sigilweaver/SigilYX.git
cd SigilYX
python -m venv .venv
.venv\Scripts\activate         # Windows
# source .venv/bin/activate    # macOS / Linux
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

</details>

## License

[Apache License 2.0](LICENSE).

The format specification was reconstructed from the published Alteryx engine SDK headers and from existing open-source implementations; the implementation itself is original. See [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md) for attribution of vendored components.
