//! # SigilYX
//!
//! A fast, safe Rust library for reading and writing Alteryx YXDB files, with native
//! Polars DataFrame integration.
//!
//! Not affiliated with Alteryx, Inc. "Alteryx" is a registered trademark of Alteryx, Inc.

pub mod error;
pub mod field;
pub mod header;
pub mod lzf;
pub mod record;

pub use error::{YxdbError, Result};
pub use field::{FieldType, FieldMeta};
