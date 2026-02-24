---
sidebar_position: 3
---

# Pandas

SigilYX can read and write YXDB files as Pandas DataFrames. Data flows through PyArrow under the hood, so you need the `[pandas]` extra installed.

## Installation

```bash
pip install sigilyx[pandas]
```

## Reading

```python
import sigilyx as yx

df = yx.read_yxdb_pandas("data.yxdb")
print(type(df))  # <class 'pandas.core.frame.DataFrame'>
```

The read path is: Rust reader -> Arrow arrays -> PyArrow Table -> Pandas DataFrame. Despite the conversion steps, this is still orders of magnitude faster than pure-Python YXDB readers.

## Writing

```python
import sigilyx as yx
import pandas as pd

df = pd.read_csv("data.csv")
yx.write_yxdb_pandas("output.yxdb", df)
```

Pandas DataFrames are converted to PyArrow Tables before being passed to the Rust writer.

## Type Mapping

| Pandas dtype | YXDB Type | Notes |
| --- | --- | --- |
| `bool` | Boolean | |
| `int16` | Int16 | |
| `int32` | Int32 | |
| `int64` | Int64 | |
| `float32` | Float | |
| `float64` | Double | |
| `object` (strings) | V_WString | Variable-length UTF-16 |
| `datetime64[ns]` | DateTime | Converted to microseconds |
| `object` (date) | Date | |
| `object` (time) | Time | `datetime.time` values |
| `object` (Decimal) | FixedDecimal | `decimal.Decimal` values |
| `bytes` | Blob | Raw binary data |

:::tip Prefer Polars for performance
The Polars path (`read_yxdb()`) is the fastest because it avoids the Pandas conversion overhead. If you're starting a new project and don't have an existing Pandas dependency, consider using Polars directly. See the [Polars guide](/python/polars).
:::

## Common Patterns

### Read YXDB, process with Pandas, write back

```python
import sigilyx as yx

df = yx.read_yxdb_pandas("input.yxdb")

# Standard Pandas operations
df["total"] = df["quantity"] * df["price"]
df = df[df["total"] > 100]
df = df.sort_values("total", ascending=False)

yx.write_yxdb_pandas("output.yxdb", df)
```

### Convert YXDB to CSV

```python
import sigilyx as yx

df = yx.read_yxdb_pandas("data.yxdb")
df.to_csv("data.csv", index=False)
```

### Convert Excel to YXDB

```python
import sigilyx as yx
import pandas as pd

df = pd.read_excel("data.xlsx")
yx.write_yxdb_pandas("data.yxdb", df)
```

### Interop with scikit-learn

```python
import sigilyx as yx
from sklearn.model_selection import train_test_split
from sklearn.linear_model import LinearRegression

df = yx.read_yxdb_pandas("training_data.yxdb")

X = df[["feature_1", "feature_2", "feature_3"]]
y = df["target"]

X_train, X_test, y_train, y_test = train_test_split(X, y, test_size=0.2)
model = LinearRegression().fit(X_train, y_train)
print(f"R² = {model.score(X_test, y_test):.3f}")
```
