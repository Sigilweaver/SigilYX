//! # SigilYX
//!
//! A fast, safe Rust library for reading and writing Alteryx YXDB files, with native
//! Polars DataFrame integration.
//!
//! Supports both the **E1** (original engine) and **E2** (AMP engine) YXDB layouts.
//! Format detection is automatic - E1 files begin with `"Alteryx Database File"`,
//! E2 files begin with `"Alteryx e2 Database file"`.
//!
//! ## Quick Start
//!
//! ```no_run
//! use sigilyx::{read_yxdb, write_yxdb, SpatialMode};
//!
//! // Read a YXDB file (SpatialObj columns decoded to WKB by default)
//! let df = read_yxdb("path/to/file.yxdb", SpatialMode::Wkb, false).unwrap();
//! println!("{}", df);
//!
//! // Write a DataFrame to YXDB (E1 format)
//! write_yxdb("path/to/output.yxdb", &df, &[]).unwrap();
//! ```

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod e1;
pub mod e2;
pub mod error;
pub mod field;
pub mod spatial;

pub use e1::header::{ID_WRIGLEYDB, ID_WRIGLEYDB_NO_SPATIAL_INDEX};
pub use e1::lzf::CompressionAlgorithm;
pub use e1::reader::{YxdbReader, YxdbRowReader};
pub use e1::record::FieldValue;
pub use e1::writer::{
    infer_schema as infer_schema_public, write_yxdb, write_yxdb_from_ipc,
    write_yxdb_from_ipc_spatial, write_yxdb_with_schema, YxdbWriter,
};
pub use e2::reader::E2Reader;
pub use error::{Result, YxdbError};
pub use field::{FieldMeta, FieldType};
pub use spatial::{shp_to_wkb, spatial_column_names, wkb_to_shp, SpatialMode};

use polars::prelude::*;
use std::path::Path;

/// Detected YXDB file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YxdbFormat {
    /// E1 (original engine) - 512-byte header, UTF-16LE XML, LZF compression.
    E1,
    /// E2 (AMP engine) - 100-byte header, UTF-8 XML, Snappy compression.
    E2,
}

/// Detect whether a YXDB file is E1 or E2 by reading the magic string.
pub fn detect_format<P: AsRef<Path>>(path: P) -> Result<YxdbFormat> {
    use std::io::Read;
    let mut file = std::fs::File::open(path.as_ref())?;
    let mut buf = [0u8; 64];
    match file.read_exact(&mut buf) {
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(YxdbError::InvalidFile(
                "file too small to be a valid YXDB".into(),
            ));
        }
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }

    if buf.starts_with(e2::header::MAGIC) {
        Ok(YxdbFormat::E2)
    } else if buf.starts_with(e1::header::MAGIC) {
        Ok(YxdbFormat::E1)
    } else {
        Err(YxdbError::InvalidFile(
            "file does not start with a recognized YXDB magic string".into(),
        ))
    }
}

/// Deserialize Arrow IPC bytes to a Polars DataFrame.
///
/// Utility for cross-language interop when consumers need to round-trip
/// through IPC bytes and then call `write_yxdb_with_schema`.
pub fn ipc_to_dataframe(ipc_bytes: &[u8]) -> Result<DataFrame> {
    let cursor = std::io::Cursor::new(ipc_bytes);
    IpcReader::new(cursor)
        .finish()
        .map_err(|e| YxdbError::ConversionError(format!("failed to read IPC bytes: {e}")))
}

/// Read a YXDB file and return a Polars DataFrame.
///
/// `spatial` controls how `SpatialObj` columns are returned:
///
/// - [`SpatialMode::Wkb`] - decode SHP → ISO WKB (compatible with
///   Shapely, GeoPandas, PostGIS, GDAL).
/// - [`SpatialMode::Raw`] - keep the raw SHP bytes for expert use.
///
/// `allow_unverified_e2_types` controls whether to attempt reading E2 files
/// that contain field types whose decoders have never been verified against
/// real data (Time, WString, Blob, SpatialObj). Defaults to `false` (safe).
///
/// # Errors
///
/// Returns [`YxdbError`] if the file cannot be opened, is not a valid YXDB
/// file, or contains unsupported field types.
pub fn read_yxdb<P: AsRef<Path>>(
    path: P,
    spatial: SpatialMode,
    allow_unverified_e2_types: bool,
) -> Result<DataFrame> {
    let path = path.as_ref();
    match detect_format(path)? {
        YxdbFormat::E1 => {
            let reader = YxdbReader::open(path)?;
            let fields = reader.fields.clone();
            let df = reader.into_dataframe()?;
            apply_spatial_read(df, &fields, spatial)
        }
        YxdbFormat::E2 => {
            let mut reader = E2Reader::open(path)?;
            reader.set_allow_unverified(allow_unverified_e2_types);
            let fields = reader.fields.clone();
            let df = reader.into_dataframe()?;
            apply_spatial_read(df, &fields, spatial)
        }
    }
}

/// Read a YXDB file, returning only the specified columns.
///
/// Faster than [`read_yxdb`] when you only need a subset, because it
/// skips parsing and allocating unused fields.
///
/// # Errors
///
/// Returns [`YxdbError`] if the file cannot be opened, is not valid,
/// or if any requested column name does not exist in the file.
pub fn read_yxdb_columns<P: AsRef<Path>>(
    path: P,
    columns: &[&str],
    spatial: SpatialMode,
    allow_unverified_e2_types: bool,
) -> Result<DataFrame> {
    let path = path.as_ref();
    match detect_format(path)? {
        YxdbFormat::E1 => {
            let reader = YxdbReader::open(path)?;
            let fields = reader.fields.clone();
            let df = reader.into_dataframe_projected(Some(columns))?;
            apply_spatial_read(df, &fields, spatial)
        }
        YxdbFormat::E2 => {
            // E2 reader doesn't support column projection yet;
            // read all columns and then select the requested ones
            let mut reader = E2Reader::open(path)?;
            reader.set_allow_unverified(allow_unverified_e2_types);
            let fields = reader.fields.clone();
            let df = reader.into_dataframe()?;
            let df = apply_spatial_read(df, &fields, spatial)?;
            let col_strs: Vec<PlSmallStr> = columns.iter().map(|c| PlSmallStr::from(*c)).collect();
            df.select(col_strs.as_slice())
                .map_err(|e| YxdbError::ConversionError(e.to_string()))
        }
    }
}

/// Apply spatial post-processing to a DataFrame based on the chosen mode.
fn apply_spatial_read(
    df: DataFrame,
    fields: &[FieldMeta],
    spatial: SpatialMode,
) -> Result<DataFrame> {
    match spatial {
        SpatialMode::Raw => Ok(df),
        SpatialMode::Wkb => spatial::convert_spatial_columns_to_wkb(df, fields),
        // GeoArrow mode: decode SHP → WKB (same as Wkb mode at the Rust/Polars level).
        // The GeoArrow extension type metadata (`geoarrow.wkb`) is applied in the
        // Python layer when converting to PyArrow, because Polars DataFrames do not
        // natively support Arrow extension types.
        SpatialMode::GeoArrow => spatial::convert_spatial_columns_to_wkb(df, fields),
    }
}

/// Read a YXDB file and return the DataFrame serialized as Arrow IPC bytes.
///
/// This is useful for cross-language interop (e.g. Python). The returned
/// bytes can be read by any Arrow-compatible library.
pub fn read_yxdb_to_ipc<P: AsRef<Path>>(
    path: P,
    spatial: SpatialMode,
    allow_unverified_e2_types: bool,
) -> Result<Vec<u8>> {
    let mut df = read_yxdb(path, spatial, allow_unverified_e2_types)?;
    let mut buf = Vec::new();
    IpcWriter::new(&mut buf)
        .finish(&mut df)
        .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
    Ok(buf)
}

/// Read a YXDB file in batches and return each batch as Arrow IPC bytes.
///
/// Each returned `Vec<u8>` is a complete IPC message containing up to
/// `batch_size` rows.
pub fn read_yxdb_to_ipc_batches<P: AsRef<Path>>(
    path: P,
    batch_size: usize,
    spatial: SpatialMode,
) -> Result<Vec<Vec<u8>>> {
    let mut reader = YxdbReader::open(path.as_ref())?;
    let fields = reader.fields.clone();
    let mut batches = Vec::new();

    while let Some(df) = reader.next_batch(batch_size, None)? {
        let mut df = apply_spatial_read(df, &fields, spatial)?;
        let mut buf = Vec::new();
        IpcWriter::new(&mut buf)
            .finish(&mut df)
            .map_err(|e| YxdbError::ConversionError(e.to_string()))?;
        batches.push(buf);
    }

    Ok(batches)
}

// -- Streaming Writer IPC Helpers --

use std::fs::File;
use std::io::BufWriter;

/// Create a streaming YXDB writer from Arrow IPC schema bytes.
///
/// The IPC bytes should contain at least one batch (used to infer schema).
/// Returns a writer that can accept additional IPC batches.
pub fn create_writer_from_ipc<P: AsRef<Path>>(
    path: P,
    schema_ipc: &[u8],
) -> Result<YxdbWriter<BufWriter<File>>> {
    let cursor = std::io::Cursor::new(schema_ipc);
    let df = IpcReader::new(cursor)
        .finish()
        .map_err(|e| YxdbError::ConversionError(format!("failed to read IPC schema: {e}")))?;
    YxdbWriter::new(path, &df)
}

/// Write a batch of Arrow IPC bytes to an existing streaming writer.
pub fn writer_write_batch_from_ipc(
    writer: &mut YxdbWriter<BufWriter<File>>,
    ipc_bytes: &[u8],
) -> Result<()> {
    let cursor = std::io::Cursor::new(ipc_bytes);
    let df = IpcReader::new(cursor)
        .finish()
        .map_err(|e| YxdbError::ConversionError(format!("failed to read IPC batch: {e}")))?;
    writer.write_batch(&df)
}
