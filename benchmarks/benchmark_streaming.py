#!/usr/bin/env python3
"""
SigilYX Streaming Benchmark
============================

Demonstrates the unique capability of ``scan_yxdb()`` / ``read_yxdb_batches()``
to process files **larger than available RAM** with constant memory overhead.

This benchmark is specifically designed to showcase a capability SigilYX has
that competitors lack: **streaming-native processing** backed by the Polars
lazy / streaming engine.  Zero data is held in memory beyond one batch at a
time — allowing queries on files that are 2×, 5×, or 10× the machine's RAM.

Task
----
Single-pass aggregate query on the 32 GB giant benchmark file:

  SELECT COUNT(*), SUM(f64_a), AVG(f64_b)
  FROM   file
  WHERE  i32_a > 0

Only the columns needed for the query are projected (pushdown).

Variants
--------
1. sigilyx-scan     scan_yxdb() + Polars automatic streaming engine
                    O(batch) RAM, fully vectorised — **the showcase**
2. sigilyx-batches  read_yxdb_batches() + manual running accumulator
                    O(batch) RAM, explicit Python loop (single pass)
3. sigilyx-full     read_yxdb() — full materialisation (subprocess)
                    O(file size) RAM — **DNF (OOM)** when file > available RAM
4. alteryx-cpp      Alteryx OpenYXDB C++ row-by-row reader (if built)
                    O(1) RAM but no vectorised engine; included for speed context

Usage
-----
    uv run python benchmarks/benchmark_streaming.py
    uv run python benchmarks/benchmark_streaming.py --file bench_giant_90000000.yxdb
    uv run python benchmarks/benchmark_streaming.py --skip-full-read
    uv run python benchmarks/benchmark_streaming.py --runs 3
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import NamedTuple

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
PROJECT_ROOT = Path(__file__).resolve().parent.parent
BENCHMARKS_DIR = PROJECT_ROOT / "benchmarks"
DATA_DIR = BENCHMARKS_DIR / "data"
CPP_DIR = BENCHMARKS_DIR / "cpp"

sys.path.insert(0, str(PROJECT_ROOT / "python"))

import polars as pl
import sigilyx as yx

# Query columns
FILTER_COL = "i32_a"
FILTER_VAL = 0
AGG_COLS   = ("f64_a", "f64_b")


# ---------------------------------------------------------------------------
# Peak-RSS sampler (background thread)
# ---------------------------------------------------------------------------
class PeakRssSampler:
    """Sample process RSS in a background thread; report peak."""

    def __init__(self, interval: float = 0.05):
        self._interval = interval
        self._peak_mb = 0.0
        self._stop = threading.Event()
        try:
            import psutil
            self._proc = psutil.Process(os.getpid())
            self._available = True
        except ImportError:
            self._available = False

    def __enter__(self):
        if self._available:
            self._thread = threading.Thread(target=self._run, daemon=True)
            self._thread.start()
        return self

    def __exit__(self, *_):
        self._stop.set()
        if self._available:
            self._thread.join(timeout=2)

    def _run(self):
        while not self._stop.is_set():
            try:
                rss = self._proc.memory_info().rss / 1e6
                if rss > self._peak_mb:
                    self._peak_mb = rss
            except Exception:
                break
            self._stop.wait(self._interval)

    @property
    def peak_mb(self) -> float:
        return self._peak_mb if self._available else float("nan")


def _avail_ram_gb() -> float:
    try:
        import psutil
        return psutil.virtual_memory().available / 1e9
    except ImportError:
        return float("nan")


def _total_ram_gb() -> float:
    try:
        import psutil
        return psutil.virtual_memory().total / 1e9
    except ImportError:
        return float("nan")


# ---------------------------------------------------------------------------
# Result type
# ---------------------------------------------------------------------------
class BenchResult(NamedTuple):
    variant: str
    status: str           # "ok" | "dnf" | "skip"
    elapsed_s: float
    peak_rss_mb: float
    rows_processed: int
    file_gb: float
    note: str = ""

    @property
    def throughput_gb_s(self) -> float:
        return self.file_gb / self.elapsed_s if self.elapsed_s > 0 else 0.0

    @property
    def throughput_mrows_s(self) -> float:
        return self.rows_processed / self.elapsed_s / 1e6 if self.elapsed_s > 0 else 0.0


# ---------------------------------------------------------------------------
# Variant 1: scan_yxdb + Polars streaming (single-pass aggregate)
# ---------------------------------------------------------------------------
def bench_scan_streaming(path: str, file_gb: float) -> BenchResult:
    """scan_yxdb() — single-pass streaming aggregate, O(batch) RAM."""
    with PeakRssSampler() as rss:
        t0 = time.perf_counter()
        result = (
            yx.scan_yxdb(path)
            .filter(pl.col(FILTER_COL) > FILTER_VAL)
            .select(
                n         = pl.len(),
                sum_f64a  = pl.col(AGG_COLS[0]).sum(),
                mean_f64b = pl.col(AGG_COLS[1]).mean(),
            )
            .collect()
        )
        elapsed = time.perf_counter() - t0
        rows = int(result["n"][0])

    return BenchResult(
        variant="sigilyx-scan",
        status="ok",
        elapsed_s=elapsed,
        peak_rss_mb=rss.peak_mb,
        rows_processed=rows,
        file_gb=file_gb,
        note="scan_yxdb() + Polars streaming engine (single pass)",
    )


# ---------------------------------------------------------------------------
# Variant 2: read_yxdb_batches (single-pass manual accumulator)
# ---------------------------------------------------------------------------
def bench_batches(path: str, file_gb: float, batch_size: int = 65_536) -> BenchResult:
    """read_yxdb_batches() — single-pass running aggregate, O(batch) RAM."""
    with PeakRssSampler() as rss:
        t0 = time.perf_counter()
        total_rows = 0
        sum_f64a   = 0.0
        mean_acc   = 0.0  # weighted for global mean

        for batch in yx.read_yxdb_batches(
            path, batch_size=batch_size,
            columns=[FILTER_COL, AGG_COLS[0], AGG_COLS[1]],
        ):
            filtered = batch.filter(pl.col(FILTER_COL) > FILTER_VAL)
            n = len(filtered)
            if n:
                total_rows += n
                sum_f64a   += float(filtered[AGG_COLS[0]].sum())
                mean_acc   += float(filtered[AGG_COLS[1]].mean() or 0.0) * n

        elapsed = time.perf_counter() - t0

    return BenchResult(
        variant="sigilyx-batches",
        status="ok",
        elapsed_s=elapsed,
        peak_rss_mb=rss.peak_mb,
        rows_processed=total_rows,
        file_gb=file_gb,
        note=f"read_yxdb_batches(batch_size={batch_size:,}, single pass)",
    )


# ---------------------------------------------------------------------------
# Variant 3: full read — in a subprocess so an OOM can't kill this process
# ---------------------------------------------------------------------------
_FULL_READ_SCRIPT = """\
import sys, time, json
sys.path.insert(0, {pypath!r})
import sigilyx as yx
path = {path!r}
t0 = time.perf_counter()
try:
    df = yx.read(path)
    elapsed = time.perf_counter() - t0
    print(json.dumps({{"status":"ok","elapsed":elapsed,"rows":df.height}}))
except MemoryError:
    elapsed = time.perf_counter() - t0
    print(json.dumps({{"status":"oom","elapsed":elapsed,"rows":0}}))
except Exception as e:
    elapsed = time.perf_counter() - t0
    print(json.dumps({{"status":"error","elapsed":elapsed,"rows":0,"note":str(e)}}))
"""


def bench_full_read(path: str, file_gb: float, timeout_s: float = 120.0) -> BenchResult:
    """yx.read() via subprocess — catches OOM without killing this process."""
    script = _FULL_READ_SCRIPT.format(
        pypath=str(PROJECT_ROOT / "python"),
        path=path,
    )
    t0 = time.perf_counter()
    try:
        proc = subprocess.run(
            [sys.executable, "-c", script],
            capture_output=True, text=True, timeout=timeout_s,
        )
        elapsed = time.perf_counter() - t0
        stdout = proc.stdout.strip()
        if stdout:
            data = json.loads(stdout)
            if data["status"] == "ok":
                return BenchResult("sigilyx-full-read", "ok", data["elapsed"],
                                   float("nan"), data["rows"], file_gb,
                                   "read() full materialisation")
            if data["status"] == "oom":
                note = f"OOM after {data['elapsed']:.0f}s — file exceeds available RAM"
            else:
                note = f"Error: {data.get('note', '')}"
        else:
            note = f"Subprocess crashed (exit {proc.returncode}): {proc.stderr[:120]}"
        return BenchResult("sigilyx-full-read", "dnf", elapsed,
                           float("nan"), 0, file_gb, note)

    except subprocess.TimeoutExpired:
        return BenchResult("sigilyx-full-read", "dnf", timeout_s,
                           float("nan"), 0, file_gb,
                           f"OOM / timeout (>{timeout_s:.0f}s)")
    except Exception as e:
        return BenchResult("sigilyx-full-read", "dnf", 0,
                           float("nan"), 0, file_gb, str(e))


# ---------------------------------------------------------------------------
# Variant 4: Alteryx OpenYXDB C++ (row-by-row, no streaming engine)
# ---------------------------------------------------------------------------
def bench_alteryx_cpp(path: str, file_gb: float) -> BenchResult:
    """Run the pre-built Alteryx OpenYXDB C++ benchmark binary if available."""
    binary = CPP_DIR / "alteryx_build" / "alteryx_openyxdb_benchmark.exe"
    if not binary.exists():
        return BenchResult(
            variant="alteryx-cpp", status="skip",
            elapsed_s=0, peak_rss_mb=float("nan"),
            rows_processed=0, file_gb=file_gb,
            note="build first: benchmarks/cpp/build_alteryx.ps1",
        )
    try:
        t0 = time.perf_counter()
        proc = subprocess.run(
            [str(binary), path, "1"],
            capture_output=True, text=True, timeout=900,
        )
        elapsed = time.perf_counter() - t0
        if proc.returncode != 0:
            return BenchResult("alteryx-cpp", "dnf", elapsed, float("nan"),
                               0, file_gb, f"exit {proc.returncode}")
        data = json.loads(proc.stdout.strip())
        return BenchResult("alteryx-cpp", "ok", elapsed, float("nan"),
                           int(data.get("rows", 0)), file_gb,
                           "Alteryx OpenYXDB C++ (row-by-row, no streaming engine)")
    except subprocess.TimeoutExpired:
        return BenchResult("alteryx-cpp", "dnf", 900, float("nan"),
                           0, file_gb, "DNF: timed out after 900s")
    except Exception as e:
        return BenchResult("alteryx-cpp", "dnf", 0, float("nan"),
                           0, file_gb, str(e))


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------
STATUS_ICON = {"ok": "✓", "dnf": "✗", "skip": "—"}


def print_results(results: list[BenchResult], file_name: str,
                  file_gb: float, avail_gb: float, total_gb: float) -> None:
    W = 100
    print()
    print("=" * W)
    print(f"  SigilYX Streaming Benchmark  ·  {file_name}  ({file_gb:.2f} GB)")
    if total_gb == total_gb:
        ram_tag = ("  ⚠  FILE > AVAILABLE RAM → full-read WILL OOM"
                   if file_gb > avail_gb else "")
        print(f"  Machine RAM: {avail_gb:.1f} GB available / {total_gb:.1f} GB total{ram_tag}")
    print("=" * W)
    print()
    hdr = (f"  {'Variant':<22}  {'':>4}  {'Time':>8}  {'GB/s':>6}  "
           f"{'Mrows/s':>8}  {'Peak RAM':>10}  Note")
    print(hdr)
    print("  " + "-" * (len(hdr) - 2))
    for r in results:
        icon = STATUS_ICON.get(r.status, "?")
        if r.status == "ok":
            t_s  = f"{r.elapsed_s:8.1f}s"
            gb_s = f"{r.throughput_gb_s:6.2f}"
            mr_s = f"{r.throughput_mrows_s:8.2f}"
        else:
            label = "DNF" if r.status == "dnf" else "—"
            t_s, gb_s, mr_s = f"{label:>9}", f"{'—':>6}", f"{'—':>8}"
        rss = (f"{r.peak_rss_mb:,.0f} MB" if r.peak_rss_mb == r.peak_rss_mb
               else "N/A")
        print(f"  {r.variant:<22}  {icon:>4}  {t_s}  {gb_s}  {mr_s}  {rss:>10}  {r.note}")
    print()

    ok  = [r for r in results if r.status == "ok"]
    dnf = [r for r in results if r.status == "dnf"]
    if len(ok) >= 2:
        fastest = min(ok, key=lambda r: r.elapsed_s)
        slowest = max(ok, key=lambda r: r.elapsed_s)
        if fastest is not slowest:
            print(f"  {fastest.variant} is "
                  f"{slowest.elapsed_s / fastest.elapsed_s:.1f}× faster than "
                  f"{slowest.variant}")
    if dnf:
        print(f"  DNF: {', '.join(r.variant for r in dnf)}")
    if ok:
        best = min(ok, key=lambda r: r.elapsed_s)
        rss_s = (f"{best.peak_rss_mb / 1024:.2f} GB RAM"
                 if best.peak_rss_mb == best.peak_rss_mb else "")
        print(f"\n  Best: {best.variant}  —  {best.file_gb:.1f} GB processed in "
              f"{best.elapsed_s:.1f}s  ({best.throughput_gb_s:.2f} GB/s"
              + (f", peak {rss_s}" if rss_s else "") + ")")
    print()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
def main() -> None:
    parser = argparse.ArgumentParser(
        description="SigilYX streaming benchmark — constant-memory processing of large YXDB files"
    )
    parser.add_argument("--file", default="bench_giant_90000000.yxdb",
                        help="YXDB file in benchmarks/data/")
    parser.add_argument("--runs", type=int, default=1)
    parser.add_argument("--skip-full-read", "--no-full-read",
                        dest="skip_full_read", action="store_true",
                        help="Skip the full-read OOM variant")
    parser.add_argument("--full-read-timeout", type=float, default=120.0,
                        help="Seconds before declaring full-read a DNF (default 120)")
    parser.add_argument("--batch-size", type=int, default=65_536)
    args = parser.parse_args()

    path = DATA_DIR / args.file
    if not path.exists():
        print(f"ERROR: {path} not found.")
        print("  Generate: uv run python benchmarks/generate_benchmark_data.py --giant")
        sys.exit(1)

    file_gb  = path.stat().st_size / 1e9
    avail_gb = _avail_ram_gb()
    total_gb = _total_ram_gb()

    print(f"\nSigilYX Streaming Benchmark")
    print(f"  File    : {path.name}  ({file_gb:.2f} GB)")
    print(f"  Polars  : {pl.__version__}")
    if total_gb == total_gb:
        print(f"  RAM     : {avail_gb:.1f} GB avail / {total_gb:.1f} GB total")
        if file_gb > avail_gb:
            print(f"  NOTE    : File ({file_gb:.1f} GB) > available RAM ({avail_gb:.1f} GB)")
            print(f"            → sigilyx-full-read expected to DNF (OOM)")
    print()

    all_results: list[BenchResult] = []

    for run in range(1, args.runs + 1):
        if args.runs > 1:
            print(f"── Run {run}/{args.runs} {'─'*60}")

        print(f"  [1/4] scan_yxdb (streaming) ...", end=" ", flush=True)
        r1 = bench_scan_streaming(str(path), file_gb)
        all_results.append(r1)
        rss_s = (f"peak {r1.peak_rss_mb/1024:.2f} GB RAM"
                 if r1.peak_rss_mb == r1.peak_rss_mb else "")
        print(f"{r1.elapsed_s:.1f}s  ·  {r1.throughput_gb_s:.2f} GB/s"
              + (f"  ·  {rss_s}" if rss_s else ""))

        print(f"  [2/4] read_yxdb_batches ...", end=" ", flush=True)
        r2 = bench_batches(str(path), file_gb, args.batch_size)
        all_results.append(r2)
        rss_s = (f"peak {r2.peak_rss_mb/1024:.2f} GB RAM"
                 if r2.peak_rss_mb == r2.peak_rss_mb else "")
        print(f"{r2.elapsed_s:.1f}s  ·  {r2.throughput_gb_s:.2f} GB/s"
              + (f"  ·  {rss_s}" if rss_s else ""))

        if not args.skip_full_read:
            print(f"  [3/4] read() full materialise (subprocess, "
                  f"timeout={args.full_read_timeout:.0f}s) ...", end=" ", flush=True)
            r3 = bench_full_read(str(path), file_gb, args.full_read_timeout)
            all_results.append(r3)
            print(r3.note)
        else:
            all_results.append(BenchResult(
                "sigilyx-full-read", "skip", 0, float("nan"),
                0, file_gb, "skipped (--skip-full-read)"))
            print(f"  [3/4] sigilyx-full-read skipped")

        print(f"  [4/4] Alteryx OpenYXDB C++ ...", end=" ", flush=True)
        r4 = bench_alteryx_cpp(str(path), file_gb)
        all_results.append(r4)
        if r4.status == "ok":
            print(f"{r4.elapsed_s:.1f}s  ·  {r4.throughput_gb_s:.2f} GB/s")
        else:
            print(r4.note)

    last = all_results[-4:]
    print_results(last, path.name, file_gb, avail_gb, total_gb)

    if args.runs > 1:
        from collections import defaultdict
        by_v: dict[str, list[BenchResult]] = defaultdict(list)
        for r in all_results:
            if r.status == "ok":
                by_v[r.variant].append(r)
        if by_v:
            print("  Multi-run averages:")
            for v, rs in by_v.items():
                avg_t  = sum(r.elapsed_s for r in rs) / len(rs)
                avg_gb = sum(r.throughput_gb_s for r in rs) / len(rs)
                print(f"    {v:<22}  {avg_t:7.1f}s  {avg_gb:6.2f} GB/s  (n={len(rs)})")
            print()


if __name__ == "__main__":
    main()
