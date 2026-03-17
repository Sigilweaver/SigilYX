//! E2 (AMP engine) YXDB format support.
//!
//! The E2 format uses a 100-byte header, UTF-8 XML metadata, Snappy compression,
//! and compact variable-length record encoding. Files begin with
//! `"Alteryx e2 Database file"`.

pub mod header;
pub mod reader;
pub mod record;
