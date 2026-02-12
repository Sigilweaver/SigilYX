// Alteryx OpenYXDB Write Benchmark
//
// Benchmarks write performance of the official Alteryx OpenYXDB (C++) library.
// Reads a source YXDB file into memory, then times writing it back out
// repeatedly to fresh temp files.
//
// Usage:
//     alteryx_openyxdb_write_benchmark.exe <source_yxdb> [runs]
//
// Methodology:
//   - Reads all records from source file into memory (not timed).
//   - Uses QueryPerformanceCounter (sub-microsecond resolution, monotonic).
//   - Performs 10 warmup write cycles before measurement.
//   - Each timed run: Create() -> AppendRecord() x N -> Close().
//   - Temp files deleted between runs (outside timer).
//   - Reports results as JSON on stdout.

#ifndef _CRT_SECURE_NO_WARNINGS
#define _CRT_SECURE_NO_WARNINGS
#endif

#ifdef _WIN32
#include <windows.h>
#endif

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <memory>
#include <string>
#include <vector>

#ifndef _WIN32
#include <chrono>
#endif

#include "SrcLib_Replacement.h"
#include "FieldType.h"
#include "Open_AlteryxYXDB.h"

static const int WARMUP_RUNS = 10;

struct Timer {
#ifdef _WIN32
    LARGE_INTEGER freq, start_time;
    Timer() { QueryPerformanceFrequency(&freq); }
    void start() { QueryPerformanceCounter(&start_time); }
    double elapsed_seconds() {
        LARGE_INTEGER end_time;
        QueryPerformanceCounter(&end_time);
        return (double)(end_time.QuadPart - start_time.QuadPart) / (double)freq.QuadPart;
    }
#else
    std::chrono::high_resolution_clock::time_point start_time;
    Timer() {}
    void start() { start_time = std::chrono::high_resolution_clock::now(); }
    double elapsed_seconds() {
        auto end = std::chrono::high_resolution_clock::now();
        return std::chrono::duration<double>(end - start_time).count();
    }
#endif
};

struct Stats {
    int count;
    double mean_s, median_s, stdev_s, min_s, max_s;
    double p5_s, p25_s, p75_s, p95_s, iqr_s, cv;
};

Stats compute_stats(std::vector<double>& times) {
    int n = (int)times.size();
    std::sort(times.begin(), times.end());

    double sum = 0;
    for (double t : times) sum += t;
    double mean = sum / n;

    double median;
    if (n % 2 == 0)
        median = (times[n/2 - 1] + times[n/2]) / 2.0;
    else
        median = times[n/2];

    double variance = 0;
    for (double t : times) {
        double d = t - mean;
        variance += d * d;
    }
    if (n > 1) variance /= (n - 1);
    double stdev = sqrt(variance);

    auto percentile = [&](double p) -> double {
        int idx = (int)(n * p);
        if (idx < 0) idx = 0;
        if (idx >= n) idx = n - 1;
        return times[idx];
    };

    double p5 = percentile(0.05);
    double p25 = percentile(0.25);
    double p75 = percentile(0.75);
    double p95 = percentile(0.95);
    double cv_val = mean > 0 ? stdev / mean : 0;

    return {
        n, mean, median, stdev, times[0], times[n-1],
        p5, p25, p75, p95, p75 - p25, cv_val
    };
}

std::wstring to_wstring(const char* s) {
    std::wstring ws;
    while (*s) ws += (wchar_t)*s++;
    return ws;
}

std::string extract_filename(const char* path) {
    std::string s(path);
    size_t pos = s.find_last_of("/\\");
    if (pos != std::string::npos) return s.substr(pos + 1);
    return s;
}

long long get_file_size(const char* path) {
    FILE* f = fopen(path, "rb");
    if (!f) return 0;
    _fseeki64(f, 0, SEEK_END);
    long long size = _ftelli64(f);
    fclose(f);
    return size;
}

// Generate a temp file path for a given run index
std::string make_temp_path(const char* source_path, int run_idx) {
    // Use the source directory + _bench_write_N.yxdb
    std::string s(source_path);
    size_t dot = s.rfind('.');
    if (dot == std::string::npos) dot = s.size();
    char buf[32];
    snprintf(buf, sizeof(buf), "_bench_write_%d.yxdb", run_idx);
    return s.substr(0, dot) + buf;
}

void delete_file(const char* path) {
#ifdef _WIN32
    DeleteFileA(path);
#else
    remove(path);
#endif
}

// Write all records to a new YXDB file
void write_all_records(
    const wchar_t* out_path,
    const U16unit* record_info_xml,
    const std::vector<std::vector<uint8_t>>& records
) {
    Alteryx::OpenYXDB::Open_AlteryxYXDB file;
    SRC::WString wpath(out_path);
    file.Create(wpath, record_info_xml);

    for (const auto& rec_bytes : records) {
        const SRC::RecordData* pRec = reinterpret_cast<const SRC::RecordData*>(rec_bytes.data());
        file.AppendRecord(pRec);
    }

    file.Close();
}

int main(int argc, char* argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <source_yxdb> [runs]\n", argv[0]);
        fprintf(stderr, "Benchmarks write performance using the Alteryx OpenYXDB C++ library.\n");
        return 1;
    }

    const char* source_path = argv[1];
    int runs = argc >= 3 ? atoi(argv[2]) : 50;
    std::wstring source_wpath = to_wstring(source_path);

    try {
        // --- Phase 1: Read source file into memory (not timed) ---
        fprintf(stderr, "Loading source file into memory...\n");

        Alteryx::OpenYXDB::Open_AlteryxYXDB source_file;
        SRC::WString wpath(source_wpath.c_str());
        source_file.Open(wpath);

        unsigned num_fields = source_file.m_recordInfo.NumFields();

        // Get the RecordInfo XML for creating output files
        SRC::String record_info_xml = source_file.m_recordInfo.GetRecordXmlMetaData();

        // Read all records into memory as raw byte copies
        std::vector<std::vector<uint8_t>> records;
        records.reserve(1000000); // pre-allocate for large files

        long long num_rows = 0;
        while (const SRC::RecordData* pRec = source_file.ReadRecord()) {
            size_t rec_len = source_file.m_recordInfo.GetRecordLen(pRec);
            const uint8_t* raw = reinterpret_cast<const uint8_t*>(pRec);
            records.emplace_back(raw, raw + rec_len);
            num_rows++;
        }
        source_file.Close();

        long long source_file_size = get_file_size(source_path);
        std::string fname = extract_filename(source_path);

        fprintf(stderr, "Loaded %lld records (%u fields) from %s\n",
                num_rows, num_fields, source_path);
        fprintf(stderr, "In-memory record data: %.1f MB\n",
                [&]() {
                    size_t total = 0;
                    for (const auto& r : records) total += r.size();
                    return total / 1048576.0;
                }());

        // --- Phase 2: Warmup ---
        fprintf(stderr, "Warming up (%d write cycles)...\n", WARMUP_RUNS);
        for (int i = 0; i < WARMUP_RUNS; ++i) {
            std::string tmp_path = make_temp_path(source_path, 9000 + i);
            std::wstring tmp_wpath = to_wstring(tmp_path.c_str());
            write_all_records(tmp_wpath.c_str(), record_info_xml.c_str(), records);
            delete_file(tmp_path.c_str());
        }

        // --- Phase 3: Timed runs ---
        fprintf(stderr, "Running %d timed write iterations...\n", runs);

        Timer timer;
        std::vector<double> times;
        times.reserve(runs);
        long long output_file_size = 0;

        for (int i = 0; i < runs; ++i) {
            std::string tmp_path = make_temp_path(source_path, i);
            std::wstring tmp_wpath = to_wstring(tmp_path.c_str());

            timer.start();
            write_all_records(tmp_wpath.c_str(), record_info_xml.c_str(), records);
            double elapsed = timer.elapsed_seconds();

            times.push_back(elapsed);

            // Capture output file size from last run
            if (i == runs - 1) {
                output_file_size = get_file_size(tmp_path.c_str());
            }

            // Clean up temp file (outside timer)
            delete_file(tmp_path.c_str());
        }

        // --- Phase 4: Compute stats and output JSON ---
        Stats stats = compute_stats(times);
        double throughput_rows = stats.median_s > 0 ? num_rows / stats.median_s : 0;
        double throughput_mb = stats.median_s > 0
            ? (output_file_size / 1048576.0) / stats.median_s : 0;

        printf("{\n");
        printf("  \"library\": \"alteryx-openyxdb\",\n");
        printf("  \"version\": \"main (2024)\",\n");
        printf("  \"language\": \"C++\",\n");
        printf("  \"file\": \"%s\",\n", fname.c_str());
        printf("  \"rows\": %lld,\n", num_rows);
        printf("  \"cols\": %u,\n", num_fields);
        printf("  \"file_size_bytes\": %lld,\n", source_file_size);
        printf("  \"output_file_size_bytes\": %lld,\n", output_file_size);
        printf("  \"input_type\": \"C++ RecordData (in-memory)\",\n");
        printf("  \"output_type\": \"write\",\n");
        printf("  \"throughput_rows_per_s\": %.2f,\n", throughput_rows);
        printf("  \"throughput_mb_per_s\": %.2f,\n", throughput_mb);
        printf("  \"count\": %d,\n", stats.count);
        printf("  \"mean_s\": %.9f,\n", stats.mean_s);
        printf("  \"median_s\": %.9f,\n", stats.median_s);
        printf("  \"stdev_s\": %.9f,\n", stats.stdev_s);
        printf("  \"min_s\": %.9f,\n", stats.min_s);
        printf("  \"max_s\": %.9f,\n", stats.max_s);
        printf("  \"p5_s\": %.9f,\n", stats.p5_s);
        printf("  \"p25_s\": %.9f,\n", stats.p25_s);
        printf("  \"p75_s\": %.9f,\n", stats.p75_s);
        printf("  \"p95_s\": %.9f,\n", stats.p95_s);
        printf("  \"iqr_s\": %.9f,\n", stats.iqr_s);
        printf("  \"cv\": %.9f\n", stats.cv);
        printf("}\n");

        fprintf(stderr, "Done. Median: %.6f s, Throughput: %.0f rows/s (%.1f MB/s)\n",
                stats.median_s, throughput_rows, throughput_mb);

    } catch (const SRC::Error& e) {
        fprintf(stderr, "ERROR (SRC::Error): %s\n",
                SRC::ConvertToAString(e.GetErrorDescription()).c_str());
        return 2;
    } catch (const std::exception& e) {
        fprintf(stderr, "ERROR (std::exception): %s\n", e.what());
        return 2;
    } catch (...) {
        fprintf(stderr, "ERROR: Unknown exception\n");
        return 2;
    }

    return 0;
}
