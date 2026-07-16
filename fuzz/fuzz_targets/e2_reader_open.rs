#![no_main]

use libfuzzer_sys::fuzz_target;
use sigilyx::E2Reader;
use std::io::Write;

fuzz_target!(|data: &[u8]| {
    let Ok(mut file) = tempfile::NamedTempFile::new() else {
        return;
    };
    if file.write_all(data).is_err() {
        return;
    }

    if let Ok(mut reader) = E2Reader::open(file.path()) {
        // Exercise the speculative decoders (Time, WString, Blob, SpatialObj)
        // too, not just the verified E2 field types.
        reader.set_allow_unverified(true);
        let _ = reader.into_dataframe();
    }
});
