"""YXDB field type mapping and metadata classes."""

from __future__ import annotations

import polars as pl


# -- YXDB field type → Polars dtype mapping --

# Maps the canonical YXDB XML type name (from FieldType::Display) to the
# corresponding Polars data type.
_YXDB_TYPE_MAP: dict[str, pl.DataType] = {
    "Bool": pl.Boolean,
    "Byte": pl.Int16,
    "Int16": pl.Int16,
    "Int32": pl.Int32,
    "Int64": pl.Int64,
    "Float": pl.Float32,
    "Double": pl.Float64,
    "FixedDecimal": pl.Decimal,   # precision/scale filled per-column
    "String": pl.String,
    "WString": pl.String,
    "V_String": pl.String,
    "V_WString": pl.String,
    "Date": pl.Date,
    "Time": pl.Time,
    "DateTime": pl.Datetime("us"),
    "Blob": pl.Binary,
    "SpatialObj": pl.Binary,
}


def _yxdb_schema_to_polars(schema_info: list[dict]) -> dict[str, pl.DataType]:
    """Convert YXDB field metadata (from Rust) to a Polars SchemaDict."""
    result: dict[str, pl.DataType] = {}
    for field in schema_info:
        name = field["name"]
        ft = field["type"]
        if ft == "FixedDecimal":
            result[name] = pl.Decimal(
                precision=field.get("size", 18),
                scale=field.get("scale", 0),
            )
        else:
            result[name] = _YXDB_TYPE_MAP.get(ft, pl.String)
    return result


class FieldInfo:
    """Metadata for a single field (column) in a YXDB file.

    Attributes
    ----------
    name : str
        Column name.
    field_type : str
        YXDB field type (e.g. 'Int32', 'V_WString', 'Date').
    size : int
        Declared size (max chars for strings, precision for decimals).
    scale : int
        Scale (decimal places for FixedDecimal, 0 otherwise).
    """

    __slots__ = ("name", "field_type", "size", "scale")

    def __init__(self, d: dict):
        self.name: str = d["name"]
        self.field_type: str = d["type"]
        self.size: int = d.get("size", 0)
        self.scale: int = d.get("scale", 0)

    def __repr__(self) -> str:
        parts = f"name={self.name!r}, type={self.field_type!r}"
        if self.size:
            parts += f", size={self.size}"
        if self.scale:
            parts += f", scale={self.scale}"
        return f"FieldInfo({parts})"

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, FieldInfo):
            return NotImplemented
        return (
            self.name == other.name
            and self.field_type == other.field_type
            and self.size == other.size
            and self.scale == other.scale
        )
