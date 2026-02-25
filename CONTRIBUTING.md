# Contributing to SigilYX

We welcome contributions! This page covers the essentials — see the full [Contributing Guide](https://sigilweaver.app/sigilyx/developer/contributing) on the docs site for details.

## Quick Start

```bash
git clone https://github.com/sigilweaver/sigilyx.git
cd sigilyx
python -m venv .venv && .venv/Scripts/activate  # or source .venv/bin/activate
pip install maturin polars pyarrow pandas
maturin develop --release
```

## Before You Start

1. Check [GitHub Issues](https://github.com/sigilweaver/sigilyx/issues) for existing reports
2. For significant changes, open an issue first to discuss the approach

## Code Style

- **Rust:** `cargo fmt --all` and `cargo clippy --workspace -- -D warnings`
- **Python:** PEP 8, 120-char line length, type hints, Google-style docstrings

## Pull Request Checklist

1. Fork and branch from `main`
2. Add tests for your changes
3. Run the full suite:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace -- -D warnings
   cargo test --workspace
   pytest tests/ -v
   ```
4. Open a PR with a clear description

## License

By submitting a pull request, you agree that your contributions are licensed under AGPL-3.0, consistent with the project license.
