// yxdb-net Read Benchmark
//
// Benchmarks the yxdb-net (C#/.NET) library's read performance on YXDB files.
// Outputs JSON results to stdout for consumption by the cross-language
// benchmark orchestrator.
//
// Usage:
//     dotnet run -- -file path/to/file.yxdb -runs 100
//
// Methodology:
//   - Reads all records and all fields from the file per run.
//   - Uses System.Diagnostics.Stopwatch (high-resolution monotonic timer).
//   - Performs 10 warmup runs before measurement (covers JIT compilation).
//   - Forces GC.Collect() between warmup and timed runs.
//   - Reports results as JSON on stdout.
//   - Each run constructs a new YxdbReader, iterates all records,
//     extracts all field values, and closes the reader.

using System.Diagnostics;
using System.Text.Json;

const int WarmupRuns = 10;

// -- Argument parsing --------------------------------------------------------
string? filePath = null;
int runs = 100;

for (int i = 0; i < args.Length; i++)
{
    switch (args[i])
    {
        case "-file" when i + 1 < args.Length:
            filePath = args[++i];
            break;
        case "-runs" when i + 1 < args.Length:
            runs = int.Parse(args[++i]);
            break;
    }
}

if (string.IsNullOrEmpty(filePath))
{
    Console.Error.WriteLine("Usage: dotnet run -- -file path/to/file.yxdb [-runs N]");
    return 1;
}

if (!File.Exists(filePath))
{
    Console.Error.WriteLine($"ERROR: File not found: {filePath}");
    return 1;
}

var fileInfo = new FileInfo(filePath);

// -- ReadAllRecords ----------------------------------------------------------
(long rowCount, int colCount) ReadAllRecords(string path)
{
    var reader = new yxdb.YxdbReader(path);
    var fields = reader.ListFields();
    int numCols = fields.Count;
    long numRows = 0;

    while (reader.Next())
    {
        for (int j = 0; j < numCols; j++)
        {
            var field = fields[j];
            switch (field.Type)
            {
                case yxdb.YxdbField.DataType.Byte:
                    reader.ReadByte(j);
                    break;
                case yxdb.YxdbField.DataType.Long:
                    reader.ReadLong(j);
                    break;
                case yxdb.YxdbField.DataType.Double:
                    reader.ReadDouble(j);
                    break;
                case yxdb.YxdbField.DataType.Boolean:
                    reader.ReadBool(j);
                    break;
                case yxdb.YxdbField.DataType.String:
                    reader.ReadString(j);
                    break;
                case yxdb.YxdbField.DataType.Date:
                    reader.ReadDate(j);
                    break;
                case yxdb.YxdbField.DataType.Blob:
                    reader.ReadBlob(j);
                    break;
                default:
                    reader.ReadString(j);
                    break;
            }
        }
        numRows++;
    }

    reader.Close();
    return (numRows, numCols);
}

// -- Warmup ------------------------------------------------------------------
Console.Error.WriteLine($"Warming up ({WarmupRuns} runs)...");
long totalRows = 0;
int totalCols = 0;
for (int i = 0; i < WarmupRuns; i++)
{
    (totalRows, totalCols) = ReadAllRecords(filePath);
}

Console.Error.WriteLine($"File: {filePath} ({totalRows} rows x {totalCols} cols)");
Console.Error.WriteLine($"Running {runs} timed iterations...");

// Force GC after warmup to start timed runs from a clean state
GC.Collect();
GC.WaitForPendingFinalizers();
GC.Collect();

// -- Timed runs --------------------------------------------------------------
var sw = new Stopwatch();
var times = new double[runs];

for (int i = 0; i < runs; i++)
{
    sw.Restart();
    ReadAllRecords(filePath);
    sw.Stop();
    times[i] = sw.Elapsed.TotalSeconds;
}

// -- Statistics --------------------------------------------------------------
Array.Sort(times);
int n = times.Length;

double sum = 0;
foreach (var t in times) sum += t;
double mean = sum / n;

double median = n % 2 == 0
    ? (times[n / 2 - 1] + times[n / 2]) / 2.0
    : times[n / 2];

double variance = 0;
foreach (var t in times)
{
    double d = t - mean;
    variance += d * d;
}
if (n > 1) variance /= (n - 1);
double stdev = Math.Sqrt(variance);

double Percentile(double p)
{
    int idx = (int)(n * p);
    if (idx < 0) idx = 0;
    if (idx >= n) idx = n - 1;
    return times[idx];
}

double p5 = Percentile(0.05);
double p25 = Percentile(0.25);
double p75 = Percentile(0.75);
double p95 = Percentile(0.95);
double cv = mean > 0 ? stdev / mean : 0;
double throughput = median > 0 ? totalRows / median : 0;

// -- JSON output -------------------------------------------------------------
var result = new Dictionary<string, object>
{
    ["library"] = "yxdb-net",
    ["language"] = "C# (.NET)",
    ["file"] = fileInfo.Name,
    ["rows"] = totalRows,
    ["cols"] = totalCols,
    ["file_size_bytes"] = fileInfo.Length,
    ["output_type"] = "row-by-row (typed values)",
    ["throughput_rows_per_s"] = throughput,
    ["count"] = n,
    ["mean_s"] = mean,
    ["median_s"] = median,
    ["stdev_s"] = stdev,
    ["min_s"] = times[0],
    ["max_s"] = times[n - 1],
    ["p5_s"] = p5,
    ["p25_s"] = p25,
    ["p75_s"] = p75,
    ["p95_s"] = p95,
    ["iqr_s"] = p75 - p25,
    ["cv"] = cv,
};

var jsonOptions = new JsonSerializerOptions { WriteIndented = true };
Console.WriteLine(JsonSerializer.Serialize(result, jsonOptions));

Console.Error.WriteLine($"Done. Median: {median:F6} s, Throughput: {throughput:F0} rows/s");

return 0;
