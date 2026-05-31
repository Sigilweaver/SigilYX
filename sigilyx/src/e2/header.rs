//! E2 YXDB file header parsing.
//!
//! The E2 header is 100 bytes:
//! - Bytes 0..64: Magic `"Alteryx e2 Database file"` space-padded to 64 bytes
//! - Bytes 64..68: File ID `0x00440208` (u32 LE)
//! - Bytes 68..72: Unknown constant `0x40000001` (u32 LE)
//! - Bytes 72..96: All zeros (reserved)
//! - Bytes 96..100: Metadata size in bytes (u32 LE) — byte length of UTF-8 XML

use crate::error::{Result, YxdbError};
use crate::field::{FieldMeta, FieldType};

pub const HEADER_SIZE: usize = 100;
pub const MAGIC: &[u8] = b"Alteryx e2 Database file";
pub const FILE_ID: u32 = 0x00440208;

/// Parsed E2 file header.
#[derive(Debug)]
pub struct E2Header {
    /// File ID (always `0x00440208` for E2).
    pub file_id: u32,
    /// Byte length of the UTF-8 XML metadata that follows the header.
    pub metadata_size: u32,
}

impl E2Header {
    /// Parse a 100-byte E2 header buffer.
    pub fn parse(buf: &[u8; HEADER_SIZE]) -> Result<Self> {
        // Validate magic string
        if &buf[0..MAGIC.len()] != MAGIC {
            return Err(YxdbError::InvalidFile(
                "file does not start with 'Alteryx e2 Database file'".into(),
            ));
        }

        let file_id = u32::from_le_bytes(buf[64..68].try_into().unwrap());
        if file_id != FILE_ID {
            return Err(YxdbError::InvalidFile(format!(
                "unexpected E2 file ID: 0x{file_id:08X} (expected 0x{FILE_ID:08X})"
            )));
        }

        let metadata_size = u32::from_le_bytes(buf[96..100].try_into().unwrap());

        Ok(E2Header {
            file_id,
            metadata_size,
        })
    }
}

/// Parse UTF-8 XML metadata into a list of [`FieldMeta`].
///
/// E2 uses the same `<RecordInfo>` XML schema as E1 but encoded in UTF-8.
/// Field offsets are NOT meaningful for E2 (compact encoding means fields
/// have variable sizes), but we compute them as sequential indices for
/// identification purposes.
pub fn parse_meta_xml(xml: &str) -> Result<Vec<FieldMeta>> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    let mut fields = Vec::new();
    let mut in_record_info = false;
    let mut index: usize = 0;

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

                fields.push(FieldMeta {
                    name,
                    field_type,
                    size,
                    scale,
                    // E2 uses compact encoding, so offset is just a field index
                    offset: index,
                });
                index += 1;

                if fields.len() > 100_000 {
                    return Err(YxdbError::InvalidFile(
                        "field count exceeds limit of 100,000".into(),
                    ));
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(YxdbError::XmlError(format!(
                    "error parsing E2 XML metadata: {e}"
                )));
            }
            _ => {}
        }
    }

    if fields.is_empty() {
        return Err(YxdbError::InvalidFile(
            "no fields found in E2 XML metadata".into(),
        ));
    }

    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_header() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..MAGIC.len()].copy_from_slice(MAGIC);
        // Space-pad to 64 bytes
        for b in &mut buf[MAGIC.len()..64] {
            *b = b' ';
        }
        // File ID
        buf[64..68].copy_from_slice(&FILE_ID.to_le_bytes());
        // Unknown constant
        buf[68..72].copy_from_slice(&0x40000001u32.to_le_bytes());
        // Metadata size = 500
        buf[96..100].copy_from_slice(&500u32.to_le_bytes());

        let header = E2Header::parse(&buf).unwrap();
        assert_eq!(header.file_id, FILE_ID);
        assert_eq!(header.metadata_size, 500);
    }

    #[test]
    fn reject_invalid_magic() {
        let buf = [0u8; HEADER_SIZE];
        assert!(E2Header::parse(&buf).is_err());
    }

    #[test]
    fn reject_wrong_file_id() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..MAGIC.len()].copy_from_slice(MAGIC);
        // Wrong file ID
        buf[64..68].copy_from_slice(&0x00440204u32.to_le_bytes());
        assert!(E2Header::parse(&buf).is_err());
    }

    #[test]
    fn parse_e2_xml_metadata() {
        let xml = r#"<RecordInfo>
            <Field name="ID" type="Int32" size="4" />
            <Field name="Name" type="V_WString" size="256" />
            <Field name="Score" type="Double" size="8" />
        </RecordInfo>"#;
        let fields = parse_meta_xml(xml).unwrap();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "ID");
        assert_eq!(fields[0].field_type, FieldType::Int32);
        assert_eq!(fields[1].name, "Name");
        assert_eq!(fields[1].field_type, FieldType::VWString);
        assert_eq!(fields[2].name, "Score");
        assert_eq!(fields[2].field_type, FieldType::Double);
    }
}
