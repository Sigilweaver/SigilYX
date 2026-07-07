//! Memory benchmark harness (not part of the public API).
//!
//! Generates a large synthetic YXDB file, then reads it back via the eager
//! `into_dataframe()` path. Run under `/usr/bin/time -v` to observe peak RSS:
//!
//!   cargo run --release --example mem_bench -- generate /tmp/bench.yxdb 4000000
//!   /usr/bin/time -v cargo run --release --example mem_bench -- read /tmp/bench.yxdb
use polars::prelude::*;
use sigilyx::{write_yxdb, YxdbReader};
use std::env;
use std::time::Instant;

/// Peak resident set size (VmHWM), in bytes, from /proc/self/status.
fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let kb: u64 = rest.trim().trim_end_matches(" kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

fn report_peak_rss() {
    match peak_rss_bytes() {
        Some(b) => eprintln!("peak RSS (VmHWM): {:.1} MiB", b as f64 / (1024.0 * 1024.0)),
        None => eprintln!("peak RSS: unavailable (not Linux?)"),
    }
}

fn make_fixed_df(n: usize) -> DataFrame {
    let ids: Vec<i64> = (0..n as i64).collect();
    let a: Vec<i32> = (0..n as i32).map(|i| i.wrapping_mul(7)).collect();
    let b: Vec<f64> = (0..n).map(|i| (i as f64) * 2.5).collect();
    df!("id" => ids, "a" => a, "b" => b).unwrap()
}

fn make_df(n: usize) -> DataFrame {
    let ids: Vec<i64> = (0..n as i64).collect();
    let values: Vec<f64> = (0..n).map(|i| (i as f64) * 1.0001).collect();
    let names: Vec<String> = (0..n)
        .map(|i| format!("record-name-{i:08}-payload"))
        .collect();
    let notes: Vec<String> = (0..n)
        .map(|i| format!("some longer descriptive text field for row {i} used to simulate a realistic wide string column with enough bytes to matter"))
        .collect();
    df!(
        "id" => ids,
        "value" => values,
        "name" => names,
        "notes" => notes,
    )
    .unwrap()
}

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("generate") => {
            let path = args.get(2).expect("path");
            let n: usize = args.get(3).expect("n").parse().expect("n is usize");
            let t0 = Instant::now();
            let df = make_df(n);
            eprintln!("generated {n} rows in {:?}", t0.elapsed());
            let t1 = Instant::now();
            write_yxdb(path, &df, &[]).unwrap();
            eprintln!("wrote {path} in {:?}", t1.elapsed());
        }
        Some("read") => {
            let path = args.get(2).expect("path");
            let t0 = Instant::now();
            let reader = YxdbReader::open(path).unwrap();
            let n = reader.header.num_records;
            let df = reader.into_dataframe().unwrap();
            eprintln!(
                "read {} records ({} cols) in {:?}",
                n,
                df.width(),
                t0.elapsed()
            );
            report_peak_rss();

            // Correctness check: recompute every expected value independently
            // (not by re-deriving from the DataFrame itself) and compare. This
            // is what actually exercises the chunk carry-over logic across
            // internal chunk boundaries for a file this size.
            let n = n as usize;
            let id_col = df.column("id").unwrap().i64().unwrap();
            let value_col = df.column("value").unwrap().f64().unwrap();
            let name_col = df.column("name").unwrap().str().unwrap();
            let notes_col = df.column("notes").unwrap().str().unwrap();
            let mut id_sum: i128 = 0;
            for i in 0..n {
                let expected_id = i as i64;
                assert_eq!(id_col.get(i), Some(expected_id), "id mismatch at row {i}");
                id_sum += expected_id as i128;
                let expected_value = expected_id as f64 * 1.0001;
                assert!(
                    (value_col.get(i).unwrap() - expected_value).abs() < 1e-6,
                    "value mismatch at row {i}"
                );
                let expected_name = format!("record-name-{i:08}-payload");
                assert_eq!(
                    name_col.get(i),
                    Some(expected_name.as_str()),
                    "name mismatch at row {i}"
                );
                let expected_notes = format!("some longer descriptive text field for row {i} used to simulate a realistic wide string column with enough bytes to matter");
                assert_eq!(
                    notes_col.get(i),
                    Some(expected_notes.as_str()),
                    "notes mismatch at row {i}"
                );
            }
            let expected_sum: i128 = (0..n as i128).sum();
            assert_eq!(id_sum, expected_sum, "id checksum mismatch");
            eprintln!("verified {n} rows byte-for-byte against independently recomputed expected values - OK");
        }
        Some("generate-fixed") => {
            let path = args.get(2).expect("path");
            let n: usize = args.get(3).expect("n").parse().expect("n is usize");
            let df = make_fixed_df(n);
            write_yxdb(path, &df, &[]).unwrap();
            eprintln!("wrote fixed-size {path} ({n} rows)");
        }
        Some("read-fixed-verify") => {
            let path = args.get(2).expect("path");
            let reader = YxdbReader::open(path).unwrap();
            let n = reader.header.num_records as usize;
            let df = reader.into_dataframe().unwrap();
            let id_col = df.column("id").unwrap().i64().unwrap();
            let a_col = df.column("a").unwrap().i32().unwrap();
            let b_col = df.column("b").unwrap().f64().unwrap();
            for i in 0..n {
                assert_eq!(id_col.get(i), Some(i as i64), "id mismatch at {i}");
                assert_eq!(
                    a_col.get(i),
                    Some((i as i32).wrapping_mul(7)),
                    "a mismatch at {i}"
                );
                assert!(
                    (b_col.get(i).unwrap() - (i as f64) * 2.5).abs() < 1e-9,
                    "b mismatch at {i}"
                );
            }
            eprintln!("verified {n} fixed-size rows - OK");
            report_peak_rss();
        }
        Some("read-projected-verify") => {
            let path = args.get(2).expect("path");
            let reader = YxdbReader::open(path).unwrap();
            let n = reader.header.num_records as usize;
            let df = reader
                .into_dataframe_projected(Some(&["id", "notes"]))
                .unwrap();
            assert_eq!(df.width(), 2, "expected only 2 projected columns");
            let id_col = df.column("id").unwrap().i64().unwrap();
            let notes_col = df.column("notes").unwrap().str().unwrap();
            for i in 0..n {
                assert_eq!(id_col.get(i), Some(i as i64), "id mismatch at {i}");
                let expected_notes = format!("some longer descriptive text field for row {i} used to simulate a realistic wide string column with enough bytes to matter");
                assert_eq!(
                    notes_col.get(i),
                    Some(expected_notes.as_str()),
                    "notes mismatch at {i}"
                );
            }
            eprintln!("verified {n} rows with column projection (id, notes only) - OK");
            report_peak_rss();
        }
        Some("read-batches") => {
            let path = args.get(2).expect("path");
            let batch_size: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(65_536);
            let t0 = Instant::now();
            let mut reader = YxdbReader::open(path).unwrap();
            let mut total = 0usize;
            while let Some(batch) = reader.next_batch(batch_size, None).unwrap() {
                total += batch.height();
            }
            eprintln!("streamed {total} records in {:?}", t0.elapsed());
            report_peak_rss();
        }
        Some("read-batches-verify") => {
            let path = args.get(2).expect("path");
            let batch_size: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(65_536);
            let t0 = Instant::now();
            let mut reader = YxdbReader::open(path).unwrap();
            let mut i: usize = 0;
            while let Some(batch) = reader.next_batch(batch_size, None).unwrap() {
                let id_col = batch.column("id").unwrap().i64().unwrap();
                let value_col = batch.column("value").unwrap().f64().unwrap();
                let name_col = batch.column("name").unwrap().str().unwrap();
                let notes_col = batch.column("notes").unwrap().str().unwrap();
                for row in 0..batch.height() {
                    assert_eq!(id_col.get(row), Some(i as i64), "id mismatch at row {i}");
                    let expected_value = i as f64 * 1.0001;
                    assert!(
                        (value_col.get(row).unwrap() - expected_value).abs() < 1e-6,
                        "value mismatch at row {i}"
                    );
                    let expected_name = format!("record-name-{i:08}-payload");
                    assert_eq!(
                        name_col.get(row),
                        Some(expected_name.as_str()),
                        "name mismatch at row {i}"
                    );
                    let expected_notes = format!("some longer descriptive text field for row {i} used to simulate a realistic wide string column with enough bytes to matter");
                    assert_eq!(
                        notes_col.get(row),
                        Some(expected_notes.as_str()),
                        "notes mismatch at row {i}"
                    );
                    i += 1;
                }
            }
            eprintln!(
                "verified {i} streamed rows byte-for-byte in {:?} - OK",
                t0.elapsed()
            );
            report_peak_rss();
        }
        _ => {
            eprintln!(
                "usage: mem_bench <generate|read|read-batches|read-batches-verify> <path> [n|batch_size]"
            );
            std::process::exit(1);
        }
    }
}
