use crate::error::{Result, YxdbError};
use crate::field::{FieldMeta, FieldType};

/// YXDB file header - always 512 bytes.
///
/// ## Layout
/// - Bytes 0..21: Magic string `"Alteryx Database File"` (ASCII, null-padded)
/// - Bytes 64..68: `file_id` (i32 LE) - `0x00440204` (WrigleyDB, no spatial index)
/// - Bytes 80..84: `meta_info_size` (u32 LE) - number of **UTF-16 code units**
///   in the XML metadata that follows the header (including a null terminator)
/// - Bytes 96..104: `record_block_index_pos` (i64 LE) - file offset of the
///   RecordBlockIndex, marking the end of compressed block data
/// - Bytes 104..112: `num_records` (u64 LE)
/// - Bytes 112..116: `compression_version` (i32 LE) - `0` for uncompressed (no block framing),
///   `1` for LZF compression with block framing
///
/// The XML metadata immediately follows the 512-byte header and is encoded as
/// little-endian UTF-16. Its byte length is `meta_info_size * 2`.
#[derive(Debug)]
pub struct YxdbHeader {
    pub num_records: u64,
    pub meta_info_size: u32,
    pub compression_version: i32,
    /// File offset where the RecordBlockIndex starts (end of block data).
    pub record_block_index_pos: i64,
    /// File ID / version identifier.
    ///
    /// - `0x00440204` - WrigleyDB without spatial index
    /// - `0x00440205` - WrigleyDB with spatial index
    pub file_id: u32,
    /// File offset of the spatial index section (0 if none).
    ///
    /// Only meaningful when `file_id == 0x00440205`.
    pub spatial_index_pos: i64,
}

pub const HEADER_SIZE: usize = 512;
pub const MAGIC: &[u8] = b"Alteryx Database File";

/// Maximum number of records we accept from the file header.
///
/// 10 billion records is far beyond any realistic YXDB file.
/// This guards against corrupt headers that claim huge record counts and
/// cause the reader to attempt multi-gigabyte allocations.
const MAX_RECORDS: u64 = 10_000_000_000;

/// Maximum number of fields (columns) we accept from the XML metadata.
///
/// 100,000 columns is generous - real-world files rarely exceed a few hundred.
/// This prevents pathological XML metadata from consuming excessive memory.
pub const MAX_FIELDS: usize = 100_000;

/// Maximum byte length we accept for the UTF-16 XML metadata block.
///
/// 64 MiB is far beyond any real YXDB schema (observed files carry metadata
/// in the KB range). `meta_info_size` comes straight from the file header as
/// an untrusted u32, so without this cap a corrupt/malicious file can force
/// a multi-gigabyte allocation before a single byte of metadata is read
/// (fuzzing found this via `oom-...` crashes allocating 2-7 GiB).
pub const MAX_META_BYTES: u64 = 64 * 1024 * 1024;

/// YXDB file ID for files **with** a spatial index.
pub const ID_WRIGLEYDB: u32 = 0x00440205;
/// YXDB file ID for files **without** a spatial index.
pub const ID_WRIGLEYDB_NO_SPATIAL_INDEX: u32 = 0x00440204;

impl YxdbHeader {
    /// Returns `true` if this file contains a spatial index.
    pub fn has_spatial_index(&self) -> bool {
        self.file_id == ID_WRIGLEYDB && self.spatial_index_pos > 0
    }

    /// Parse a 512-byte header buffer.
    pub fn parse(buf: &[u8; HEADER_SIZE]) -> Result<Self> {
        if &buf[0..MAGIC.len()] != MAGIC {
            return Err(YxdbError::InvalidFile(
                "file does not start with 'Alteryx Database File'".into(),
            ));
        }

        let meta_info_size = u32::from_le_bytes(buf[80..84].try_into().unwrap());
        let record_block_index_pos = i64::from_le_bytes(buf[96..104].try_into().unwrap());
        let num_records = u64::from_le_bytes(buf[104..112].try_into().unwrap());
        let compression_version = i32::from_le_bytes(buf[112..116].try_into().unwrap());

        // Guard against corrupt headers claiming an unreasonable metadata size.
        let meta_byte_len = meta_info_size as u64 * 2;
        if meta_byte_len > MAX_META_BYTES {
            return Err(YxdbError::InvalidFile(format!(
                "header metadata size {meta_byte_len} bytes exceeds limit of {MAX_META_BYTES} (corrupt file?)",
            )));
        }

        // Guard against corrupt headers with unreasonable record counts.
        if num_records > MAX_RECORDS {
            return Err(YxdbError::InvalidFile(format!(
                "header record count {num_records} exceeds limit of {MAX_RECORDS} (corrupt file?)",
            )));
        }

        let file_id = u32::from_le_bytes(buf[64..68].try_into().unwrap());
        let spatial_index_pos = i64::from_le_bytes(buf[88..96].try_into().unwrap());

        Ok(YxdbHeader {
            num_records,
            meta_info_size,
            compression_version,
            record_block_index_pos,
            file_id,
            spatial_index_pos,
        })
    }
}

/// Parse the UTF-16-encoded XML metadata into a list of [`FieldMeta`].
pub fn parse_meta_xml(xml: &str) -> Result<Vec<FieldMeta>> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    let mut fields = Vec::new();
    let mut offset: usize = 0;
    let mut in_record_info = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"RecordInfo" => {
                in_record_info = true;
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == b"RecordInfo" => {
                in_record_info = false;
            }
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e))
                if e.name().as_ref() == b"Field" && in_record_info =>
            {
                let mut name = String::new();
                let mut type_str = String::new();
                let mut size: usize = 0;
                let mut scale: usize = 0;

                for attr in e.attributes().flatten() {
                    match attr.key.as_ref() {
                        b"name" => {
                            name = attr
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .map(|v| v.into_owned())
                                .unwrap_or_else(|_| {
                                    String::from_utf8_lossy(&attr.value).to_string()
                                });
                        }
                        b"type" => {
                            type_str = String::from_utf8_lossy(&attr.value).to_string();
                        }
                        b"size" => {
                            size = String::from_utf8_lossy(&attr.value).parse().unwrap_or(0);
                        }
                        b"scale" => {
                            scale = String::from_utf8_lossy(&attr.value).parse().unwrap_or(0);
                        }
                        _ => {}
                    }
                }

                let field_type = FieldType::from_xml_str(&type_str)
                    .ok_or_else(|| YxdbError::UnsupportedFieldType(type_str.clone()))?;

                let current_offset = offset;
                offset += field_type.fixed_bytes(size);

                fields.push(FieldMeta {
                    name,
                    field_type,
                    size,
                    scale,
                    offset: current_offset,
                });

                if fields.len() > MAX_FIELDS {
                    return Err(YxdbError::InvalidFile(format!(
                        "field count exceeds limit of {MAX_FIELDS}"
                    )));
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(YxdbError::XmlError(format!(
                    "error parsing XML metadata: {e}"
                )));
            }
            _ => {}
        }
    }

    if fields.is_empty() {
        return Err(YxdbError::InvalidFile(
            "no fields found in XML metadata".into(),
        ));
    }

    Ok(fields)
}

/// Decode a byte slice of little-endian UTF-16 into a Rust String.
///
/// The input should NOT include a trailing null terminator pair.
pub fn decode_utf16_le(bytes: &[u8]) -> String {
    let code_units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&code_units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_magic_check() {
        let mut buf = [0u8; HEADER_SIZE];
        let res = YxdbHeader::parse(&buf);
        assert!(res.is_err());

        buf[..MAGIC.len()].copy_from_slice(MAGIC);
        let res = YxdbHeader::parse(&buf);
        assert!(res.is_ok());
    }

    #[test]
    fn header_round_trip_fields() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[..MAGIC.len()].copy_from_slice(MAGIC);
        // meta_info_size = 1000 at offset 80
        buf[80..84].copy_from_slice(&1000u32.to_le_bytes());
        // record_block_index_pos = 999999 at offset 96
        buf[96..104].copy_from_slice(&999999i64.to_le_bytes());
        // num_records = 50000 at offset 104
        buf[104..112].copy_from_slice(&50000u64.to_le_bytes());
        // compression_version = 1 at offset 112
        buf[112..116].copy_from_slice(&1i32.to_le_bytes());

        let hdr = YxdbHeader::parse(&buf).unwrap();
        assert_eq!(hdr.meta_info_size, 1000);
        assert_eq!(hdr.record_block_index_pos, 999999);
        assert_eq!(hdr.num_records, 50000);
        assert_eq!(hdr.compression_version, 1);
    }

    #[test]
    fn header_zero_records() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[..MAGIC.len()].copy_from_slice(MAGIC);
        let hdr = YxdbHeader::parse(&buf).unwrap();
        assert_eq!(hdr.num_records, 0);
        assert_eq!(hdr.compression_version, 0);
    }

    #[test]
    fn header_oversized_meta_info_size_rejected() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[..MAGIC.len()].copy_from_slice(MAGIC);
        // meta_info_size = u32::MAX code units → ~8 GiB of metadata bytes.
        // This is the exact class of value libFuzzer's OOM detector found:
        // a corrupt header claiming an unreasonable metadata size caused a
        // multi-gigabyte allocation before a single byte was read.
        buf[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = YxdbHeader::parse(&buf).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("exceeds limit"), "unexpected error: {msg}");
    }

    #[test]
    fn header_truncated_magic() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[..10].copy_from_slice(&MAGIC[..10]); // only partial magic
        assert!(YxdbHeader::parse(&buf).is_err());
    }

    #[test]
    fn header_negative_record_count_rejected() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[..MAGIC.len()].copy_from_slice(MAGIC);
        // Write -1i64 at offset 104 → interpreted as u64::MAX
        buf[104..112].copy_from_slice(&(-1i64).to_le_bytes());
        let err = YxdbHeader::parse(&buf).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("exceeds limit"), "unexpected error: {msg}");
    }

    #[test]
    fn parse_simple_xml() {
        let xml = r#"<RecordInfo>
            <Field name="ID" type="Int32" />
            <Field name="Name" type="V_WString" size="256" />
            <Field name="Value" type="Double" />
        </RecordInfo>"#;
        let fields = parse_meta_xml(xml).unwrap();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "ID");
        assert_eq!(fields[0].field_type, FieldType::Int32);
        assert_eq!(fields[0].offset, 0);
        // Int32 = 4+1 = 5 bytes
        assert_eq!(fields[1].offset, 5);
        assert_eq!(fields[1].field_type, FieldType::VWString);
        // V_WString = 4 bytes (pointer)
        assert_eq!(fields[2].offset, 9);
        assert_eq!(fields[2].field_type, FieldType::Double);
    }

    #[test]
    fn parse_xml_all_types() {
        let xml = r#"<RecordInfo>
            <Field name="a" type="Bool" />
            <Field name="b" type="Byte" />
            <Field name="c" type="Int16" />
            <Field name="d" type="Int32" />
            <Field name="e" type="Int64" />
            <Field name="f" type="Float" />
            <Field name="g" type="Double" />
            <Field name="h" type="FixedDecimal" size="19" scale="4" />
            <Field name="i" type="String" size="10" />
            <Field name="j" type="WString" size="10" />
            <Field name="k" type="V_String" size="256" />
            <Field name="l" type="V_WString" size="256" />
            <Field name="m" type="Date" />
            <Field name="n" type="Time" />
            <Field name="o" type="DateTime" />
            <Field name="p" type="Blob" size="1000" />
            <Field name="q" type="SpatialObj" size="1000" />
        </RecordInfo>"#;
        let fields = parse_meta_xml(xml).unwrap();
        assert_eq!(fields.len(), 17);
        assert_eq!(fields[0].field_type, FieldType::Bool);
        assert_eq!(fields[7].field_type, FieldType::FixedDecimal);
        assert_eq!(fields[7].scale, 4);
        assert_eq!(fields[16].field_type, FieldType::SpatialObj);
    }

    #[test]
    fn parse_xml_empty_record_info() {
        let xml = r#"<RecordInfo></RecordInfo>"#;
        let result = parse_meta_xml(xml);
        assert!(result.is_err()); // no fields => error
    }

    #[test]
    fn parse_xml_unsupported_type() {
        let xml = r#"<RecordInfo>
            <Field name="x" type="UnknownType" />
        </RecordInfo>"#;
        let result = parse_meta_xml(xml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_xml_nested_in_outer_element() {
        // RecordInfo inside a wrapper - should still parse
        let xml = r#"<MetaInfo>
            <RecordInfo>
                <Field name="x" type="Int32" />
            </RecordInfo>
        </MetaInfo>"#;
        let fields = parse_meta_xml(xml).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "x");
    }

    #[test]
    fn parse_xml_field_outside_record_info_ignored() {
        // Field tags outside RecordInfo should be ignored
        let xml = r#"<Root>
            <Field name="ignored" type="Int32" />
            <RecordInfo>
                <Field name="real" type="Int32" />
            </RecordInfo>
            <Field name="also_ignored" type="Int32" />
        </Root>"#;
        let fields = parse_meta_xml(xml).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "real");
    }

    #[test]
    fn decode_utf16le_ascii() {
        let bytes = [0x48, 0x00, 0x69, 0x00]; // "Hi"
        assert_eq!(decode_utf16_le(&bytes), "Hi");
    }

    #[test]
    fn decode_utf16le_non_ascii() {
        // ü = U+00FC = [0xFC, 0x00]
        let bytes = [0xFC, 0x00];
        assert_eq!(decode_utf16_le(&bytes), "ü");
    }

    #[test]
    fn decode_utf16le_empty() {
        assert_eq!(decode_utf16_le(&[]), "");
    }

    #[test]
    fn decode_utf16le_cjk() {
        // U+65E5 (日) = [0xE5, 0x65]
        let bytes = [0xE5, 0x65];
        assert_eq!(decode_utf16_le(&bytes), "日");
    }

    #[test]
    fn header_file_id_and_spatial_index() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[..MAGIC.len()].copy_from_slice(MAGIC);

        // Default: no spatial index
        buf[64..68].copy_from_slice(&ID_WRIGLEYDB_NO_SPATIAL_INDEX.to_le_bytes());
        let hdr = YxdbHeader::parse(&buf).unwrap();
        assert_eq!(hdr.file_id, ID_WRIGLEYDB_NO_SPATIAL_INDEX);
        assert!(!hdr.has_spatial_index());
        assert_eq!(hdr.spatial_index_pos, 0);
    }

    #[test]
    fn header_with_spatial_index() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[..MAGIC.len()].copy_from_slice(MAGIC);

        // With spatial index
        buf[64..68].copy_from_slice(&ID_WRIGLEYDB.to_le_bytes());
        // nSpatialIndexPos at offset 88..96 (aligned)
        buf[88..96].copy_from_slice(&12345i64.to_le_bytes());

        let hdr = YxdbHeader::parse(&buf).unwrap();
        assert_eq!(hdr.file_id, ID_WRIGLEYDB);
        assert!(hdr.has_spatial_index());
        assert_eq!(hdr.spatial_index_pos, 12345);
    }

    #[test]
    fn header_spatial_index_zero_pos_means_no_index() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[..MAGIC.len()].copy_from_slice(MAGIC);

        // file_id says spatial index but pos is 0 - treat as no index
        buf[64..68].copy_from_slice(&ID_WRIGLEYDB.to_le_bytes());
        buf[88..96].copy_from_slice(&0i64.to_le_bytes());

        let hdr = YxdbHeader::parse(&buf).unwrap();
        assert!(!hdr.has_spatial_index());
    }
}
