//! E1 (original Alteryx engine) YXDB format support.
//!
//! The E1 format uses a 512-byte header, UTF-16LE XML metadata, LZF compression,
//! and fixed-size record layouts. Files begin with `"Alteryx Database File"`.

pub mod header;
pub mod lzf;
pub mod reader;
pub mod record;
pub mod writer;
