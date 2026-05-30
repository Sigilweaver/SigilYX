---
sidebar_position: 10
description: "Iterate over YXDB records one row at a time with the Python row reader."
---

# Row Reader

SigilYX provides a `YxdbRowReader` class for iterating over YXDB records one at a time. This is useful for streaming processing where you don't need a columnar DataFrame, or when working with custom processing pipelines.

:::tip
The columnar reader (`read_yxdb()`) is significantly faster than row-by-row iteration. Use the row reader only when you need per-record control or cannot hold even a single batch in memory.
:::

## Basic Usage

```python
import sigilyx as yx

reader = yx.YxdbRowReader("data.yxdb")

while reader.next():
    name = reader.read_name("Name")
    amount = reader.read_name("Amount")
    print(f"{name}: {amount}")

reader.close()
```

## Context Manager

The reader implements the context manager protocol for automatic cleanup:

```python
import sigilyx as yx

with yx.YxdbRowReader("data.yxdb") as reader:
    print(f"File has {reader.num_records:,} records")

    while reader.next():
        row = reader.read_dict()
        process(row)
```

## Iterator Protocol

`YxdbRowReader` is also a Python iterator, yielding tuples of field values:

```python
import sigilyx as yx

for row in yx.YxdbRowReader("data.yxdb"):
    print(row)  # (1, "Alice", 42.5, ...)
```

## Reading Fields

| Method | Returns | Description |
| --- | --- | --- |
| `reader.read_index(i)` | Single value | Read by column index (0-based) |
| `reader.read_name(name)` | Single value | Read by column name |
| `reader.read_all()` | `tuple` | All field values from the current record |
| `reader.read_dict()` | `dict` | All values as `{name: value}` |

## Metadata

Access file metadata without reading data:

```python
import sigilyx as yx

with yx.YxdbRowReader("data.yxdb") as reader:
    print(f"Records: {reader.num_records:,}")

    for field in reader.fields:
        print(f"  {field.name}: {field.field_type} (size={field.size})")
```

## Use Cases

### Write to a database row by row

```python
import sigilyx as yx
import sqlite3

conn = sqlite3.connect("output.db")
conn.execute("CREATE TABLE data (id INTEGER, name TEXT, amount REAL)")

with yx.YxdbRowReader("data.yxdb") as reader:
    for row in reader:
        conn.execute("INSERT INTO data VALUES (?, ?, ?)", row)

conn.commit()
```

### Filter and count without DataFrames

```python
import sigilyx as yx

count = 0
total = 0.0

with yx.YxdbRowReader("data.yxdb") as reader:
    while reader.next():
        status = reader.read_name("Status")
        if status == "active":
            count += 1
            total += reader.read_name("Amount")

print(f"{count} active records, total: {total:,.2f}")
```
