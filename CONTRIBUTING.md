# Contributing to SigilYX

We welcome contributions! This page covers the essentials - see the full [Contributing Guide](https://sigilweaver.app/sigilyx/developer/contributing) on the docs site for details.

## Quick Start

```bash
git clone https://github.com/Sigilweaver/SigilYX.git
cd SigilYX
python -m venv .venv

# Activate the virtual environment:
# Windows:   .venv\Scripts\activate
# macOS/Linux: source .venv/bin/activate

pip install maturin polars pyarrow pandas
maturin develop --release
```

## Before You Start

1. Check [GitHub Issues](https://github.com/Sigilweaver/SigilYX/issues) for existing reports
2. For significant changes, open an issue first to discuss the approach

## Code Style

- **Rust:** `cargo fmt --all` and `cargo clippy --workspace -- -D warnings`
- **Python:** PEP 8, 120-char line length, type hints, Google-style docstrings
- Source is ASCII-only (no smart quotes, em-dashes, or box-drawing characters)
- Prefer [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`)

## Pull Request Checklist

1. Fork and branch from `main`
2. Add tests for your changes
3. Update [CHANGELOG.md](CHANGELOG.md) with an `[Unreleased]` entry for user-facing changes
4. Run the full suite. `python ci.py` runs the same checks CI does (fmt,
   clippy, Rust tests, and the Python tests) in one command; or run them
   individually:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace -- -D warnings
   cargo test -p sigilyx --locked   # sigilyx-python is a PyO3 extension-module crate and
                                    # can't be tested standalone - see Python step below
   pytest tests/ -v
   ```
5. Open a PR with a clear description

## Vendor Software and Clean-Room Policy

SigilYX's YXDB support (both E1 and E2) was built without reference to any Alteryx proprietary software - see [SPECIFICATION.md](SPECIFICATION.md) for the methodology behind each format variant. Do not run, depend on, or validate changes against Alteryx Designer or any tool that reads YXDB through Alteryx's own libraries - not in CI, not in tests, not in local development.

**Pull requests written or verified with the help of Alteryx Designer or other proprietary Alteryx tooling will not be accepted**, regardless of quality. If you found a bug this way, please open an issue describing the symptom and affected file instead, without pasting output learned from vendor software.

## Security

Please report security vulnerabilities privately - see [SECURITY.md](SECURITY.md). Do not open a public issue for security reports.

## DCO

By submitting a contribution you certify that you have the right to submit the work under the project license and agree to the [Developer Certificate of Origin](https://developercertificate.org/).

## License

By submitting a pull request, you agree that your contributions are licensed under Apache-2.0, consistent with the project license.
