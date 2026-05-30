use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyTuple};
use pyo3_polars::PyDataFrame;
use sigilyx_core::{
    read_yxdb_to_ipc, read_yxdb_to_ipc_batches, FieldValue, SpatialMode, YxdbReader, YxdbRowReader,
};

use sigilyx_core::YxdbWriter;
use std::fs::File;
use std::io::BufWriter;

/// Convert a sigilyx_core error to the most appropriate Python exception.
fn to_py_err(e: sigilyx_core::YxdbError) -> PyErr {
    use sigilyx_core::YxdbError;
    match &e {
        YxdbError::Io(io_err) => match io_err.kind() {
            std::io::ErrorKind::NotFound => {
                pyo3::exceptions::PyFileNotFoundError::new_err(e.to_string())
            }
            std::io::ErrorKind::PermissionDenied => {
                pyo3::exceptions::PyPermissionError::new_err(e.to_string())
            }
            _ => pyo3::exceptions::PyOSError::new_err(e.to_string()),
        },
        YxdbError::InvalidFile(_) | YxdbError::XmlError(_) | YxdbError::LzfError(_) => {
            pyo3::exceptions::PyValueError::new_err(e.to_string())
        }
        YxdbError::UnsupportedFieldType(_) | YxdbError::ConversionError(_) => {
            pyo3::exceptions::PyTypeError::new_err(e.to_string())
        }
    }
}

/// Parse a Python spatial mode string into a SpatialMode enum.
/// Accepts any casing (e.g. "WKB", "wkb", "Wkb" all work).
fn parse_spatial_mode(mode: &str) -> PyResult<SpatialMode> {
    match mode.to_ascii_lowercase().as_str() {
        "wkb" => Ok(SpatialMode::Wkb),
        "raw" => Ok(SpatialMode::Raw),
        "geoarrow" => Ok(SpatialMode::GeoArrow),
        _ => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "invalid spatial mode {mode:?}, expected \"wkb\", \"raw\", or \"geoarrow\""
        ))),
    }
}

/// Read a YXDB file and return a Polars DataFrame directly (zero-copy via Arrow C Data Interface).
///
/// ``spatial`` controls how SpatialObj columns are returned:
///   - ``"wkb"`` (default) — decode SHP → ISO WKB
///   - ``"raw"`` — keep the internal SHP bytes
///
/// ```python
/// from sigilyx import read_yxdb_df
/// df = read_yxdb_df("path/to/file.yxdb")
/// df = read_yxdb_df("path/to/file.yxdb", spatial="raw")
/// ```
#[pyfunction]
#[pyo3(signature = (path, spatial = "wkb", allow_unverified_e2_types = false))]
fn read_yxdb_df(
    path: &str,
    spatial: &str,
    allow_unverified_e2_types: bool,
) -> PyResult<PyDataFrame> {
    let mode = parse_spatial_mode(spatial)?;
    let df = sigilyx_core::read_yxdb(path, mode, allow_unverified_e2_types).map_err(to_py_err)?;
    Ok(PyDataFrame(df))
}

/// Read a YXDB file, returning only the specified columns as a Polars DataFrame.
///
/// ``spatial`` controls how SpatialObj columns are returned (see ``read_yxdb_df``).
///
/// ```python
/// from sigilyx import read_yxdb_df_columns
/// df = read_yxdb_df_columns("file.yxdb", ["col_a", "col_b"])
/// ```
#[pyfunction]
#[pyo3(signature = (path, columns, spatial = "wkb", allow_unverified_e2_types = false))]
fn read_yxdb_df_columns(
    path: &str,
    columns: Vec<String>,
    spatial: &str,
    allow_unverified_e2_types: bool,
) -> PyResult<PyDataFrame> {
    let mode = parse_spatial_mode(spatial)?;
    let col_refs: Vec<&str> = columns.iter().map(|s| s.as_str()).collect();
    let df = sigilyx_core::read_yxdb_columns(path, &col_refs, mode, allow_unverified_e2_types)
        .map_err(to_py_err)?;
    Ok(PyDataFrame(df))
}

/// Write a Polars DataFrame to a YXDB file directly (zero-copy via Arrow C Data Interface).
///
/// ``spatial_columns`` names Binary columns that contain WKB geometry data.
/// These will be written as ``SpatialObj`` fields (WKB → SHP conversion).
/// Omit or pass an empty list for non-spatial data.
///
/// ```python
/// from sigilyx import write_yxdb_df
/// write_yxdb_df("output.yxdb", df)
/// write_yxdb_df("output.yxdb", df, spatial_columns=["geometry"])
/// ```
#[pyfunction]
#[pyo3(signature = (path, pydf, spatial_columns = None))]
fn write_yxdb_df(
    path: &str,
    pydf: PyDataFrame,
    spatial_columns: Option<Vec<String>>,
) -> PyResult<()> {
    let cols = spatial_columns.unwrap_or_default();
    let col_refs: Vec<&str> = cols.iter().map(|s| s.as_str()).collect();
    sigilyx_core::write_yxdb(path, &pydf.0, &col_refs).map_err(to_py_err)?;
    Ok(())
}

/// Write a Polars DataFrame to YXDB with an explicit field schema.
///
/// ``type_overrides`` is a dict mapping column name to a dict with keys:
///   - ``type``: YXDB type name (e.g. ``"String"``, ``"WString"``, ``"V_String"``, ``"V_WString"``)
///   - ``size`` (optional): field size (max chars for strings, precision for FixedDecimal)
///   - ``scale`` (optional): scale (only for FixedDecimal)
///
/// Columns not in ``type_overrides`` use the default inferred type.
///
/// ```python
/// from sigilyx import write_yxdb_df_with_overrides
/// write_yxdb_df_with_overrides("out.yxdb", df, {"name": {"type": "String", "size": 64}})
/// ```
#[pyfunction]
#[pyo3(signature = (path, pydf, type_overrides, spatial_columns = None))]
fn write_yxdb_df_with_overrides(
    path: &str,
    pydf: PyDataFrame,
    type_overrides: &Bound<'_, PyDict>,
    spatial_columns: Option<Vec<String>>,
) -> PyResult<()> {
    let spatial_cols = spatial_columns.unwrap_or_default();
    let spatial_refs: Vec<&str> = spatial_cols.iter().map(|s| s.as_str()).collect();
    let mut fields =
        sigilyx_core::infer_schema_public(&pydf.0, &spatial_refs).map_err(to_py_err)?;

    apply_type_overrides(&mut fields, type_overrides)?;

    sigilyx_core::write_yxdb_with_schema(path, &pydf.0, &fields).map_err(to_py_err)?;
    Ok(())
}

/// Write YXDB from IPC bytes with explicit field type overrides.
///
/// This is the IPC fallback for ``write_yxdb_df_with_overrides`` when
/// the direct DataFrame path fails due to pyo3-polars compat_level mismatch.
#[pyfunction]
#[pyo3(signature = (path, ipc_bytes, type_overrides, spatial_columns = None))]
fn write_yxdb_ipc_with_overrides(
    _py: Python<'_>,
    path: &str,
    ipc_bytes: &[u8],
    type_overrides: &Bound<'_, PyDict>,
    spatial_columns: Option<Vec<String>>,
) -> PyResult<()> {
    let df = sigilyx_core::ipc_to_dataframe(ipc_bytes).map_err(to_py_err)?;

    let spatial_cols = spatial_columns.unwrap_or_default();
    let spatial_refs: Vec<&str> = spatial_cols.iter().map(|s| s.as_str()).collect();
    let mut fields = sigilyx_core::infer_schema_public(&df, &spatial_refs).map_err(to_py_err)?;

    apply_type_overrides(&mut fields, type_overrides)?;

    sigilyx_core::write_yxdb_with_schema(path, &df, &fields).map_err(to_py_err)?;
    Ok(())
}

/// Apply type overrides from a Python dict to a mutable field schema.
///
/// Shared logic used by both the direct DataFrame and IPC write paths.
fn apply_type_overrides(
    fields: &mut [sigilyx_core::FieldMeta],
    type_overrides: &Bound<'_, PyDict>,
) -> PyResult<()> {
    use sigilyx_core::field::FieldType;

    for (key, value) in type_overrides.iter() {
        let col_name: String = key.extract()?;
        let override_dict: &Bound<'_, PyDict> = value.cast()?;

        let type_str: String = override_dict
            .get_item("type")?
            .ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "type_overrides[{col_name:?}] must have a 'type' key"
                ))
            })?
            .extract()?;

        let field_type: FieldType = type_str.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "unknown YXDB type {type_str:?} for column {col_name:?}"
            ))
        })?;

        let size: Option<usize> = override_dict
            .get_item("size")?
            .map(|v| v.extract())
            .transpose()?;
        let scale: Option<usize> = override_dict
            .get_item("scale")?
            .map(|v| v.extract())
            .transpose()?;

        let field = fields
            .iter_mut()
            .find(|f| f.name == col_name)
            .ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "column {col_name:?} not found in DataFrame"
                ))
            })?;
        field.field_type = field_type;
        if let Some(s) = size {
            field.size = s;
        }
        if let Some(s) = scale {
            field.scale = s;
        }
    }

    // Recompute offsets after overrides
    let mut offset = 0;
    for field in fields.iter_mut() {
        field.offset = offset;
        offset += field.field_type.fixed_bytes(field.size);
    }

    Ok(())
}

/// Convert SHP geometry bytes to WKB. Returns None for null shapes.
///
/// ```python
/// wkb = sigilyx.shp_to_wkb(shp_bytes)
/// ```
#[pyfunction]
fn shp_to_wkb_py(py: Python<'_>, shp: &[u8]) -> PyResult<Option<Py<PyBytes>>> {
    match sigilyx_core::shp_to_wkb(shp).map_err(to_py_err)? {
        None => Ok(None),
        Some(wkb) => Ok(Some(PyBytes::new(py, &wkb).into())),
    }
}

/// Convert WKB geometry bytes to SHP format.
///
/// ```python
/// shp = sigilyx.wkb_to_shp(wkb_bytes)
/// ```
#[pyfunction]
fn wkb_to_shp_py(py: Python<'_>, wkb: &[u8]) -> PyResult<Py<PyBytes>> {
    let shp = sigilyx_core::wkb_to_shp(wkb).map_err(to_py_err)?;
    Ok(PyBytes::new(py, &shp).into())
}

/// Read a YXDB file and return Arrow IPC bytes (legacy API).
#[pyfunction]
#[pyo3(signature = (path, spatial = "wkb", allow_unverified_e2_types = false))]
fn read_yxdb(
    py: Python<'_>,
    path: &str,
    spatial: &str,
    allow_unverified_e2_types: bool,
) -> PyResult<Py<PyBytes>> {
    let mode = parse_spatial_mode(spatial)?;
    let ipc_bytes = read_yxdb_to_ipc(path, mode, allow_unverified_e2_types).map_err(to_py_err)?;
    Ok(PyBytes::new(py, &ipc_bytes).into())
}

/// Read a YXDB file and return field metadata as a list of dicts.
///
/// Each dict contains: name, type, size, scale.
#[pyfunction]
fn read_yxdb_schema(py: Python<'_>, path: &str) -> PyResult<Py<PyAny>> {
    let reader = YxdbReader::open(path).map_err(to_py_err)?;

    let list = pyo3::types::PyList::empty(py);
    for field in &reader.fields {
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("name", &field.name)?;
        dict.set_item("type", field.field_type.to_string())?;
        dict.set_item("size", field.size)?;
        dict.set_item("scale", field.scale)?;
        list.append(dict)?;
    }
    Ok(list.into())
}

/// Return the number of records in a YXDB file without reading all data.
#[pyfunction]
fn read_yxdb_record_count(path: &str) -> PyResult<u64> {
    let reader = YxdbReader::open(path).map_err(to_py_err)?;
    Ok(reader.header.num_records)
}

/// Return spatial index metadata from a YXDB file header.
///
/// Returns a dict with keys:
///   - ``has_spatial_index`` (bool): whether the file contains a spatial index
///   - ``spatial_index_pos`` (int): file offset of the spatial index (0 if none)
///   - ``file_id`` (int): the file ID/version from the header
///   - ``spatial_columns`` (list[str]): names of SpatialObj columns
#[pyfunction]
fn read_yxdb_spatial_info(py: Python<'_>, path: &str) -> PyResult<Py<PyAny>> {
    let reader = YxdbReader::open(path).map_err(to_py_err)?;
    let dict = PyDict::new(py);
    dict.set_item("has_spatial_index", reader.header.has_spatial_index())?;
    dict.set_item("spatial_index_pos", reader.header.spatial_index_pos)?;
    dict.set_item("file_id", reader.header.file_id)?;
    let spatial_cols = sigilyx_core::spatial_column_names(&reader.fields);
    let py_list = PyList::empty(py);
    for col in &spatial_cols {
        py_list.append(col)?;
    }
    dict.set_item("spatial_columns", py_list)?;
    Ok(dict.into())
}

/// Read a YXDB file in batches, returning a list of Arrow IPC byte chunks.
///
/// Each chunk is an independent IPC buffer containing up to `batch_size` rows.
/// This enables streaming / memory-efficient processing of large files.
#[pyfunction]
#[pyo3(signature = (path, batch_size = 65536, spatial = "wkb"))]
fn read_yxdb_batches(
    py: Python<'_>,
    path: &str,
    batch_size: usize,
    spatial: &str,
) -> PyResult<Vec<Py<PyBytes>>> {
    if batch_size == 0 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "batch_size must be > 0",
        ));
    }
    let mode = parse_spatial_mode(spatial)?;
    let ipc_chunks = read_yxdb_to_ipc_batches(path, batch_size, mode).map_err(to_py_err)?;
    let py_chunks: Vec<Py<PyBytes>> = ipc_chunks
        .iter()
        .map(|chunk| PyBytes::new(py, chunk).into())
        .collect();
    Ok(py_chunks)
}

/// Write a Polars DataFrame (as Arrow IPC bytes) to a YXDB file.
///
/// In Python:
///
/// ```python
/// import polars as pl
/// from sigilyx import write_yxdb
///
/// df = pl.DataFrame({"id": [1, 2, 3], "name": ["Alice", "Bob", "Charlie"]})
/// write_yxdb("output.yxdb", df.to_ipc())
/// ```
#[pyfunction]
#[pyo3(signature = (path, ipc_bytes, spatial_columns = None))]
fn write_yxdb(
    _py: Python<'_>,
    path: &str,
    ipc_bytes: &[u8],
    spatial_columns: Option<Vec<String>>,
) -> PyResult<()> {
    let cols = spatial_columns.unwrap_or_default();
    let col_refs: Vec<&str> = cols.iter().map(|s| s.as_str()).collect();
    sigilyx_core::write_yxdb_from_ipc_spatial(path, ipc_bytes, &col_refs).map_err(to_py_err)?;
    Ok(())
}

// ── Streaming Writer ───────────────────────────────────────────────────

/// A streaming YXDB writer that accepts data in batches.
///
/// This enables memory-efficient writing of large datasets that are
/// processed in chunks. Each batch is written incrementally without
/// holding the entire dataset in memory.
///
/// Use `YxdbStreamWriter(path, schema_ipc_bytes)` to create a writer,
/// then call `write_batch(ipc_bytes)` for each batch, and finally
/// `finish()` to finalize the file.
#[pyclass]
struct YxdbStreamWriter {
    writer: Option<YxdbWriter<BufWriter<File>>>,
}

#[pymethods]
impl YxdbStreamWriter {
    /// Create a new streaming YXDB writer.
    ///
    /// `path`: Output file path.
    /// `schema_ipc_bytes`: Arrow IPC bytes from a template DataFrame
    ///   (used to infer the YXDB schema). Can be an empty DataFrame
    ///   with the correct column types.
    #[new]
    fn new(path: &str, schema_ipc_bytes: &[u8]) -> PyResult<Self> {
        let writer =
            sigilyx_core::create_writer_from_ipc(path, schema_ipc_bytes).map_err(to_py_err)?;
        Ok(Self {
            writer: Some(writer),
        })
    }

    /// Write a batch of records (as Arrow IPC bytes) to the YXDB file.
    fn write_batch(&mut self, ipc_bytes: &[u8]) -> PyResult<()> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("writer already finished"))?;
        sigilyx_core::writer_write_batch_from_ipc(writer, ipc_bytes).map_err(to_py_err)
    }

    /// Finalize the YXDB file and return the total number of records written.
    ///
    /// This updates the header with the final record count. Must be called
    /// to produce a valid YXDB file.
    fn finish(&mut self) -> PyResult<u64> {
        let writer = self
            .writer
            .take()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("writer already finished"))?;
        let count = writer.record_count();
        writer.finish().map_err(to_py_err)?;
        Ok(count)
    }

    /// Context manager entry — returns self.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context manager exit — automatically calls finish() if not already done.
    #[pyo3(signature = (_exc_type=None, _exc_val=None, _exc_tb=None))]
    fn __exit__(
        &mut self,
        _exc_type: Option<&Bound<'_, pyo3::types::PyType>>,
        _exc_val: Option<&Bound<'_, pyo3::PyAny>>,
        _exc_tb: Option<&Bound<'_, pyo3::PyAny>>,
    ) -> PyResult<bool> {
        if self.writer.is_some() {
            self.finish()?;
        }
        Ok(false) // don't suppress exceptions
    }

    /// Get the current record count.
    fn record_count(&self) -> PyResult<u64> {
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("writer already finished"))?;
        Ok(writer.record_count())
    }
}

// ── Streaming Batch Reader ─────────────────────────────────────────────

/// A streaming, columnar YXDB batch reader exposed to Python.
///
/// Implements the iterator protocol (`__iter__` / `__next__`) so that
/// each call to `next()` reads one batch of up to `batch_size` rows from
/// the underlying Rust `YxdbReader`, returning a `PyDataFrame`.
///
/// Supports optional **column projection** (only materialise requested
/// columns) and **n_rows limit** (stop after reading at most N rows).
///
/// This is the building block for `scan_yxdb()` and `read_yxdb_batches()`.
#[pyclass(name = "_YxdbBatchReader")]
struct PyYxdbBatchReader {
    reader: Option<YxdbReader>,
    batch_size: usize,
    columns: Option<Vec<String>>,
    n_rows_limit: Option<u64>,
    rows_read: u64,
}

#[pymethods]
impl PyYxdbBatchReader {
    /// Create a new streaming batch reader.
    ///
    /// * `path`       – YXDB file path.
    /// * `batch_size` – Maximum rows per yielded DataFrame.
    /// * `columns`    – Optional list of column names to project.
    /// * `n_rows`     – Optional total row limit (early termination).
    #[new]
    #[pyo3(signature = (path, batch_size = 65536, columns = None, n_rows = None))]
    fn new(
        path: &str,
        batch_size: usize,
        columns: Option<Vec<String>>,
        n_rows: Option<u64>,
    ) -> PyResult<Self> {
        if batch_size == 0 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "batch_size must be > 0",
            ));
        }
        let reader = YxdbReader::open(path).map_err(to_py_err)?;
        Ok(Self {
            reader: Some(reader),
            batch_size,
            columns,
            n_rows_limit: n_rows,
            rows_read: 0,
        })
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<PyDataFrame>> {
        let reader = match self.reader.as_mut() {
            Some(r) => r,
            None => return Ok(None),
        };

        // Respect n_rows limit
        let effective_batch = match self.n_rows_limit {
            Some(limit) => {
                let remaining = limit.saturating_sub(self.rows_read) as usize;
                if remaining == 0 {
                    return Ok(None);
                }
                self.batch_size.min(remaining)
            }
            None => self.batch_size,
        };

        // Build column refs for projection
        let col_strs: Option<Vec<&str>> = self
            .columns
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_str()).collect());
        let col_refs: Option<&[&str]> = col_strs.as_deref();

        // Release the GIL during the heavy IO/decompression/parsing work.
        let df = py.detach(|| {
            reader
                .next_batch(effective_batch, col_refs)
                .map_err(to_py_err)
        })?;

        match df {
            Some(d) => {
                self.rows_read += d.height() as u64;
                Ok(Some(PyDataFrame(d)))
            }
            None => Ok(None),
        }
    }

    /// Return the schema (list of field dicts) without consuming data.
    fn schema(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("reader is closed"))?;

        let list = PyList::empty(py);
        for field in &reader.fields {
            let dict = PyDict::new(py);
            dict.set_item("name", &field.name)?;
            dict.set_item("type", field.field_type.to_string())?;
            dict.set_item("size", field.size)?;
            dict.set_item("scale", field.scale)?;
            list.append(dict)?;
        }
        Ok(list.into())
    }
}

// ── Row-by-Row Reader ────────────────────────────────────────────────

/// Convert a Rust FieldValue to a Python object.
fn field_value_to_py(py: Python<'_>, val: FieldValue) -> Py<PyAny> {
    match val {
        FieldValue::Bool(None)
        | FieldValue::Byte(None)
        | FieldValue::Int16(None)
        | FieldValue::Int32(None)
        | FieldValue::Int64(None)
        | FieldValue::Float(None)
        | FieldValue::Double(None)
        | FieldValue::Decimal(None)
        | FieldValue::String(None)
        | FieldValue::Date(None)
        | FieldValue::Time(None)
        | FieldValue::DateTime(None)
        | FieldValue::Blob(None) => py.None(),

        FieldValue::Bool(Some(v)) => v.into_pyobject(py).unwrap().to_owned().into_any().unbind(),
        FieldValue::Byte(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::Int16(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::Int32(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::Int64(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::Float(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::Double(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::Decimal(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::String(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::Date(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::Time(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::DateTime(Some(v)) => v.into_pyobject(py).unwrap().into_any().unbind(),
        FieldValue::Blob(Some(v)) => PyBytes::new(py, &v).into_any().unbind(),
        // BlobRef is an internal sentinel resolved before reaching consumers.
        // If one leaks through, treat as None.
        FieldValue::BlobRef(_, _) => py.None(),
    }
}

/// A row-by-row YXDB file reader exposed to Python.
///
/// Provides a cursor-style API for iterating records one at a time
/// and extracting typed field values, without building columnar data.
#[pyclass(name = "_YxdbRowReader")]
struct PyYxdbRowReader {
    reader: Option<YxdbRowReader>,
}

#[pymethods]
impl PyYxdbRowReader {
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let reader = YxdbRowReader::open(path).map_err(to_py_err)?;
        Ok(Self {
            reader: Some(reader),
        })
    }

    /// Advance to the next record. Returns True if a record is available.
    fn next_record(&mut self) -> PyResult<bool> {
        let reader = self
            .reader
            .as_mut()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("reader is closed"))?;
        reader.next().map_err(to_py_err)
    }

    /// Read a field value by column index (0-based).
    fn read_index(&self, py: Python<'_>, index: usize) -> PyResult<Py<PyAny>> {
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("reader is closed"))?;
        let val = reader.read_index(index).map_err(to_py_err)?;
        Ok(field_value_to_py(py, val))
    }

    /// Read a field value by column name.
    fn read_name(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("reader is closed"))?;
        let val = reader.read_name(name).map_err(to_py_err)?;
        Ok(field_value_to_py(py, val))
    }

    /// Read all field values from the current record as a tuple.
    fn read_all(&self, py: Python<'_>) -> PyResult<Py<PyTuple>> {
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("reader is closed"))?;
        let values = reader.read_all().map_err(to_py_err)?;
        let py_vals: Vec<Py<PyAny>> = values
            .into_iter()
            .map(|v| field_value_to_py(py, v))
            .collect();
        Ok(PyTuple::new(py, &py_vals)?.into())
    }

    /// Read all field values as a dict {name: value}.
    fn read_dict(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("reader is closed"))?;
        let values = reader.read_all().map_err(to_py_err)?;
        let dict = PyDict::new(py);
        for (field, val) in reader.fields().iter().zip(values) {
            dict.set_item(&field.name, field_value_to_py(py, val))?;
        }
        Ok(dict.into())
    }

    /// Return the total number of records in the file (from header).
    fn num_records(&self) -> PyResult<u64> {
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("reader is closed"))?;
        Ok(reader.num_records())
    }

    /// Return field metadata as a list of dicts.
    fn fields(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("reader is closed"))?;
        let list = PyList::empty(py);
        for field in reader.fields() {
            let dict = PyDict::new(py);
            dict.set_item("name", &field.name)?;
            dict.set_item("type", field.field_type.to_string())?;
            dict.set_item("size", field.size)?;
            dict.set_item("scale", field.scale)?;
            list.append(dict)?;
        }
        Ok(list.into())
    }

    /// Close the reader and release resources.
    fn close(&mut self) {
        self.reader.take();
    }
}

/// SigilYX — High-performance YXDB file reader and writer.
#[pymodule]
fn sigilyx(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(read_yxdb, m)?)?;
    m.add_function(wrap_pyfunction!(read_yxdb_df, m)?)?;
    m.add_function(wrap_pyfunction!(read_yxdb_df_columns, m)?)?;
    m.add_function(wrap_pyfunction!(write_yxdb_df, m)?)?;
    m.add_function(wrap_pyfunction!(write_yxdb_df_with_overrides, m)?)?;
    m.add_function(wrap_pyfunction!(write_yxdb_ipc_with_overrides, m)?)?;
    m.add_function(wrap_pyfunction!(shp_to_wkb_py, m)?)?;
    m.add_function(wrap_pyfunction!(wkb_to_shp_py, m)?)?;
    m.add_function(wrap_pyfunction!(read_yxdb_schema, m)?)?;
    m.add_function(wrap_pyfunction!(read_yxdb_record_count, m)?)?;
    m.add_function(wrap_pyfunction!(read_yxdb_spatial_info, m)?)?;
    m.add_function(wrap_pyfunction!(read_yxdb_batches, m)?)?;
    m.add_function(wrap_pyfunction!(write_yxdb, m)?)?;
    m.add_class::<YxdbStreamWriter>()?;
    m.add_class::<PyYxdbBatchReader>()?;
    m.add_class::<PyYxdbRowReader>()?;
    Ok(())
}
