// Alteryx OpenYXDB Read Benchmark
//
// Benchmarks the official Alteryx OpenYXDB (C++) library's read performance.
// This is distinct from the NedHarding Open_AlteryxYXDB fork.
//
// Usage:
//     alteryx_openyxdb_benchmark.exe <path_to_yxdb> [runs]
//
// Methodology:
//   - Reads all records and all fields from the file per run.
//   - Uses QueryPerformanceCounter (sub-microsecond resolution, monotonic).
//   - Performs 10 warmup runs before measurement.
//   - Reports results as JSON on stdout.

// Must match the library's stdafx.h include order:
// 1. Windows headers first (provides HANDLE etc. needed by lzf_src.h)
// 2. Standard C++ headers
// 3. SrcLib_Replacement.h (provides SRC::String, SRC::WString)
// 4. Library headers last

#define _CRT_SECURE_NO_WARNINGS

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

struct BenchmarkResult {
    long long rows;
    int cols;
};

BenchmarkResult read_all_records(const wchar_t* file_path) {
    Alteryx::OpenYXDB::Open_AlteryxYXDB file;
    SRC::WString wpath(file_path);
    file.Open(wpath);

    unsigned num_fields = file.m_recordInfo.NumFields();
    long long num_rows = 0;

    while (const SRC::RecordData* pRec = file.ReadRecord()) {
        for (unsigned i = 0; i < num_fields; ++i) {
            const SRC::FieldBase* pField = file.m_recordInfo[i];

            switch (pField->m_ft) {
                case SRC::E_FT_Bool:
                    pField->GetAsBool(pRec);
                    break;
                case SRC::E_FT_Byte:
                case SRC::E_FT_Int16:
                case SRC::E_FT_Int32:
                    pField->GetAsInt32(pRec);
                    break;
                case SRC::E_FT_Int64:
                    pField->GetAsInt64(pRec);
                    break;
                case SRC::E_FT_Float:
                case SRC::E_FT_Double:
                case SRC::E_FT_FixedDecimal:
                    pField->GetAsDouble(pRec);
                    break;
                case SRC::E_FT_String:
                case SRC::E_FT_V_String:
                    pField->GetAsAString(pRec);
                    break;
                case SRC::E_FT_WString:
                case SRC::E_FT_V_WString:
                case SRC::E_FT_Date:
                case SRC::E_FT_Time:
                case SRC::E_FT_DateTime:
                    pField->GetAsWString(pRec);
                    break;
                case SRC::E_FT_Blob:
                case SRC::E_FT_SpatialObj:
                    pField->GetAsBlob(pRec);
                    break;
                default:
                    pField->GetAsWString(pRec);
                    break;
            }
        }
        num_rows++;
    }

    file.Close();
    return { num_rows, (int)num_fields };
}

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

int main(int argc, char* argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <path_to_yxdb> [runs]\n", argv[0]);
        return 1;
    }

    const char* file_path_narrow = argv[1];
    int runs = argc >= 3 ? atoi(argv[2]) : 100;
    std::wstring file_path = to_wstring(file_path_narrow);

    try {
    // Warmup
    fprintf(stderr, "Warming up (%d runs)...\n", WARMUP_RUNS);
    BenchmarkResult info = { 0, 0 };
    for (int i = 0; i < WARMUP_RUNS; ++i) {
        info = read_all_records(file_path.c_str());
    }

    fprintf(stderr, "File: %s (%lld rows x %d cols)\n", file_path_narrow, info.rows, info.cols);
    fprintf(stderr, "Running %d timed iterations...\n", runs);

    Timer timer;
    std::vector<double> times;
    times.reserve(runs);

    for (int i = 0; i < runs; ++i) {
        timer.start();
        read_all_records(file_path.c_str());
        double elapsed = timer.elapsed_seconds();
        times.push_back(elapsed);
    }

    Stats stats = compute_stats(times);
    double throughput = stats.median_s > 0 ? info.rows / stats.median_s : 0;
    long long file_size = get_file_size(file_path_narrow);
    std::string fname = extract_filename(file_path_narrow);

    printf("{\n");
    printf("  \"library\": \"alteryx-openyxdb\",\n");
    printf("  \"version\": \"main (2024)\",\n");
    printf("  \"language\": \"C++\",\n");
    printf("  \"file\": \"%s\",\n", fname.c_str());
    printf("  \"rows\": %lld,\n", info.rows);
    printf("  \"cols\": %d,\n", info.cols);
    printf("  \"file_size_bytes\": %lld,\n", file_size);
    printf("  \"output_type\": \"row-by-row (typed values)\",\n");
    printf("  \"throughput_rows_per_s\": %.2f,\n", throughput);
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

    fprintf(stderr, "Done. Median: %.6f s, Throughput: %.0f rows/s\n",
            stats.median_s, throughput);

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
