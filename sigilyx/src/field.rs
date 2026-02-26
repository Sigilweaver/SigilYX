/// Field types supported by the YXDB format.
///
/// Each variant corresponds to an Alteryx data type as stored in the XML
/// metadata header of a YXDB file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Bool,
    Byte,
    Int16,
    Int32,
    Int64,
    Float,
    Double,
    FixedDecimal,
    String,
    WString,
    VString,
    VWString,
    Date,
    Time,
    DateTime,
    Blob,
    SpatialObj,
}

impl std::fmt::Display for FieldType {
    /// Display the canonical YXDB XML name for this field type.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_xml_str())
    }
}

impl std::str::FromStr for FieldType {
    type Err = ();

    /// Parse a field type from the XML `type` attribute string.
    fn from_str(s: &str) -> std::result::Result<Self, ()> {
        match s {
            "Bool" => Ok(FieldType::Bool),
            "Byte" => Ok(FieldType::Byte),
            "Int16" => Ok(FieldType::Int16),
            "Int32" => Ok(FieldType::Int32),
            "Int64" => Ok(FieldType::Int64),
            "Float" => Ok(FieldType::Float),
            "Double" => Ok(FieldType::Double),
            "FixedDecimal" => Ok(FieldType::FixedDecimal),
            "String" => Ok(FieldType::String),
            "WString" => Ok(FieldType::WString),
            "V_String" => Ok(FieldType::VString),
            "V_WString" => Ok(FieldType::VWString),
            "Date" => Ok(FieldType::Date),
            "Time" => Ok(FieldType::Time),
            "DateTime" => Ok(FieldType::DateTime),
            "Blob" => Ok(FieldType::Blob),
            "SpatialObj" => Ok(FieldType::SpatialObj),
            _ => Err(()),
        }
    }
}

impl FieldType {
    /// Parse a field type from the XML `type` attribute string.
    pub fn from_xml_str(s: &str) -> Option<FieldType> {
        s.parse().ok()
    }

    /// Return the canonical YXDB XML attribute name for this type.
    pub fn as_xml_str(&self) -> &'static str {
        match self {
            FieldType::Bool => "Bool",
            FieldType::Byte => "Byte",
            FieldType::Int16 => "Int16",
            FieldType::Int32 => "Int32",
            FieldType::Int64 => "Int64",
            FieldType::Float => "Float",
            FieldType::Double => "Double",
            FieldType::FixedDecimal => "FixedDecimal",
            FieldType::String => "String",
            FieldType::WString => "WString",
            FieldType::VString => "V_String",
            FieldType::VWString => "V_WString",
            FieldType::Date => "Date",
            FieldType::Time => "Time",
            FieldType::DateTime => "DateTime",
            FieldType::Blob => "Blob",
            FieldType::SpatialObj => "SpatialObj",
        }
    }

    /// Returns the number of fixed bytes this field type occupies in a record
    /// buffer, given the field's declared size.
    ///
    /// For variable-length types (V_String, V_WString, Blob, SpatialObj), the
    /// fixed portion is always 4 bytes (a u32 pointer/inline indicator).
    pub fn fixed_bytes(&self, size: usize) -> usize {
        match self {
            FieldType::Bool => 1,
            FieldType::Byte => 1 + 1,             // value + null byte
            FieldType::Int16 => 2 + 1,            // value + null byte
            FieldType::Int32 => 4 + 1,            // value + null byte
            FieldType::Int64 => 8 + 1,            // value + null byte
            FieldType::Float => 4 + 1,            // value + null byte
            FieldType::Double => 8 + 1,           // value + null byte
            FieldType::FixedDecimal => size + 1,  // ASCII digits + null byte
            FieldType::String => size + 1,        // ASCII chars + null byte
            FieldType::WString => (size * 2) + 1, // UTF-16 chars + null byte
            FieldType::VString => 4,              // u32 pointer
            FieldType::VWString => 4,             // u32 pointer
            FieldType::Date => 10 + 1,            // "YYYY-MM-DD" + null byte
            FieldType::Time => 8 + 1,             // "HH:MM:SS" + null byte
            FieldType::DateTime => 19 + 1,        // "YYYY-MM-DD HH:MM:SS" + null byte
            FieldType::Blob => 4,                 // u32 pointer
            FieldType::SpatialObj => 4,           // u32 pointer
        }
    }

    /// Returns true if this is a variable-length field type.
    pub fn is_variable(&self) -> bool {
        matches!(
            self,
            FieldType::VString | FieldType::VWString | FieldType::Blob | FieldType::SpatialObj
        )
    }
}

/// Metadata for a single field (column) in a YXDB file.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldMeta {
    /// Column name.
    pub name: String,
    /// Field data type.
    pub field_type: FieldType,
    /// Declared size (meaning varies by type: max chars for strings, precision
    /// for FixedDecimal, etc.).
    pub size: usize,
    /// Scale (only meaningful for FixedDecimal).
    pub scale: usize,
    /// Byte offset of this field within the fixed portion of a record.
    pub offset: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_field_types() {
        let types = [
            ("Bool", FieldType::Bool),
            ("Byte", FieldType::Byte),
            ("Int16", FieldType::Int16),
            ("Int32", FieldType::Int32),
            ("Int64", FieldType::Int64),
            ("Float", FieldType::Float),
            ("Double", FieldType::Double),
            ("FixedDecimal", FieldType::FixedDecimal),
            ("String", FieldType::String),
            ("WString", FieldType::WString),
            ("V_String", FieldType::VString),
            ("V_WString", FieldType::VWString),
            ("Date", FieldType::Date),
            ("Time", FieldType::Time),
            ("DateTime", FieldType::DateTime),
            ("Blob", FieldType::Blob),
            ("SpatialObj", FieldType::SpatialObj),
        ];
        for (s, expected) in types {
            assert_eq!(FieldType::from_xml_str(s), Some(expected), "failed for {s}");
        }
    }

    #[test]
    fn unknown_type_returns_none() {
        assert_eq!(FieldType::from_xml_str("Unknown"), None);
    }
}
