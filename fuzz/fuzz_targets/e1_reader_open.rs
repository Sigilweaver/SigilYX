#![no_main]

use libfuzzer_sys::fuzz_target;
use sigilyx::YxdbReader;
use std::io::Write;

fuzz_target!(|data: &[u8]| {
    let Ok(mut file) = tempfile::NamedTempFile::new() else {
        return;
    };
    if file.write_all(data).is_err() {
        return;
    }

    if let Ok(reader) = YxdbReader::open(file.path()) {
        let _ = reader.into_dataframe();
    }
});
