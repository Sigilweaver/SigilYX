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

    #[error("LZF decompression error: {0}")]
    LzfError(String),

    #[error("data conversion error: {0}")]
    ConversionError(String),
}

pub type Result<T> = std::result::Result<T, YxdbError>;
