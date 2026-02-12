// yxdb-go Read Benchmark
//
// Benchmarks the yxdb-go library's read performance on YXDB files.
// Outputs JSON results to stdout for consumption by the cross-language
// benchmark orchestrator.
//
// Usage:
//
//	go run main.go -file path/to/file.yxdb -runs 100
//
// Methodology:
//   - Reads all records and all fields from the file per run.
//   - Uses time.Now() / time.Since() (monotonic clock, nanosecond resolution).
//   - Performs 10 warmup runs before measurement.
//   - Reports results as JSON on stdout for machine-readable consumption.
//   - Each run opens the file, iterates all records, extracts all field
//     values (by index), and closes the reader.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"math"
	"os"
	"sort"
	"time"

	yxdb "github.com/tlarsendataguy-yxdb/yxdb-go"
	"github.com/tlarsendataguy-yxdb/yxdb-go/yxrecord"
)

const warmupRuns = 10

// Stats holds descriptive statistics for a set of timing measurements.
type Stats struct {
	Count   int     `json:"count"`
	MeanS   float64 `json:"mean_s"`
	MedianS float64 `json:"median_s"`
	StdevS  float64 `json:"stdev_s"`
	MinS    float64 `json:"min_s"`
	MaxS    float64 `json:"max_s"`
	P5S     float64 `json:"p5_s"`
	P25S    float64 `json:"p25_s"`
	P75S    float64 `json:"p75_s"`
	P95S    float64 `json:"p95_s"`
	IqrS    float64 `json:"iqr_s"`
	CV      float64 `json:"cv"`
}

// Result holds the full benchmark result for JSON output.
type Result struct {
	Library          string  `json:"library"`
	Language         string  `json:"language"`
	File             string  `json:"file"`
	Rows             int64   `json:"rows"`
	Cols             int     `json:"cols"`
	FileSizeBytes    int64   `json:"file_size_bytes"`
	OutputType       string  `json:"output_type"`
	ThroughputRowsPS float64 `json:"throughput_rows_per_s"`
	Stats
}

func computeStats(times []float64) Stats {
	n := len(times)
	if n == 0 {
		return Stats{}
	}

	sorted := make([]float64, n)
	copy(sorted, times)
	sort.Float64s(sorted)

	sum := 0.0
	for _, t := range sorted {
		sum += t
	}
	mean := sum / float64(n)

	var median float64
	if n%2 == 0 {
		median = (sorted[n/2-1] + sorted[n/2]) / 2.0
	} else {
		median = sorted[n/2]
	}

	variance := 0.0
	for _, t := range sorted {
		d := t - mean
		variance += d * d
	}
	if n > 1 {
		variance /= float64(n - 1)
	}
	stdev := math.Sqrt(variance)

	percentile := func(p float64) float64 {
		idx := int(float64(n) * p)
		if idx < 0 {
			idx = 0
		}
		if idx >= n {
			idx = n - 1
		}
		return sorted[idx]
	}

	p5 := percentile(0.05)
	p25 := percentile(0.25)
	p75 := percentile(0.75)
	p95 := percentile(0.95)

	cv := 0.0
	if mean > 0 {
		cv = stdev / mean
	}

	return Stats{
		Count:   n,
		MeanS:   mean,
		MedianS: median,
		StdevS:  stdev,
		MinS:    sorted[0],
		MaxS:    sorted[n-1],
		P5S:     p5,
		P25S:    p25,
		P75S:    p75,
		P95S:    p95,
		IqrS:    p75 - p25,
		CV:      cv,
	}
}

// readAllRecords opens a YXDB file, iterates all records, extracts all
// field values, and returns the record count and column count.
func readAllRecords(filePath string) (int64, int, error) {
	reader, err := yxdb.ReadFile(filePath)
	if err != nil {
		return 0, 0, fmt.Errorf("failed to open %s: %w", filePath, err)
	}

	fields := reader.ListFields()
	numCols := len(fields)
	var numRows int64

	for reader.Next() {
		// Extract every field value by index to match what other benchmarks do.
		// yxdb-go exposes a DataType enum (int), not string type names.
		for i, field := range fields {
			switch field.Type {
			case yxrecord.Byte:
				reader.ReadByteWithIndex(i)
			case yxrecord.Int64:
				reader.ReadInt64WithIndex(i)
			case yxrecord.Float64:
				reader.ReadFloat64WithIndex(i)
			case yxrecord.Boolean:
				reader.ReadBoolWithIndex(i)
			case yxrecord.String:
				reader.ReadStringWithIndex(i)
			case yxrecord.Date:
				reader.ReadTimeWithIndex(i)
			case yxrecord.Blob:
				reader.ReadBlobWithIndex(i)
			default:
				reader.ReadStringWithIndex(i)
			}
		}
		numRows++
	}

	return numRows, numCols, nil
}

func main() {
	filePath := flag.String("file", "", "Path to the YXDB file to benchmark")
	runs := flag.Int("runs", 100, "Number of timed runs")
	flag.Parse()

	if *filePath == "" {
		fmt.Fprintln(os.Stderr, "Usage: go run main.go -file path/to/file.yxdb [-runs N]")
		os.Exit(1)
	}

	fileInfo, err := os.Stat(*filePath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "ERROR: Cannot stat file: %v\n", err)
		os.Exit(1)
	}

	// Warmup
	fmt.Fprintf(os.Stderr, "Warming up (%d runs)...\n", warmupRuns)
	var totalRows int64
	var totalCols int
	for i := 0; i < warmupRuns; i++ {
		rows, cols, err := readAllRecords(*filePath)
		if err != nil {
			fmt.Fprintf(os.Stderr, "ERROR during warmup: %v\n", err)
			os.Exit(1)
		}
		totalRows = rows
		totalCols = cols
	}

	fmt.Fprintf(os.Stderr, "File: %s (%d rows x %d cols)\n", *filePath, totalRows, totalCols)
	fmt.Fprintf(os.Stderr, "Running %d timed iterations...\n", *runs)

	// Timed runs
	times := make([]float64, 0, *runs)
	for i := 0; i < *runs; i++ {
		start := time.Now()
		_, _, err := readAllRecords(*filePath)
		elapsed := time.Since(start).Seconds()
		if err != nil {
			fmt.Fprintf(os.Stderr, "ERROR during run %d: %v\n", i, err)
			os.Exit(1)
		}
		times = append(times, elapsed)
	}

	stats := computeStats(times)

	throughput := 0.0
	if stats.MedianS > 0 {
		throughput = float64(totalRows) / stats.MedianS
	}

	result := Result{
		Library:          "yxdb-go",
		Language:         "Go",
		File:             fileInfo.Name(),
		Rows:             totalRows,
		Cols:             totalCols,
		FileSizeBytes:    fileInfo.Size(),
		OutputType:       "row-by-row (typed values)",
		ThroughputRowsPS: throughput,
		Stats:            stats,
	}

	jsonBytes, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		fmt.Fprintf(os.Stderr, "ERROR marshaling JSON: %v\n", err)
		os.Exit(1)
	}

	fmt.Println(string(jsonBytes))
	fmt.Fprintf(os.Stderr, "Done. Median: %.6f s, Throughput: %.0f rows/s\n",
		stats.MedianS, throughput)
}
