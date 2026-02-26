// SigilYX Pure Rust Read Benchmark
//
// Measures native Rust read performance using sigilyx directly,
// without any Python/PyO3 overhead.
//
// Usage:
//     sigilyx_rust_benchmark.exe -file path/to/file.yxdb -runs 100 [-mode columnar|row]
//
// Modes:
//   - columnar (default): Calls sigilyx::read_yxdb(path) which returns a Polars DataFrame.
//   - row: Uses YxdbRowReader for row-by-row iteration with read_all() per record.
//
// Methodology:
//   - Uses std::time::Instant (monotonic, nanosecond resolution).
//   - 10 warmup runs, then N timed runs.
//   - columnar: opens file, parses header+metadata, decompresses LZF blocks,
//     extracts all fields, builds columnar Arrow arrays, returns DataFrame.
//   - row: opens file, iterates all records with next() + read_all(),
//     extracts typed FieldValue per field per row.

use serde::Serialize;
use std::env;
use std::path::Path;
use std::time::Instant;

const WARMUP_RUNS: usize = 10;

#[derive(Serialize)]
struct BenchResult {
    library: String,
    version: String,
    language: String,
    file: String,
    rows: usize,
    cols: usize,
    file_size_bytes: u64,
    output_type: String,
    throughput_rows_per_s: f64,
    count: usize,
    mean_s: f64,
    median_s: f64,
    stdev_s: f64,
    min_s: f64,
    max_s: f64,
    p5_s: f64,
    p25_s: f64,
    p75_s: f64,
    p95_s: f64,
    iqr_s: f64,
    cv: f64,
}

fn compute_stats(times: &mut [f64]) -> (f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64) {
    let n = times.len();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let sum: f64 = times.iter().sum();
    let mean = sum / n as f64;

    let median = if n.is_multiple_of(2) {
        (times[n / 2 - 1] + times[n / 2]) / 2.0
    } else {
        times[n / 2]
    };

    let variance: f64 =
        times.iter().map(|t| (t - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0).max(1.0);
    let stdev = variance.sqrt();

    let percentile = |p: f64| -> f64 {
        let idx = ((n as f64) * p) as usize;
        times[idx.min(n - 1)]
    };

    let p5 = percentile(0.05);
    let p25 = percentile(0.25);
    let p75 = percentile(0.75);
    let p95 = percentile(0.95);
    let cv = if mean > 0.0 { stdev / mean } else { 0.0 };

    (
        mean,
        median,
        stdev,
        times[0],
        times[n - 1],
        p5,
        p25,
        p75,
        p95,
        p75 - p25,
        cv,
    )
}

/// Run the columnar (DataFrame) benchmark.
fn bench_columnar(file_path: &str, runs: usize) -> (usize, usize, Vec<f64>) {
    let mut total_rows: usize = 0;
    let mut total_cols: usize = 0;

    // Warmup
    eprintln!("Mode: columnar (DataFrame)");
    eprintln!("Warming up ({} runs)...", WARMUP_RUNS);
    for _ in 0..WARMUP_RUNS {
        let df =
            sigilyx::read_yxdb(file_path, sigilyx::SpatialMode::Raw).expect("Failed to read YXDB");
        total_rows = df.height();
        total_cols = df.width();
        drop(df);
    }

    eprintln!("Running {} timed iterations...", runs);
    let mut times: Vec<f64> = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let df =
            sigilyx::read_yxdb(file_path, sigilyx::SpatialMode::Raw).expect("Failed to read YXDB");
        let elapsed = start.elapsed().as_secs_f64();
        drop(df);
        times.push(elapsed);
    }

    (total_rows, total_cols, times)
}

/// Run the row-by-row benchmark.
fn bench_row(file_path: &str, runs: usize) -> (usize, usize, Vec<f64>) {
    let mut total_rows: usize = 0;
    let mut total_cols: usize = 0;

    // Warmup
    eprintln!("Mode: row (row-by-row)");
    eprintln!("Warming up ({} runs)...", WARMUP_RUNS);
    for _ in 0..WARMUP_RUNS {
        let mut reader = sigilyx::YxdbRowReader::open(file_path).expect("Failed to open YXDB");
        total_cols = reader.fields().len();
        let mut rows = 0usize;
        while reader.next().expect("Failed to read next record") {
            let _vals = reader.read_all().expect("Failed to read all fields");
            rows += 1;
        }
        total_rows = rows;
    }

    eprintln!("Running {} timed iterations...", runs);
    let mut times: Vec<f64> = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let mut reader = sigilyx::YxdbRowReader::open(file_path).expect("Failed to open YXDB");
        while reader.next().expect("Failed to read next record") {
            let _vals = reader.read_all().expect("Failed to read all fields");
        }
        let elapsed = start.elapsed().as_secs_f64();
        times.push(elapsed);
    }

    (total_rows, total_cols, times)
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut file_path: Option<String> = None;
    let mut runs: usize = 100;
    let mut mode = "columnar".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-file" if i + 1 < args.len() => {
                file_path = Some(args[i + 1].clone());
                i += 2;
            }
            "-runs" if i + 1 < args.len() => {
                runs = args[i + 1].parse().unwrap_or(100);
                i += 2;
            }
            "-mode" if i + 1 < args.len() => {
                mode = args[i + 1].clone();
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let file_path = match file_path {
        Some(p) => p,
        None => {
            eprintln!("Usage: sigilyx_rust_benchmark -file path/to/file.yxdb [-runs N] [-mode columnar|row]");
            std::process::exit(1);
        }
    };

    let path = Path::new(&file_path);
    if !path.exists() {
        eprintln!("ERROR: File not found: {}", file_path);
        std::process::exit(1);
    }

    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let file_name = path.file_name().unwrap().to_string_lossy().to_string();

    eprintln!("File: {}", file_path);

    let (total_rows, total_cols, mut times) = match mode.as_str() {
        "columnar" => bench_columnar(&file_path, runs),
        "row" => bench_row(&file_path, runs),
        _ => {
            eprintln!("ERROR: Unknown mode '{}'. Use 'columnar' or 'row'.", mode);
            std::process::exit(1);
        }
    };

    eprintln!("{} rows x {} cols", total_rows, total_cols);

    let (mean, median, stdev, min, max, p5, p25, p75, p95, iqr, cv) = compute_stats(&mut times);

    let throughput = if median > 0.0 {
        total_rows as f64 / median
    } else {
        0.0
    };

    let (library, output_type) = match mode.as_str() {
        "row" => (
            "sigilyx (row)".to_string(),
            "Vec<FieldValue> (row-by-row)".to_string(),
        ),
        _ => (
            "sigilyx".to_string(),
            "Polars DataFrame (columnar)".to_string(),
        ),
    };

    let result = BenchResult {
        library,
        version: env!("CARGO_PKG_VERSION").to_string(),
        language: "Rust (native)".to_string(),
        file: file_name,
        rows: total_rows,
        cols: total_cols,
        file_size_bytes: file_size,
        output_type,
        throughput_rows_per_s: throughput,
        count: runs,
        mean_s: mean,
        median_s: median,
        stdev_s: stdev,
        min_s: min,
        max_s: max,
        p5_s: p5,
        p25_s: p25,
        p75_s: p75,
        p95_s: p95,
        iqr_s: iqr,
        cv,
    };

    let json = serde_json::to_string_pretty(&result).expect("Failed to serialize JSON");
    println!("{}", json);

    eprintln!(
        "Done. Median: {:.6} s, Throughput: {:.0} rows/s",
        median, throughput
    );
}
