// The `ladybug` feature links liblbug.a (the prebuilt Ladybug engine), which
// bundles its own zstd and simsimd objects; the Lance crates link the same
// libraries through `zstd-sys`/`simsimd`. macOS ld64 silently takes the first
// definition, but GNU ld / lld on Linux reject the duplicates. Let the first
// copy (liblbug's, it is pulled in with --whole-archive) win for the binary
// only; every dependency crate is built exactly as before.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let ladybug = std::env::var_os("CARGO_FEATURE_LADYBUG").is_some();
    let linux = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux");
    if ladybug && linux {
        println!("cargo:rustc-link-arg-bins=-Wl,--allow-multiple-definition");
    }
}
