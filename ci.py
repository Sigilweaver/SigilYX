"""Pre-commit check: auto-fix what's safe, flag what isn't."""

import subprocess, sys, shutil

def run(cmd, check=True, **kw):
    print(f"\n> {' '.join(cmd)}")
    r = subprocess.run(cmd, **kw)
    if check and r.returncode != 0:
        return False
    return True

def has(name):
    return shutil.which(name) is not None

errors = []

# 1. Auto-format (safe to fix)
print("--- Formatting (auto-fix) ---")
run(["cargo", "fmt", "--all"], check=False)

# 2. Clippy (not auto-fixable, report only)
print("\n--- Clippy ---")
skip_py = [] if has("python") else ["--exclude", "sigilyx-python"]
if not run(["cargo", "clippy", "--workspace", "--locked"] + skip_py + ["--", "-D", "warnings"], check=False):
    errors.append("clippy")

# 3. Tests
print("\n--- Tests ---")
if not run(["cargo", "test", "--workspace", "--locked"] + skip_py, check=False):
    errors.append("tests")

# Summary
print("\n" + "=" * 40)
if errors:
    print(f"FAILED: {', '.join(errors)}")
    sys.exit(1)
else:
    note = "" if has("python") else " (no Python found - skipped sigilyx-python)"
    print(f"All good!{note}")
