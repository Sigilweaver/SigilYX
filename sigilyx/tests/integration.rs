//! Integration tests for the sigilyx crate.
//!
//! These tests exercise the public API with complete YXDB files,
//! verifying end-to-end read, write, and round-trip correctness.

use sigilyx::{
    read_yxdb, read_yxdb_columns, write_yxdb, write_yxdb_with_schema, SpatialMode, YxdbReader,
    YxdbWriter,
};
use std::path::{Path, PathBuf};

fn test_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test_files")
        .join(name)
}

// -- Read Tests --

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn read_all_types_file() {
    let df = read_yxdb(test_file("AllTypes.yxdb"), SpatialMode::Raw, false).unwrap();
    assert!(df.height() > 0);
    assert!(df.width() > 0);
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn read_people_file() {
    let df = read_yxdb(test_file("People.yxdb"), SpatialMode::Raw, false).unwrap();
    assert!(df.height() > 0);
    // People file should have typical columns
    assert!(df.width() >= 2);
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn read_null_values_file() {
    let df = read_yxdb(test_file("NullValues.yxdb"), SpatialMode::Raw, false).unwrap();
    assert!(df.height() > 0);
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn read_many_records_file() {
    let df = read_yxdb(test_file("ManyRecords.yxdb"), SpatialMode::Raw, false).unwrap();
    assert!(
        df.height() > 1000,
        "expected many records, got {}",
        df.height()
    );
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn read_strings_file() {
    let df = read_yxdb(test_file("Strings.yxdb"), SpatialMode::Raw, false).unwrap();
    assert!(df.height() > 0);
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn read_single_column_file() {
    let df = read_yxdb(test_file("SingleColumn.yxdb"), SpatialMode::Raw, false).unwrap();
    assert_eq!(df.width(), 1);
    assert!(df.height() > 0);
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn read_large_blob_file() {
    let df = read_yxdb(test_file("LargeBlob.yxdb"), SpatialMode::Raw, false).unwrap();
    assert!(df.height() > 0);
}

#[test]
fn read_invalid_file_returns_error() {
    let err = read_yxdb(test_file("not_a_yxdb.txt"), SpatialMode::Raw, false);
    assert!(err.is_err());
}

#[test]
fn read_too_small_file_returns_error() {
    let err = read_yxdb(test_file("too_small.bin"), SpatialMode::Raw, false);
    assert!(err.is_err());
}

#[test]
fn read_nonexistent_file_returns_error() {
    let err = read_yxdb(test_file("does_not_exist.yxdb"), SpatialMode::Raw, false);
    assert!(err.is_err());
}

// -- Column Projection Tests --

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn read_columns_subset() {
    let df_full = read_yxdb(test_file("AllTypes.yxdb"), SpatialMode::Raw, false).unwrap();
    let first_col = df_full.get_column_names()[0].to_string();

    let df_proj = read_yxdb_columns(
        test_file("AllTypes.yxdb"),
        &[first_col.as_str()],
        SpatialMode::Raw,
        false,
    )
    .unwrap();

    assert_eq!(df_proj.width(), 1);
    assert_eq!(df_proj.height(), df_full.height());
}

#[test]
fn read_columns_unknown_returns_error() {
    let err = read_yxdb_columns(
        test_file("People.yxdb"),
        &["nonexistent_column"],
        SpatialMode::Raw,
        false,
    );
    assert!(err.is_err());
}

// -- Write + Round-Trip Tests --

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn roundtrip_all_types() {
    let df = read_yxdb(test_file("AllTypes.yxdb"), SpatialMode::Raw, false).unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();

    write_yxdb(tmp.path(), &df, &[]).unwrap();

    let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
    assert_eq!(df.height(), df2.height());
    assert_eq!(df.width(), df2.width());

    // Verify column names match
    assert_eq!(df.get_column_names(), df2.get_column_names());
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn roundtrip_null_values() {
    let df = read_yxdb(test_file("NullValues.yxdb"), SpatialMode::Raw, false).unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();

    write_yxdb(tmp.path(), &df, &[]).unwrap();

    let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
    assert_eq!(df.height(), df2.height());
    assert_eq!(df.width(), df2.width());
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn roundtrip_many_records() {
    let df = read_yxdb(test_file("ManyRecords.yxdb"), SpatialMode::Raw, false).unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();

    write_yxdb(tmp.path(), &df, &[]).unwrap();

    let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
    assert_eq!(df.height(), df2.height());
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn roundtrip_strings() {
    let df = read_yxdb(test_file("Strings.yxdb"), SpatialMode::Raw, false).unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();

    write_yxdb(tmp.path(), &df, &[]).unwrap();

    let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
    assert_eq!(df.height(), df2.height());
    assert_eq!(df.width(), df2.width());
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn roundtrip_large_blob() {
    let df = read_yxdb(test_file("LargeBlob.yxdb"), SpatialMode::Raw, false).unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();

    write_yxdb(tmp.path(), &df, &[]).unwrap();

    let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
    assert_eq!(df.height(), df2.height());
}

// -- Streaming Writer Integration --

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn streaming_writer_roundtrip() {
    let df = read_yxdb(test_file("People.yxdb"), SpatialMode::Raw, false).unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();

    let mut writer = YxdbWriter::new(tmp.path(), &df).unwrap();
    writer.write_batch(&df).unwrap();
    writer.finish().unwrap();

    let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
    assert_eq!(df.height(), df2.height());
    assert_eq!(df.width(), df2.width());
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn streaming_writer_multiple_batches() {
    let df = read_yxdb(test_file("People.yxdb"), SpatialMode::Raw, false).unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();

    let mut writer = YxdbWriter::new(tmp.path(), &df).unwrap();
    // Write the same data twice
    writer.write_batch(&df).unwrap();
    writer.write_batch(&df).unwrap();
    writer.finish().unwrap();

    let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
    assert_eq!(df.height() * 2, df2.height());
}

// -- Batched Reader Integration --

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn batched_reader_reads_all_rows() {
    let df_full = read_yxdb(test_file("ManyRecords.yxdb"), SpatialMode::Raw, false).unwrap();
    let expected_rows = df_full.height();

    let mut reader = YxdbReader::open(test_file("ManyRecords.yxdb")).unwrap();
    let mut total_rows = 0;

    while let Some(batch) = reader.next_batch(100, None).unwrap() {
        assert!(batch.height() <= 100);
        total_rows += batch.height();
    }

    assert_eq!(total_rows, expected_rows);
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn batched_reader_with_projection() {
    let df_full = read_yxdb(test_file("AllTypes.yxdb"), SpatialMode::Raw, false).unwrap();
    let first_col = df_full.get_column_names()[0].to_string();

    let mut reader = YxdbReader::open(test_file("AllTypes.yxdb")).unwrap();
    let mut total_rows = 0;

    while let Some(batch) = reader.next_batch(50, Some(&[first_col.as_str()])).unwrap() {
        assert_eq!(batch.width(), 1);
        total_rows += batch.height();
    }

    assert_eq!(total_rows, df_full.height());
}

// -- Schema / Metadata Tests --

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn reader_exposes_field_metadata() {
    let reader = YxdbReader::open(test_file("AllTypes.yxdb")).unwrap();

    assert!(!reader.fields.is_empty());
    assert!(reader.header.num_records > 0);

    // Every field should have a non-empty name
    for field in &reader.fields {
        assert!(!field.name.is_empty());
    }
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn write_with_explicit_schema_roundtrip() {
    let df = read_yxdb(test_file("People.yxdb"), SpatialMode::Raw, false).unwrap();
    let reader = YxdbReader::open(test_file("People.yxdb")).unwrap();
    let fields = reader.fields.clone();
    let tmp = tempfile::NamedTempFile::new().unwrap();

    write_yxdb_with_schema(tmp.path(), &df, &fields).unwrap();

    let df2 = read_yxdb(tmp.path(), SpatialMode::Raw, false).unwrap();
    assert_eq!(df.height(), df2.height());
    assert_eq!(df.width(), df2.width());
}

// -- IPC Interop Tests --

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn ipc_roundtrip() {
    use sigilyx::{ipc_to_dataframe, read_yxdb_to_ipc};

    let ipc_bytes = read_yxdb_to_ipc(test_file("People.yxdb"), SpatialMode::Raw, false).unwrap();
    let df = ipc_to_dataframe(&ipc_bytes).unwrap();

    let df_direct = read_yxdb(test_file("People.yxdb"), SpatialMode::Raw, false).unwrap();
    assert_eq!(df.height(), df_direct.height());
    assert_eq!(df.width(), df_direct.width());
}

#[ignore = "requires test_files/*.yxdb fixtures (never committed) - see TODO"]
#[test]
fn ipc_batches_cover_all_rows() {
    use sigilyx::read_yxdb_to_ipc_batches;

    let df_full = read_yxdb(test_file("ManyRecords.yxdb"), SpatialMode::Raw, false).unwrap();
    let batches =
        read_yxdb_to_ipc_batches(test_file("ManyRecords.yxdb"), 500, SpatialMode::Raw).unwrap();

    let total: usize = batches
        .iter()
        .map(|b| sigilyx::ipc_to_dataframe(b).unwrap().height())
        .sum();

    assert_eq!(total, df_full.height());
}
