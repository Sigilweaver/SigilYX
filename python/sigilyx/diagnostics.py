"""Privacy-aware diagnostics for troubleshooting YXDB reads.

The report deliberately excludes the source path, file name, row values, and
raw file bytes. Schema names and full exception text are opt-in because they
can contain sensitive business information.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import re
import sys
from collections import Counter
from pathlib import Path
from typing import Any, List, Optional, Union

from sigilyx._readers import read_schema, read_yxdb, record_count

_E1_MAGIC = b"Alteryx Database File"
_E2_MAGIC = b"Alteryx e2 Database file"
_RECORD_NUMBER = re.compile(r"\brecord (\d+)\b", re.IGNORECASE)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _format(path: Path) -> str:
    with path.open("rb") as stream:
        magic = stream.read(64)
    if magic.startswith(_E2_MAGIC):
        return "E2"
    if magic.startswith(_E1_MAGIC):
        return "E1"
    return "unrecognized"


def _redact_path(message: str, path: Path) -> str:
    """Remove the supplied path from an error string before it is reported."""
    candidates = {str(path), str(path.absolute()), str(path.resolve())}
    for candidate in sorted(candidates, key=len, reverse=True):
        if candidate:
            message = message.replace(candidate, "<input path>")
    return message


def _failure(
    exc: Exception, path: Path, include_error_details: bool
) -> dict[str, Any]:
    report: dict[str, Any] = {
        "status": "error",
        "exception_type": type(exc).__name__,
    }
    match = _RECORD_NUMBER.search(str(exc))
    if match is not None:
        report["record_number"] = int(match.group(1))
    if include_error_details:
        report["error_message"] = _redact_path(str(exc), path)
    return report


def diagnose_yxdb(
    path: Union[str, Path],
    *,
    include_schema: bool = False,
    include_error_details: bool = False,
) -> dict[str, Any]:
    """Return a structured, privacy-aware diagnostic report for a YXDB file.

    The default report excludes the input path, file name, row values, raw
    record bytes, schema names, and full error text. Set ``include_schema``
    or ``include_error_details`` only after reviewing the disclosure risk.
    The report does decode the file to reproduce a read failure, but it never
    includes decoded values in its result.
    """
    source = Path(path)
    report: dict[str, Any] = {
        "report_version": 1,
        "privacy": {
            "includes_input_path": False,
            "includes_file_name": False,
            "includes_row_values": False,
            "includes_raw_file_bytes": False,
            "includes_schema_names": include_schema,
            "includes_full_error_messages": include_error_details,
        },
        "environment": {
            "sigilyx_version": _version(),
            "python_version": platform.python_version(),
            "python_implementation": platform.python_implementation(),
            "operating_system": platform.system(),
            "operating_system_release": platform.release(),
            "architecture": platform.machine(),
        },
        "file": {},
        "operations": {},
    }

    try:
        report["file"]["size_bytes"] = source.stat().st_size
    except OSError as exc:
        report["file"]["size_bytes"] = _failure(exc, source, include_error_details)

    try:
        report["file"]["sha256"] = _sha256(source)
    except OSError as exc:
        report["file"]["sha256"] = _failure(exc, source, include_error_details)

    try:
        report["file"]["format"] = _format(source)
    except OSError as exc:
        report["file"]["format"] = _failure(exc, source, include_error_details)

    try:
        schema = read_schema(source)
        type_counts = dict(sorted(Counter(field["type"] for field in schema).items()))
        schema_report: dict[str, Any] = {
            "status": "ok",
            "field_count": len(schema),
            "field_type_counts": type_counts,
        }
        if include_schema:
            schema_report["fields"] = schema
        report["operations"]["read_schema"] = schema_report
    except Exception as exc:  # Native errors map to several Python exception types.
        report["operations"]["read_schema"] = _failure(exc, source, include_error_details)

    try:
        report["operations"]["record_count"] = {
            "status": "ok",
            "value": record_count(source),
        }
    except Exception as exc:  # Native errors map to several Python exception types.
        report["operations"]["record_count"] = _failure(exc, source, include_error_details)

    try:
        dataframe = read_yxdb(source, spatial="raw")
        report["operations"]["decode"] = {
            "status": "ok",
            "row_count": dataframe.height,
            "column_count": dataframe.width,
        }
    except Exception as exc:  # Native errors map to several Python exception types.
        report["operations"]["decode"] = _failure(exc, source, include_error_details)

    return report


def _version() -> str:
    # Import lazily to avoid a circular import while sigilyx.__init__ loads us.
    from sigilyx import __version__

    return __version__


def main(argv: Optional[List[str]] = None) -> int:
    """Write a diagnostic report to stdout for ``python -m sigilyx.diagnostics``."""
    parser = argparse.ArgumentParser(description="Create a privacy-aware SigilYX diagnostic report.")
    parser.add_argument("path", help="Path to the YXDB file to inspect.")
    parser.add_argument(
        "--include-schema",
        action="store_true",
        help="Include schema names, sizes, and scales. Review before sharing.",
    )
    parser.add_argument(
        "--include-error-details",
        action="store_true",
        help="Include complete parser errors, which can include field names. Review before sharing.",
    )
    args = parser.parse_args(argv)
    report = diagnose_yxdb(
        args.path,
        include_schema=args.include_schema,
        include_error_details=args.include_error_details,
    )
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
