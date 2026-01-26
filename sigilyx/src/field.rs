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

impl FieldType {
    /// Parse a field type from the XML `type` attribute string.
    pub fn from_str(s: &str) -> Option<FieldType> {
        match s {
            "Bool" => Some(FieldType::Bool),
            "Byte" => Some(FieldType::Byte),
            "Int16" => Some(FieldType::Int16),
            "Int32" => Some(FieldType::Int32),
            "Int64" => Some(FieldType::Int64),
            "Float" => Some(FieldType::Float),
            "Double" => Some(FieldType::Double),
            "FixedDecimal" => Some(FieldType::FixedDecimal),
            "String" => Some(FieldType::String),
            "WString" => Some(FieldType::WString),
            "V_String" => Some(FieldType::VString),
            "V_WString" => Some(FieldType::VWString),
            "Date" => Some(FieldType::Date),
            "Time" => Some(FieldType::Time),
            "DateTime" => Some(FieldType::DateTime),
            "Blob" => Some(FieldType::Blob),
            "SpatialObj" => Some(FieldType::SpatialObj),
            _ => None,
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
            FieldType::Byte => 1 + 1,           // value + null byte
            FieldType::Int16 => 2 + 1,           // value + null byte
            FieldType::Int32 => 4 + 1,           // value + null byte
            FieldType::Int64 => 8 + 1,           // value + null byte
            FieldType::Float => 4 + 1,           // value + null byte
            FieldType::Double => 8 + 1,          // value + null byte
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
#[derive(Debug, Clone)]
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
            assert_eq!(FieldType::from_str(s), Some(expected), "failed for {s}");
        }
    }

    #[test]
    fn unknown_type_returns_none() {
        assert_eq!(FieldType::from_str("Unknown"), None);
    }
}
