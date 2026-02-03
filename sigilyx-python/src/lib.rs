use pyo3::prelude::*;
use pyo3::types::PyBytes;
use sigilyx_core::{read_yxdb_to_ipc, read_yxdb_to_ipc_batches, write_yxdb_from_ipc, YxdbReader};

/// Read a YXDB file and return Arrow IPC bytes.
///
/// In Python, use `polars.read_ipc()` on the returned bytes:
///
/// ```python
/// import polars as pl
/// from sigilyx import read_yxdb
/// df = pl.read_ipc(read_yxdb("path/to/file.yxdb"))
/// ```
#[pyfunction]
fn read_yxdb(py: Python<'_>, path: &str) -> PyResult<Py<PyBytes>> {
    let ipc_bytes = read_yxdb_to_ipc(path)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    Ok(PyBytes::new(py, &ipc_bytes).into())
}

/// Read a YXDB file and return field metadata as a list of dicts.
///
/// Each dict contains: name, type, size, scale.
#[pyfunction]
fn read_yxdb_schema(py: Python<'_>, path: &str) -> PyResult<PyObject> {
    let reader = YxdbReader::open(path)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

    let list = pyo3::types::PyList::empty(py);
    for field in &reader.fields {
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("name", &field.name)?;
        dict.set_item("type", format!("{:?}", field.field_type))?;
        dict.set_item("size", field.size)?;
        dict.set_item("scale", field.scale)?;
        list.append(dict)?;
    }
    Ok(list.into())
}

/// Return the number of records in a YXDB file without reading all data.
#[pyfunction]
fn read_yxdb_record_count(path: &str) -> PyResult<u64> {
    let reader = YxdbReader::open(path)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    Ok(reader.header.num_records)
}

/// Read a YXDB file in batches, returning a list of Arrow IPC byte chunks.
///
/// Each chunk is an independent IPC buffer containing up to `batch_size` rows.
/// This enables streaming / memory-efficient processing of large files.
#[pyfunction]
#[pyo3(signature = (path, batch_size = 65536))]
fn read_yxdb_batches(py: Python<'_>, path: &str, batch_size: usize) -> PyResult<Vec<Py<PyBytes>>> {
    let ipc_chunks = read_yxdb_to_ipc_batches(path, batch_size)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
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
fn write_yxdb(_py: Python<'_>, path: &str, ipc_bytes: &[u8]) -> PyResult<()> {
    write_yxdb_from_ipc(path, ipc_bytes)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    Ok(())
}

/// SigilYX — High-performance YXDB file reader and writer.
/// Not affiliated with Alteryx, Inc.
#[pymodule]
fn sigilyx(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(read_yxdb, m)?)?;
    m.add_function(wrap_pyfunction!(read_yxdb_schema, m)?)?;
    m.add_function(wrap_pyfunction!(read_yxdb_record_count, m)?)?;
    m.add_function(wrap_pyfunction!(read_yxdb_batches, m)?)?;
    m.add_function(wrap_pyfunction!(write_yxdb, m)?)?;
    Ok(())
}
