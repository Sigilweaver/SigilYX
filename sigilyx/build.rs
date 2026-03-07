fn main() {
    // STRICT_ALIGN: x86_64 has fast unaligned access, but ARM targets
    // may suffer slowdowns or SIGBUS on unaligned reads. Enable strict
    // alignment for non-x86_64 targets. MSVC doesn't define __amd64 so
    // the auto-detection in lzfP.h is unreliable — set explicitly.
    let strict_align = if std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default() == "x86_64" {
        "0"
    } else {
        "1"
    };

    cc::Build::new()
        .file("csrc/lzf_d.c")
        .include("csrc")
        .define("AVOID_ERRNO", "1")
        // Block structure already validated by Rust reader — skip redundant checks
        .define("CHECK_INPUT", "0")
        .define("STRICT_ALIGN", strict_align)
        // Duff's device uses intentional switch fallthrough — suppress the warning
        .flag_if_supported("-Wno-implicit-fallthrough")
        .opt_level(3)
        .compile("lzf_c");
}
