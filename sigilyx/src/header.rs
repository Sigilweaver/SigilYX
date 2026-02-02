use crate::error::{YxdbError, Result};
use crate::field::{FieldMeta, FieldType};

/// YXDB file header — always 512 bytes.
///
/// ## Layout
/// - Bytes 0..21: Magic string `"Alteryx Database File"` (ASCII, null-padded)
/// - Bytes 80..84: `meta_info_size` (u32 LE) — number of **UTF-16 code units**
///   in the XML metadata that follows the header (including a null terminator)
/// - Bytes 104..112: `num_records` (u64 LE)
///
/// The XML metadata immediately follows the 512-byte header and is encoded as
/// little-endian UTF-16. Its byte length is `meta_info_size * 2`.
#[derive(Debug)]
pub struct YxdbHeader {
    pub num_records: u64,
    pub meta_info_size: u32,
}

pub const HEADER_SIZE: usize = 512;
const MAGIC: &[u8] = b"Alteryx Database File";

impl YxdbHeader {
    /// Parse a 512-byte header buffer.
    pub fn parse(buf: &[u8; HEADER_SIZE]) -> Result<Self> {
        if &buf[0..MAGIC.len()] != MAGIC {
            return Err(YxdbError::InvalidFile(
                "file does not start with 'Alteryx Database File'".into(),
            ));
        }

        let meta_info_size = u32::from_le_bytes(buf[80..84].try_into().unwrap());
        let num_records = u64::from_le_bytes(buf[104..112].try_into().unwrap());

        Ok(YxdbHeader {
            num_records,
            meta_info_size,
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
                            name = String::from_utf8_lossy(&attr.value).to_string();
                        }
                        b"type" => {
                            type_str = String::from_utf8_lossy(&attr.value).to_string();
                        }
                        b"size" => {
                            size = String::from_utf8_lossy(&attr.value)
                                .parse()
                                .unwrap_or(0);
                        }
                        b"scale" => {
                            scale = String::from_utf8_lossy(&attr.value)
                                .parse()
                                .unwrap_or(0);
                        }
                        _ => {}
                    }
                }

                let field_type = FieldType::from_str(&type_str).ok_or_else(|| {
                    YxdbError::UnsupportedFieldType(type_str.clone())
                })?;

                let current_offset = offset;
                offset += field_type.fixed_bytes(size);

                fields.push(FieldMeta {
                    name,
                    field_type,
                    size,
                    scale,
                    offset: current_offset,
                });
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
}
