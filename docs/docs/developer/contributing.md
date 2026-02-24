---
sidebar_position: 5
---

# Contributing

We welcome contributions to SigilYX. This page covers the process and conventions.

## Before You Start

1. Check the [GitHub Issues](https://github.com/sigilweaver/sigilyx/issues) for existing reports or discussion
2. For significant changes, open an issue first to discuss the approach

## Development Setup

See [Building](/developer/building) for full environment setup instructions.

## Code Style

### Rust

- Follow standard `rustfmt` formatting: `cargo fmt --all`
- Run clippy: `cargo clippy --workspace -- -D warnings`
- Write doc comments for public API items
- Use `thiserror` for error types
- Prefer `Result<T>` return types over panics

### Python

- Follow PEP 8 with a line length of 120
- Use type hints for all public functions
- Docstrings in Google style

## Pull Request Process

1. Fork the repository
2. Create a feature branch from `main`
3. Make your changes with tests
4. Run the full test suite:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace -- -D warnings
   cargo test --workspace
   pytest tests/ -v
   ```
5. Open a PR with a clear description of the change

## Contributor License Agreement

By submitting a pull request, you agree that your contributions are licensed under AGPL-3.0, consistent with the project license.

## What to Contribute

Good first contributions:

- Bug fixes with regression tests
- Documentation improvements
- Performance improvements with benchmark evidence
- Additional test coverage for edge cases

Areas where help is especially welcome:

- ARM-specific SIMD optimizations (NEON)
- Additional spatial format support (GeoJSON, WKT)
- New output format integrations
