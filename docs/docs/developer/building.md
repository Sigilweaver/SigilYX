---
sidebar_position: 2
---

# Building from Source

## Prerequisites

| Tool | Version | Purpose |
| --- | --- | --- |
| Rust | 1.75+ | Core library and Python bindings |
| Python | 3.9+ | Python wrapper and tests |
| C compiler | Any | Vendored LZF library (handled by `cc` crate) |
| maturin | 1.0+ | Build Python extension from Rust |

## Setup

```bash
git clone https://github.com/Sigilweaver/sigilyx.git
cd sigilyx

# Create a Python virtual environment
python -m venv .venv
source .venv/bin/activate  # macOS/Linux
# .venv\Scripts\activate   # Windows

# Install build tools and dependencies
pip install maturin polars pyarrow pandas
```

## Build

### Rust Only

```bash
cargo build --release
```

This builds the core `sigilyx` crate. The release profile enables LTO (`lto = "fat"`) and single codegen unit (`codegen-units = 1`) for maximum performance.

### Python Extension

```bash
maturin develop --release
```

This compiles the Rust code and installs the Python package into your virtual environment. Changes to Rust code require re-running this command.

For faster iteration during development (slower runtime):

```bash
maturin develop
```

## Project Structure

```
sigilyx/                    # Workspace root
  Cargo.toml                # Workspace definition
  pyproject.toml             # Python package configuration
  sigilyx/                   # Core Rust library
    Cargo.toml
    build.rs                 # Compiles vendored C LZF library
    csrc/                    # Vendored C source for LZF
    src/
      lib.rs                 # Public API
      reader.rs              # Columnar reader, parallel decompression
      writer.rs              # Pipelined writer
      field.rs               # Field type definitions and parsing
      record.rs              # Record-level extraction
      header.rs              # YXDB header parsing
      lzf.rs                 # LZF compression/decompression
      spatial.rs             # SHP-to-WKB spatial conversion
      error.rs               # Error types
  sigilyx-python/            # PyO3 bindings crate
    Cargo.toml
    src/lib.rs
  python/sigilyx/            # Python wrapper module
    __init__.py              # API surface, Polars plugin registration
  tests/                     # Python test suite
```

## IDE Setup

### VS Code

Recommended extensions:

- **rust-analyzer** - Rust language support
- **Python** - Python language support
- **Even Better TOML** - TOML syntax highlighting

### Cargo Check

For fast feedback during development:

```bash
cargo check --workspace
```

This type-checks without producing binaries and is much faster than a full build.
