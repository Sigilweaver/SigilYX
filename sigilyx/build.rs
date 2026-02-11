fn main() {
    cc::Build::new()
        .file("csrc/lzf_d.c")
        .include("csrc")
        .define("AVOID_ERRNO", "1")
        // Block structure already validated by Rust reader — skip redundant checks
        .define("CHECK_INPUT", "0")
        // x64 has fast unaligned access; MSVC doesn't define __amd64 so the
        // default auto-detection in lzfP.h wrongly enables STRICT_ALIGN
        .define("STRICT_ALIGN", "0")
        .opt_level(3)
        .compile("lzf_c");
}
