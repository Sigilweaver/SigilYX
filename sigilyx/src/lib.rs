//! # SigilYX
//!
//! A fast, safe Rust library for reading and writing Alteryx YXDB files, with native
//! Polars DataFrame integration.
//!
//! Not affiliated with Alteryx, Inc. "Alteryx" is a registered trademark of Alteryx, Inc.
//!
//! ## Quick Start
//!
//! ```no_run
//! use sigilyx::{read_yxdb, write_yxdb};
//!
//! // Read a YXDB file
//! let df = read_yxdb("path/to/file.yxdb").unwrap();
//! println!("{}", df);
//!
//! // Write a DataFrame to YXDB
//! write_yxdb("path/to/output.yxdb", &df).unwrap();
//! ```

pub mod error;
pub mod field;
pub mod header;
pub mod lzf;
pub mod record;
pub mod reader;
pub mod writer;

pub use error::{YxdbError, Result};
pub use field::{FieldType, FieldMeta};
pub use reader::YxdbReader;
pub use writer::{write_yxdb, write_yxdb_with_schema, write_yxdb_from_ipc, YxdbWriter};

use polars::prelude::*;
use std::path::Path;

/// Read a YXDB file and return a Polars DataFrame.
///
/// This is the primary entry point for most users. It reads the entire file
/// into memory as a columnar DataFrame.
///
/// # Errors
///
/// Returns [`YxdbError`] if the file cannot be opened, is not a valid YXDB
/// file, or contains unsupported field types.
pub fn read_yxdb<P: AsRef<Path>>(path: P) -> Result<DataFrame> {
    let reader = YxdbReader::open(path)?;
    reader.into_dataframe()
}

/// Read a YXDB file and return the DataFrame serialized as Arrow IPC bytes.
///
/// This is useful for cross-language interop (e.g. Python). The returned
/// bytes can be read by any Arrow-compatible library:
///
/// ```python
/// import polars as pl
/// df = pl.read_ipc(data)
/// ```
pub fn read_yxdb_to_ipc<P: AsRef<Path>>(path: P) -> Result<Vec<u8>> {
    let mut df = read_yxdb(path)?;
    let mut buf = Vec::new();
    IpcWriter::new(&mut buf)
        .finish(&mut df)
        .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
    Ok(buf)
}

/// Read a YXDB file in batches and return each batch as Arrow IPC bytes.
///
/// This enables streaming/memory-efficient processing of large files.
/// Each returned `Vec<u8>` is a complete IPC message containing up to
/// `batch_size` rows.
pub fn read_yxdb_to_ipc_batches<P: AsRef<Path>>(path: P, batch_size: usize) -> Result<Vec<Vec<u8>>> {
    let mut reader = YxdbReader::open(path)?;
    let mut batches = Vec::new();

    while let Some(mut df) = reader.next_batch(batch_size)? {
        let mut buf = Vec::new();
        IpcWriter::new(&mut buf)
            .finish(&mut df)
            .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
        batches.push(buf);
    }

    Ok(batches)
}

