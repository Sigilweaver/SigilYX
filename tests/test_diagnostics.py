"""Tests for the privacy-aware diagnostic report."""

import json

import polars as pl

import sigilyx
from sigilyx.diagnostics import main


def _sample_file(tmp_path):
    path = tmp_path / "confidential_customer_export.yxdb"
    sigilyx.write_yxdb(
        path,
        pl.DataFrame(
            {
                "Customer Secret Name": ["Alice Private"],
                "Revenue": [12345],
            }
        ),
    )
    return path


def test_diagnostic_report_excludes_sensitive_data_by_default(tmp_path):
    path = _sample_file(tmp_path)

    report = sigilyx.diagnose_yxdb(path)
    serialized = json.dumps(report)

    assert report["privacy"] == {
        "includes_input_path": False,
        "includes_file_name": False,
        "includes_row_values": False,
        "includes_raw_file_bytes": False,
        "includes_schema_names": False,
        "includes_full_error_messages": False,
    }
    assert report["file"]["format"] == "E1"
    assert len(report["file"]["sha256"]) == 64
    assert report["operations"]["read_schema"] == {
        "status": "ok",
        "field_count": 2,
        "field_type_counts": {"Int64": 1, "V_String": 1},
    }
    assert report["operations"]["record_count"] == {"status": "ok", "value": 1}
    assert report["operations"]["decode"] == {
        "status": "ok",
        "row_count": 1,
        "column_count": 2,
    }
    assert str(path) not in serialized
    assert path.name not in serialized
    assert "Customer Secret Name" not in serialized
    assert "Alice Private" not in serialized


def test_diagnostic_report_can_include_schema_when_requested(tmp_path):
    path = _sample_file(tmp_path)

    report = sigilyx.diagnose_yxdb(path, include_schema=True)

    assert report["privacy"]["includes_schema_names"] is True
    assert report["operations"]["read_schema"]["fields"][0]["name"] == "Customer Secret Name"


def test_diagnostics_module_writes_json(tmp_path, capsys):
    path = _sample_file(tmp_path)

    assert main([str(path)]) == 0

    report = json.loads(capsys.readouterr().out)
    assert report["file"]["format"] == "E1"
    assert path.name not in json.dumps(report)


def test_diagnostic_error_details_redact_input_path(tmp_path):
    path = tmp_path / "do_not_disclose_this_name.yxdb"

    report = sigilyx.diagnose_yxdb(path, include_error_details=True)

    assert report["privacy"]["includes_full_error_messages"] is True
    serialized = json.dumps(report)
    assert str(path) not in serialized
    assert path.name not in serialized
