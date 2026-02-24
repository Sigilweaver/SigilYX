---
sidebar_position: 7
---

# Metadata

SigilYX can inspect YXDB files without reading any row data. This is useful for understanding file structure, validating schemas, or building data catalogs.

## Field Metadata

```python
import sigilyx as yx

fields = yx.read_yxdb_fields("data.yxdb")

for f in fields:
    print(f"{f.name:20s}  {f.field_type:15s}  size={f.size}")
```

Each field object has the following attributes:

| Attribute | Type | Description |
| --- | --- | --- |
| `name` | `str` | Column name |
| `field_type` | `str` | YXDB type name (e.g., `"V_WString"`, `"Int64"`) |
| `size` | `int` | Width for fixed-size types, max length for variable types |
| `scale` | `int` | Decimal places (only for `FixedDecimal`) |

## Record Count

Get the total number of records without reading any data:

```python
n = yx.record_count("data.yxdb")
print(f"{n:,} records")
```

This reads only the file header (512 bytes).

## Use Cases

### Schema comparison

```python
import sigilyx as yx

def get_schema(path):
    return {f.name: f.field_type for f in yx.read_yxdb_fields(path)}

schema_a = get_schema("file_a.yxdb")
schema_b = get_schema("file_b.yxdb")

added = set(schema_b) - set(schema_a)
removed = set(schema_a) - set(schema_b)
changed = {k for k in schema_a if k in schema_b and schema_a[k] != schema_b[k]}

print(f"Added: {added}")
print(f"Removed: {removed}")
print(f"Type changed: {changed}")
```

### File inventory

```python
import sigilyx as yx
from pathlib import Path

for yxdb in Path("data/").glob("**/*.yxdb"):
    n = yx.record_count(str(yxdb))
    fields = yx.read_yxdb_fields(str(yxdb))
    print(f"{yxdb.name:30s}  {n:>10,} rows  {len(fields):>3} cols")
```

### Validate before processing

```python
import sigilyx as yx

fields = yx.read_yxdb_fields("upload.yxdb")
field_names = {f.name for f in fields}

required = {"customer_id", "order_date", "amount"}
missing = required - field_names

if missing:
    raise ValueError(f"Missing required columns: {missing}")
```
