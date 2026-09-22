---
sidebar_position: 12
---

# Diagnostics

Use `diagnose_yxdb` to collect a structured report when a file does not read.
It hashes and decodes the local file, but its default output excludes the input
path, file name, row values, raw file bytes, schema names, and full parser
errors.

```python
import json
import sigilyx as yx

report = yx.diagnose_yxdb("data.yxdb")
print(json.dumps(report, indent=2, sort_keys=True))
```

The report includes the SigilYX and Python versions, operating system,
architecture, file size, SHA-256 hash, detected E1 or E2 format, schema field
count and type counts, record count, and the outcome of a full decode. A decode
failure includes its exception type and, when available, the record number.

Run the same report directly from a shell:

```bash
python -m sigilyx.diagnostics data.yxdb
```

## Sensitive details

Schema names and complete error messages can reveal business information. They
are omitted by default. If you decide they are safe to disclose, add the
appropriate flag, review the output, and only then share it.

```bash
python -m sigilyx.diagnostics data.yxdb --include-schema --include-error-details
```

The report never includes row values or raw file bytes.
