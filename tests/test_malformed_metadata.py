"""Malformed field attributes in E1 metadata raise ValueError."""

import struct

import pytest

import sigilyx


def _e1_file(tmp_path, field_xml: str) -> str:
    xml = f"<RecordInfo>{field_xml}</RecordInfo>"
    meta = xml.encode("utf-16-le") + b"\x00\x00"
    header = bytearray(512)
    header[:21] = b"Alteryx Database File"
    struct.pack_into("<I", header, 80, len(meta) // 2)
    path = tmp_path / "bad_attr.yxdb"
    path.write_bytes(bytes(header) + meta)
    return str(path)


@pytest.mark.parametrize(
    "field_xml, expected",
    [
        ('<Field name="N" type="V_WString" size="abc" />', "size"),
        ('<Field name="N" type="String" size="-5" />', "size"),
        ('<Field name="N" type="String" size="" />', "size"),
        ('<Field name="A" type="FixedDecimal" size="19" scale="x" />', "scale"),
        ('<Field name="A&bogus;" type="Int32" />', "name"),
        ('<Field type="Int32" />', "name"),
        ('<Field name="N" />', "type"),
    ],
)
def test_malformed_field_attribute_raises_value_error(tmp_path, field_xml, expected):
    path = _e1_file(tmp_path, field_xml)
    with pytest.raises(ValueError, match=f"attribute {expected}"):
        sigilyx.read_yxdb(path)
