---
sidebar_position: 1
description: "Install SigilYX from PyPI or build from source. Includes optional extras for PyArrow, Pandas, and geospatial support."
---

# Installation

## From PyPI

```bash
pip install sigilyx
```

Pre-built wheels are available for:

| Platform | Architectures |
| --- | --- |
| Windows | x64 |
| macOS | x64, ARM (Apple Silicon) |
| Linux | x64, ARM |

## Optional Extras

SigilYX's core dependency is Polars. If you need PyArrow or Pandas output, install the extras:

```bash
# PyArrow support
pip install sigilyx[arrow]

# Pandas support (includes PyArrow)
pip install sigilyx[pandas]

# Everything
pip install sigilyx[all]
```

### Geospatial Dependencies

For GeoPandas and GeoArrow support (see [Spatial & GeoArrow](/python/spatial)), install `geopandas` and `shapely` separately:

```bash
pip install geopandas shapely
```

## Requirements

- **Python**: 3.9 or later
- **Polars**: 0.20 or later (installed automatically)
- **PyArrow**: 14.0 or later (optional, for `[arrow]` and `[pandas]` extras)
- **Pandas**: 1.5 or later (optional, for `[pandas]` extra)

## From Source

Building from source requires the Rust toolchain:

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build
git clone https://github.com/sigilweaver/sigilyx.git
cd sigilyx
python -m venv .venv
source .venv/bin/activate  # or .venv\Scripts\activate on Windows
pip install maturin polars
maturin develop --release
```

The `--release` flag enables compiler optimizations. Debug builds work but are significantly slower.

## Verify Installation

```python
import sigilyx
print(sigilyx.__version__)
```
