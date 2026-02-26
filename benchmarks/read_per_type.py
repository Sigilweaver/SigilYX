"""Read benchmark for all per-type YXDB files."""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent / "python"))
import sigilyx as yx

results_file = Path(__file__).parent / "data" / "read_per_type_results.txt"
files = sorted(Path(__file__).parent.joinpath("data").glob("bench_type_*_10100000.yxdb"))

if not files:
    print("No per-type files found. Run: uv run python benchmarks/generate_benchmark_data.py --per-type")
    sys.exit(1)

header = f"{'File':<52} {'Rows':>12} {'Size MB':>9} {'Read s':>8} {'MB/s':>8} {'Mrows/s':>9}"
separator = '-' * len(header)

lines = [header, separator]

total_bytes = 0
total_time = 0.0

for f in files:
    mb = f.stat().st_size / 1e6
    t0 = time.perf_counter()
    df = yx.read(str(f))
    elapsed = time.perf_counter() - t0
    rows = df.height
    total_bytes += f.stat().st_size
    total_time += elapsed
    line = f"{f.name:<52} {rows:>12,} {mb:>9.1f} {elapsed:>8.3f} {mb/elapsed:>8.0f} {rows/elapsed/1e6:>9.2f}"
    lines.append(line)
    print(line, flush=True)

lines.append(separator)
total_mb = total_bytes / 1e6
summary = f"{'TOTAL':<52} {'':>12} {total_mb:>9.1f} {total_time:>8.3f} {total_mb/total_time:>8.0f}"
lines.append(summary)
print(separator)
print(summary)

results_file.write_text("\n".join(lines) + "\n")
print(f"\nResults saved to {results_file}")
