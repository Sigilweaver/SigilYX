use thiserror::Error;

/// Errors that can occur while reading a YXDB file.
#[derive(Debug, Error)]
pub enum YxdbError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid YXDB file: {0}")]
    InvalidFile(String),

    #[error("unsupported field type: {0}")]
    UnsupportedFieldType(String),

    #[error("XML metadata error: {0}")]
    XmlError(String),

    #[error(
        "invalid metadata attribute: field {field}, attribute {attribute}, \
         raw value {value:?}: {reason}"
    )]
    InvalidAttribute {
        /// Field name, or `#N` (zero-based position) when the name is unavailable.
        field: String,
        /// Attribute name, or `<attributes>` when the attribute list itself is malformed.
        attribute: String,
        /// Raw value as found in the file (the whole tag for a malformed attribute list).
        value: String,
        /// Why the value was rejected.
        reason: String,
    },

    #[error("LZF decompression error: {0}")]
    LzfError(String),

    #[error("data conversion error: {0}")]
    ConversionError(String),
}

pub type Result<T> = std::result::Result<T, YxdbError>;
