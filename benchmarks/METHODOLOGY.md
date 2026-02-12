# Benchmark Methodology

This document describes the methodology used in the SigilYX comparative
read benchmarks. It is written for an audience familiar with systems
benchmarking and performance measurement so that results can be
independently evaluated and reproduced.

---

## 1. Objective

Measure the **wall-clock time to read an entire YXDB file** into a
usable in-memory data structure, comparing SigilYX against every
available third-party YXDB reader across **9 benchmark targets**:

| Target | Language | Package / Source | Read API |
|--------|----------|------------------|----------|
| **sigilyx-rust** | Rust (native) | `sigilyx` (workspace crate) | `sigilyx::read_yxdb()` -> Polars DataFrame |
| **sigilyx-py-polars** | Python (Rust backend) | `sigilyx` (maturin) | `sigilyx.read_yxdb()` -> Polars DataFrame |
| **sigilyx-py-arrow** | Python (Rust backend) | `sigilyx` (maturin) | `sigilyx.read_yxdb_arrow()` -> PyArrow Table |
| **sigilyx-py-pandas** | Python (Rust backend) | `sigilyx` (maturin) | `sigilyx.read_yxdb_pandas()` -> Pandas DataFrame |
| **yxdb-py** | Pure Python | `pip install yxdb` | Row-by-row (`YxdbReader.next()` / `read_index()`) |
| **yxdb-go** | Go | `github.com/tlarsendataguy-yxdb/yxdb-go` | Row-by-row iterator |
| **yxdb-net** | C# (.NET 8) | NuGet `yxdb` v1.1.0 | Row-by-row iterator |
| **alteryx-openyxdb** | C++ (CMake) | `github.com/alteryx/OpenYXDB` | Row-by-row (`ReadRecord()`) |
| **nedharding-openyxdb** | C++ (MSVC) | `github.com/AlteryxNed/Open_AlteryxYXDB` | Row-by-row (`ReadRecord()`) |

### Version Pinning

All library versions are recorded in the JSON output for reproducibility:

| Library | Version |
|---------|---------|
| sigilyx | 0.1.0 (Polars 0.46, PyO3 0.23) |
| yxdb-py | 1.1.1 |
| yxdb-net | 1.1.0 (NuGet) |
| yxdb-go | commit 4f2c373 (Dec 2023) |
| alteryx-openyxdb | latest main (no releases) |
| nedharding-openyxdb | latest main (no releases) |

Each library is benchmarked in its **native language** using each
language's highest-resolution monotonic timer. The Python benchmark
script compares libraries that share a Python runtime directly; Rust,
Go, C#, and C++ benchmarks run as separate processes, outputting JSON
that the orchestrator collects into a unified report.

---

## 2. What is Measured

A single **"run"** consists of:

1. **Open** the YXDB file (construct reader, parse 512-byte header,
   parse XML field metadata).
2. **Decompress** all LZF-compressed record blocks.
3. **Extract** every field value for every record.
4. **Materialize** the data in the library's output format:
   - **sigilyx-rust:** Polars DataFrame (Arrow columnar, constructed
     entirely in Rust).
   - **sigilyx-py-polars:** Polars DataFrame (Arrow columnar in Rust,
     serialized via Arrow IPC, deserialized in Python).
   - **sigilyx-py-arrow:** PyArrow Table (Arrow IPC from Rust,
     zero-copy deserialized in Python via PyArrow).
   - **sigilyx-py-pandas:** Pandas DataFrame (Arrow IPC from Rust,
     converted to Pandas via Arrow-to-Pandas bridge).
   - **yxdb-py:** Row-by-row extraction via `reader.next()` and
     `reader.read_index(i)` for every field.
   - **yxdb-go:** Typed Go values extracted per field per row (no
     columnar accumulation — measures the reader's pure iteration speed).
   - **yxdb-net:** Typed C# values extracted per field per row (same
     approach as Go).
   - **alteryx-openyxdb:** Typed C++ values extracted per field per row
     via `GetAsInt64()`, `GetAsDouble()`, `GetAsWString()`, etc.
   - **nedharding-openyxdb:** Typed C++ values extracted per field per
     row via `GetAsInt32()`, `GetAsDouble()`, `GetAsWString()`, etc.

The timer starts immediately before step 1 and stops immediately after
step 4. No additional post-processing (DataFrame conversion, sorting,
etc.) is included.

### 2.1 Fairness of Comparison

The libraries have fundamentally different architectures:

- **sigilyx** (all variants) is designed for bulk columnar output. It
  decompresses all blocks, extracts values directly into Arrow column
  builders, and returns the result through an IPC zero-copy bridge.
  There is no per-row Python/Go/C# interpretation. The three Python
  output formats (Polars, Arrow, Pandas) differ only in how the Arrow
  IPC bytes are deserialized on the Python side.

- **yxdb-py** is a pure-Python row-by-row reader. Every byte of LZF
  decompression and every field extraction runs in the CPython
  interpreter.

- **yxdb-go** and **yxdb-net** are compiled row-by-row readers. They
  are faster than pure Python for CPU-bound work but lack a columnar
  output mode.

- **alteryx-openyxdb** and **nedharding-openyxdb** are native C++
  row-by-row readers. They perform LZF decompression and field
  extraction in unmanaged code with no runtime overhead (no GC, no
  JIT). They represent the performance ceiling for row-oriented reading
  of YXDB files.

This architectural difference is a feature of each library, not a
benchmark distortion. The benchmark measures the end-to-end work
required to go from a file on disk to structured data in memory — the
task every user of these libraries performs.

### 2.2 Cross-Language Timing Comparability

| Language | Timer | Resolution | Monotonic |
|----------|-------|------------|-----------|
| Rust | `std::time::Instant::now()` | ~1 ns | Yes |
| Python | `time.perf_counter()` | ~100 ns (Windows), ~1 ns (Linux) | Yes |
| Go | `time.Now()` / `time.Since()` | ~1 ns | Yes |
| C# | `System.Diagnostics.Stopwatch` | ~100 ns (Windows) | Yes |
| C++ | `QueryPerformanceCounter` (Windows) | ~100 ns | Yes |

All timers are monotonic and unaffected by wall-clock adjustments.
Resolution is orders of magnitude finer than the measured durations
(milliseconds to seconds), so timer granularity does not affect results.

---

## 3. Test Data

### 3.1 Data Generation

Benchmark data files are generated by `benchmarks/generate_benchmark_data.py`
using SigilYX's writer (`sigilyx.write_yxdb()`). This ensures:

- **Deterministic output:** All random data is seeded with `seed=42`.
  Running the generator twice produces identical files.
- **Self-contained:** Data is generated using SigilYX's own writer.
- **Controlled profiles:** Five data profiles exercise different code
  paths (numeric decoding, string handling, null checks, column count).

### 3.2 Data Profiles

| Profile | Columns | Types | Purpose |
|---------|--------:|-------|---------|
| **numeric** | 5 | Int32, Int64, Float32, Float64, Int16 | Pure numeric throughput; no string allocation |
| **string_heavy** | 5 | V_WString (short ~10, medium ~50, long ~200, nullable, mixed) | String decode and variable-length field performance |
| **mixed** | 8 | Int64, Float64, V_WString, Bool, Date, DateTime, Int16, V_WString | Representative real-world schema |
| **wide** | 50 | 15x Int64, 15x Float64, 10x V_WString, 5x Bool, 5x Date | Column-count stress (field metadata overhead) |
| **narrow** | 2 | Int64, Float64 | Minimal overhead; maximum row throughput |

### 3.3 Row Counts

| Rows | Purpose |
|-----:|---------|
| 1,000 | Baseline; detects per-invocation overhead |
| 10,000 | Mid-range; most code paths exercised |
| 100,000 | Primary throughput test; per-row processing dominates |

This produces **15 benchmark files** (5 profiles x 3 sizes), ranging
from ~15 KB (narrow, 1K rows) to ~65 MB (wide, 100K rows), totaling
approximately 149 MB.

### 3.4 File Naming Convention

```
benchmarks/data/bench_{profile}_{rows}.yxdb
```

Examples: `bench_numeric_100000.yxdb`, `bench_string_heavy_1000.yxdb`

### 3.5 Legacy Test Files

The `sigilyx/test_files/` directory contains small hand-crafted YXDB
files (AllTypes.yxdb, Strings.yxdb, NullValues.yxdb, People.yxdb,
ManyRecords.yxdb) used for unit testing and as fallback benchmarks when
generated data is not available.

---

## 4. Sample Size and Runs

| Parameter | Default | Rationale |
|-----------|---------|-----------|
| **Warmup runs** | 10 | Stabilize OS page cache, Python import caches, .NET JIT compilation, Rust/C++ branch predictors |
| **Timed runs** | 100 (configurable, minimum 100) | Provide robust estimates of central tendency and spread |

With 100 runs:
- The **standard error of the mean** is approximately `stdev / 10`,
  giving roughly +/-1% precision on the mean for typical CV values.
- **Percentiles** (p5, p25, p75, p95) are estimated from 5+ data points
  per tail, sufficient for distribution characterization.
- **Outliers** from OS scheduling jitter, background processes, or GC
  pauses are visible in the spread without dominating the median.

The sample size can be increased via `--runs N` for higher precision.

---

## 5. Warmup Protocol

Before timed runs, each benchmark performs **10 warmup iterations** of
the identical read operation. This serves several purposes:

1. **OS page cache:** After the first read, the file's contents are in
   the operating system's page cache. All subsequent reads (including
   all 100+ timed runs) measure "hot cache" performance. This is the
   appropriate comparison because:
   - It isolates the library's processing speed from disk I/O latency.
   - Real-world repeated reads (e.g., benchmarking, testing, data
     pipelines re-reading reference data) hit the page cache.
   - Cold-start performance is dominated by storage hardware, not the
     library, making it a poor discriminator.

2. **Python caches:** First imports resolve module loading, JIT-style
   optimizations in the Python runtime (e.g., specializing adaptive
   interpreter in CPython 3.11+), and any internal memoization.

3. **.NET JIT:** The first invocations trigger JIT compilation of all
   code paths. By the time timed runs begin, all methods are compiled
   to native code.

4. **Rust, Go, and C++:** All are ahead-of-time compiled, so warmup
   primarily serves page-cache stabilization and CPU branch predictor
   training.

---

## 6. Garbage Collection Handling

### Python

- `gc.collect()` is called **once** before the timed loop to clear any
  accumulated garbage from warmup.
- `gc.disable()` is called to **prevent GC pauses during timed runs**.
- `gc.enable()` is called after all timed runs complete.

This prevents the non-deterministic GC from inflating individual run
timings. The objects allocated during timed runs are short-lived; the
bounded allocation pattern means disabling GC for the duration of the
timed loop does not cause unbounded memory growth.

### C# (.NET)

- `GC.Collect()` + `GC.WaitForPendingFinalizers()` + `GC.Collect()` is
  called between warmup and timed runs to start from a clean heap state.
- GC is **not** disabled during timed runs because:
  - .NET's GC cannot be fully disabled at the application level.
  - The row-by-row reader allocates many small objects (strings, boxed
    values); preventing GC would cause OOM on large files.
  - GC pauses are a real cost of the .NET runtime and should be
    reflected in measurements.

### Go

- Go's GC runs concurrently and cannot be meaningfully disabled for
  benchmarks without distorting memory behavior.
- The warmup runs stabilize the heap, and Go's low-latency GC design
  keeps pauses in the microsecond range, well below the measurement
  granularity for most files.

### Rust

- Rust uses ownership-based memory management with no garbage collector.
  Memory allocation and deallocation are deterministic. Each run
  allocates a new DataFrame; the previous one is dropped at the end of
  the timed loop iteration.

### C++

- C++ uses manual memory management (no garbage collector). There are
  no GC pauses to account for. Memory allocation and deallocation
  during each run is deterministic.

---

## 7. Statistical Output

For each (target, file) pair, the benchmark reports:

| Metric | Definition |
|--------|-----------|
| **count** | Number of timed runs |
| **mean** | Arithmetic mean of all run times |
| **median** | 50th percentile (robust central tendency) |
| **stdev** | Sample standard deviation (Bessel-corrected, N-1) |
| **min / max** | Fastest / slowest individual run |
| **p5 / p95** | 5th and 95th percentiles (typical range) |
| **p25 / p75** | Interquartile range boundaries |
| **IQR** | p75 - p25 (robust measure of spread) |
| **CV** | Coefficient of variation (stdev / mean) |
| **throughput** | rows / median_time (rows per second) |

### Why Median for Primary Comparison

The **median** is used as the primary comparison metric rather than the
mean because:

- It is resistant to outliers (a single OS scheduling hiccup does not
  distort the result).
- For right-skewed timing distributions (common in benchmarks), the
  median better represents "typical" performance.
- The mean is still reported for completeness and for detecting
  systematic rightward skew (mean >> median indicates frequent slow
  runs).

### Coefficient of Variation (CV)

CV = stdev / mean. Typical values for these benchmarks:

| CV Range | Interpretation |
|----------|---------------|
| < 0.02 | Excellent stability — highly reproducible |
| 0.02-0.05 | Good stability — normal for user-space benchmarks |
| 0.05-0.10 | Moderate variability — check for background load |
| > 0.10 | High variability — results may not be reliable |

If CV exceeds 0.10, consider increasing the run count, closing
background applications, or running on an otherwise-idle machine.

---

## 8. Benchmark Scripts

### 8.1 Data Generation

```bash
python benchmarks/generate_benchmark_data.py
```

Generates 15 YXDB files in `benchmarks/data/` using deterministic
seeded random data. Supports `--rows` and `--profiles` flags to
generate a subset.

### 8.2 Python Multi-Format Benchmark

```bash
python benchmarks/benchmark_python_formats.py --runs 100
```

Benchmarks the 4 Python-accessible targets (`sigilyx-py-polars`,
`sigilyx-py-arrow`, `sigilyx-py-pandas`, `yxdb-py`) against all files
in `benchmarks/data/`. Outputs to `results_python_formats.json`.

### 8.3 Cross-Language Orchestrator

```bash
python benchmarks/benchmark_cross_language.py --runs 100
```

Orchestrates all 9 targets in sequence:

1. Runs the Python multi-format benchmark (subprocess)
2. Builds and runs the Rust benchmark binary (`cargo build --release`)
3. Builds and runs the Go benchmark binary
4. Builds and runs the .NET benchmark (`dotnet build -c Release`)
5. Builds and runs the NedHarding C++ benchmark
6. Builds and runs the Alteryx OpenYXDB C++ benchmark

Each step can be skipped with `--skip-rust`, `--skip-go`,
`--skip-dotnet`, `--skip-nedharding`, `--skip-alteryx-cpp`.

Produces a unified report with per-file detailed tables (target,
language, median, mean, CV, throughput, ratio vs. fastest) and a
throughput summary matrix across all targets and files.

---

## 9. Threats to Validity

### Internal Validity

| Threat | Mitigation |
|--------|-----------|
| **OS scheduling noise** | 100+ runs with median reporting; CV monitors stability |
| **GC pauses** | Python GC disabled; C#/.NET GC collected before runs |
| **JIT warmup** (.NET) | 10 warmup runs precede measurement |
| **Page cache misses** | 10 warmup runs populate cache; all timed runs are hot-cache |
| **Timer resolution** | Sub-microsecond timers vs. millisecond+ measurements |
| **Background processes** | User responsibility; CV flags instability |
| **Memory pressure** | Test files fit in RAM (< 65 MB each, 149 MB total) |

### External Validity

| Threat | Discussion |
|--------|-----------|
| **File size** | Largest test file is 65 MB / 100K rows. Results may not extrapolate to multi-GB files where memory allocation and I/O patterns differ. |
| **Schema shape** | Profiles range from 2 to 50 columns. Very wide schemas (500+ columns) may shift bottlenecks further toward field metadata handling. |
| **Data distribution** | Test data is synthetic with deterministic seed. Real-world data with different compression ratios, string lengths, or null rates may yield different speedups. |
| **Hardware dependence** | Results are specific to the test machine's CPU, RAM, and storage. ARM vs x86, SSD vs HDD, and cache hierarchy all affect relative performance. |
| **Cross-language overhead** | Python targets include Python runtime overhead (GIL, interpreter). Go, C#, Rust, and C++ run in their own processes with native execution. The comparison measures "what a user of language X experiences." |

### Construct Validity

The benchmark measures "read all data from file." It does not measure:

- **Selective reads** (column projection, row filtering) — no library
  supports this for YXDB.
- **Memory efficiency** — peak RSS is not tracked. A library that uses
  2x memory but runs 10% faster would look better in this benchmark.
- **Write performance** — separate benchmark needed.
- **API ergonomics** — not quantifiable.
- **Correctness** — the benchmark does not verify that all libraries
  produce identical output for the same input. Correctness should be
  validated separately.

---

## 10. Reproduction

### Prerequisites

```bash
# Python (required)
pip install polars yxdb               # yxdb-py + polars
pip install pyarrow pandas            # optional: for arrow/pandas targets
maturin develop --release             # build sigilyx

# Generate benchmark data (required)
python benchmarks/generate_benchmark_data.py

# Rust (optional — detected automatically)
# Requires Rust toolchain (cargo) on PATH

# Go (optional)
# Requires Go 1.21+ on PATH
cd benchmarks/go && go mod tidy

# .NET (optional)
# Requires .NET SDK 8.0+ on PATH
cd benchmarks/dotnet && dotnet restore

# C++ - NedHarding (optional, Windows only)
# Requires Visual Studio Build Tools 2022 (MSVC)
cd benchmarks/cpp && build.bat

# C++ - Alteryx OpenYXDB (optional, Windows only)
# Requires Visual Studio Build Tools 2022 (MSVC) + CMake
cd benchmarks/cpp && build_alteryx.bat
```

### Running

```bash
# Full cross-language benchmark (all 9 targets)
python benchmarks/benchmark_cross_language.py --runs 100

# Python-only multi-format benchmark (4 targets)
python benchmarks/benchmark_python_formats.py --runs 100

# Single file, more runs
python benchmarks/benchmark_cross_language.py --runs 500 --files bench_numeric_100000.yxdb

# Skip specific targets
python benchmarks/benchmark_cross_language.py --skip-go --skip-dotnet

# Custom data directory
python benchmarks/benchmark_cross_language.py --data-dir /path/to/data
```

### Interpreting Output

- **JSON results** are written to `benchmarks/results_python_formats.json`
  and `benchmarks/results_cross_language.json`.
- **"vs fastest"** shows how each target compares to the fastest target
  for each file. A value of `5.0x` means that target took 5x longer
  than the fastest.
- **Throughput** is `rows / median_time` in rows per second.
- Check **CV** values: if any exceed 0.10, the machine may have been
  under load and results should be treated with caution.

---

## 11. Known Limitations

1. **yxdb-py output format:** yxdb-py has no DataFrame output mode.
   The benchmark calls `reader.next()` and `reader.read_index(i)` for
   every field, which is the library's intended usage pattern. Values
   are extracted but not accumulated into a columnar structure.

2. **Go / C# do not accumulate columnar output.** These benchmarks
   extract typed values per field per row but do not build a columnar
   structure (no Go/C# equivalent of DataFrame). This is consistent
   with each library's intended usage pattern.

3. **Alteryx OpenYXDB** (`github.com/alteryx/OpenYXDB`) is the
   official Alteryx open-source YXDB reader, licensed under a custom
   open-source license. Its benchmark is built with CMake+MSVC and
   measures pure C++ row-by-row iteration speed.

4. **NedHarding's Open_AlteryxYXDB** (`github.com/AlteryxNed/Open_AlteryxYXDB`)
   is a fork with modifications, licensed under GPL-3.0. Its benchmark
   is built with MSVC and measures pure C++ row-by-row iteration speed.

5. **sigilyx Python output format overhead:** The three Python output
   formats (Polars, Arrow, Pandas) share the same Rust read path. The
   differences are:
   - **Polars:** Arrow IPC -> Polars `from_arrow` (near zero-copy)
   - **Arrow:** Arrow IPC -> PyArrow `RecordBatchStreamReader` (zero-copy)
   - **Pandas:** Arrow IPC -> PyArrow -> `.to_pandas()` (copy + type conversion)

   The Pandas target is expected to be slower due to the arrow-to-pandas
   conversion overhead.

6. **PyArrow and Pandas are optional.** If not installed, the
   `sigilyx-py-arrow` and `sigilyx-py-pandas` targets are skipped
   automatically.

7. **Single-threaded.** All benchmarks are single-threaded. Libraries
   that could benefit from multi-threaded decompression or parsing are
   not given that advantage.
