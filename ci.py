#!/usr/bin/env python3
"""
Local CI — run the same checks as GitHub Actions without spending money.

Usage:
    python ci.py              # run everything (lint + test-rust + test-python)
    python ci.py lint         # cargo fmt --check + clippy
    python ci.py test-rust    # cargo test
    python ci.py test-python  # maturin develop + pytest
    python ci.py all          # same as no argument
    python ci.py --fix        # auto-format before linting (useful pre-commit)
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import time

# ── Helpers ─────────────────────────────────────────────────────────────

DIVIDER = "=" * 60

def has(name: str) -> bool:
    return shutil.which(name) is not None

def run(cmd: list[str], *, label: str | None = None, **kw) -> bool:
    """Run a command, print it, return True on success."""
    tag = f"  [{label}]" if label else ""
    print(f"\n> {' '.join(cmd)}{tag}")
    t0 = time.monotonic()
    result = subprocess.run(cmd, **kw)
    elapsed = time.monotonic() - t0
    ok = result.returncode == 0
    status = "OK" if ok else f"FAILED (exit {result.returncode})"
    print(f"  {status}  ({elapsed:.1f}s)")
    return ok


def heading(text: str) -> None:
    print(f"\n{DIVIDER}\n  {text}\n{DIVIDER}")


# ── Detect environment ─────────────────────────────────────────────────

def cargo_extra_args() -> list[str]:
    """Flags shared across cargo invocations."""
    args = ["--workspace", "--locked"]
    if not has("python") and not _in_venv():
        args += ["--exclude", "sigilyx-python"]
    return args

def _in_venv() -> bool:
    return sys.prefix != sys.base_prefix

def _python() -> str:
    """Return the best python executable name."""
    if _in_venv():
        return sys.executable
    for name in ("python3", "python"):
        if has(name):
            return name
    return "python"


# ── CI steps ────────────────────────────────────────────────────────────

def step_lint(*, fix: bool = False) -> list[str]:
    """Check formatting and run clippy.  Returns list of failure names."""
    failures: list[str] = []
    heading("Formatting" + (" (auto-fix)" if fix else " (check)"))
    fmt_args = ["cargo", "fmt", "--all"]
    if not fix:
        fmt_args += ["--", "--check"]
    if not run(fmt_args, label="fmt"):
        failures.append("fmt")

    heading("Clippy")
    clippy_cmd = ["cargo", "clippy"] + cargo_extra_args() + ["--", "-D", "warnings"]
    if not run(clippy_cmd, label="clippy"):
        failures.append("clippy")

    return failures


def _ensure_python_dll_on_path() -> None:
    """On Windows, cargo test for pyo3 crates needs python3.dll on PATH."""
    if sys.platform != "win32":
        return
    import sysconfig
    # The DLL lives next to the Python executable (or in the base prefix).
    for candidate in (
        os.path.dirname(sys.executable),
        sysconfig.get_config_var("installed_base") or "",
        sys.base_prefix,
    ):
        if candidate and os.path.isfile(os.path.join(candidate, "python3.dll")):
            if candidate not in os.environ.get("PATH", ""):
                os.environ["PATH"] = candidate + os.pathsep + os.environ.get("PATH", "")
            return


def step_test_rust() -> list[str]:
    """Run the Rust test suite.  Returns list of failure names."""
    heading("Rust tests")
    _ensure_python_dll_on_path()
    if not run(["cargo", "test"] + cargo_extra_args(), label="cargo test"):
        return ["test-rust"]
    return []


def _has_maturin(py: str) -> bool:
    """Check whether maturin is importable (works even if Scripts/ is not on PATH)."""
    if has("maturin"):
        return True
    return subprocess.run([py, "-m", "maturin", "--version"],
                          capture_output=True).returncode == 0


def step_test_python() -> list[str]:
    """Build Python bindings and run pytest.  Returns list of failure names."""
    heading("Python tests")
    failures: list[str] = []
    py = _python()

    if not _has_maturin(py):
        print("  maturin not found -- trying to install it ...")
        run([py, "-m", "pip", "install", "maturin"], label="pip install maturin")

    if not _has_maturin(py):
        print("  [!] maturin still not found, skipping Python tests")
        return ["test-python (maturin missing)"]

    # Prefer `python -m maturin` so it works even when Scripts/ is not on PATH.
    maturin_cmd = ["maturin"] if has("maturin") else [py, "-m", "maturin"]

    if not run(maturin_cmd + ["develop", "--release"], label="maturin develop"):
        failures.append("maturin-build")
        return failures  # no point running pytest if build failed

    # Ensure test deps are installed
    deps = ["polars", "pyarrow", "pandas", "pytest"]
    run([py, "-m", "pip", "install", "--quiet"] + deps, label="pip install deps")

    if not run([py, "-m", "pytest", "tests/", "-v"], label="pytest"):
        failures.append("pytest")

    return failures


# ── Main ────────────────────────────────────────────────────────────────

STEPS = {
    "lint":        step_lint,
    "test-rust":   step_test_rust,
    "test-python": step_test_python,
}

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Local CI — mirrors the GitHub Actions workflow.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument(
        "steps",
        nargs="*",
        default=["all"],
        choices=list(STEPS.keys()) + ["all"],
        help="Which CI steps to run (default: all)",
    )
    parser.add_argument(
        "--fix",
        action="store_true",
        help="Auto-format code instead of just checking (applies to lint step)",
    )
    args = parser.parse_args()

    chosen = list(STEPS.keys()) if "all" in args.steps else args.steps
    all_failures: list[str] = []

    for name in chosen:
        fn = STEPS[name]
        if name == "lint":
            all_failures += fn(fix=args.fix)
        else:
            all_failures += fn()

    # Summary
    heading("Summary")
    if all_failures:
        print(f"  FAILED: {', '.join(all_failures)}")
        sys.exit(1)
    else:
        skipped = []
        if not has("python") and not _in_venv():
            skipped.append("sigilyx-python crate (no Python)")
        note = f"  (skipped: {', '.join(skipped)})" if skipped else ""
        print(f"  All checks passed!{note}")


if __name__ == "__main__":
    main()
