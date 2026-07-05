---
sidebar_position: 5
---

# Contributing

We welcome contributions to SigilYX. This page covers the process and conventions.

## Before You Start

1. Check the [GitHub Issues](https://github.com/Sigilweaver/sigilyx/issues) for existing reports or discussion
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

### General

- Source is ASCII-only (no smart quotes, em-dashes, or box-drawing characters)
- Prefer [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`) for commit messages

## Pull Request Process

1. Fork the repository
2. Create a feature branch from `main`
3. Make your changes with tests
4. Update [CHANGELOG.md](https://github.com/Sigilweaver/SigilYX/blob/main/CHANGELOG.md) with an `[Unreleased]` entry for user-facing changes
5. Run the full test suite:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace -- -D warnings
   cargo test -p sigilyx --locked   # sigilyx-python is a PyO3 extension-module crate;
                                    # it links against libpython and can't be run under
                                    # plain `cargo test` - it's exercised via the Python
                                    # suite below instead
   pytest tests/ -v
   ```
6. Open a PR with a clear description of the change

## Vendor Software and Clean-Room Policy

Both the E1 and E2 YXDB support in SigilYX were developed without using any Alteryx proprietary software - see [SPECIFICATION.md](https://github.com/Sigilweaver/SigilYX/blob/main/SPECIFICATION.md) for the full methodology behind each format variant. E1 was implemented from a spec written by studying independent open-source implementations (including Alteryx's own MIT-licensed OpenYXDB); E2 was reverse-engineered from binary analysis of arm's-length-sourced files with a documented provenance log.

Do not run, depend on, or validate parser/writer changes - for either format variant - against Alteryx Designer or any tool that reads YXDB through Alteryx's own libraries. This applies in CI, in tests, and in local development. Correctness is argued only from independent analysis, roundtrip and self-consistency invariants, and files sourced independently of Alteryx.

**Pull requests written or verified with the help of Alteryx Designer or other proprietary Alteryx tooling will not be accepted**, regardless of code quality, since accepting them would compromise the clean-room provenance of the project. If you've found a bug this way, or would simply rather not write the fix yourself, please open an issue instead. Describe the symptom - what's wrong, and on what file - without pasting output from Alteryx tooling or values learned by running it. We'll investigate and fix it from independent analysis.

## Security

Please report security vulnerabilities privately - see [SECURITY.md](https://github.com/Sigilweaver/SigilYX/blob/main/SECURITY.md). Do not open a public issue for security reports.

## DCO

By submitting a contribution you certify that you have the right to submit the work under the project license (Apache-2.0) and agree to the [Developer Certificate of Origin](https://developercertificate.org/).

## Contributor License Agreement

By submitting a pull request, you agree that your contributions are licensed under Apache-2.0, consistent with the project license.

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
