// The `ladybug` feature links liblbug.a (the prebuilt Ladybug engine), which
// bundles its own zstd and simsimd objects; the Lance crates link the same
// libraries through `zstd-sys`/`simsimd`. macOS ld64 silently takes the first
// definition, but GNU ld / lld on Linux reject the duplicates. Let the first
// copy (liblbug's, it is pulled in with --whole-archive) win for the binary
// only; every dependency crate is built exactly as before.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    record_git_revision();
    let ladybug = std::env::var_os("CARGO_FEATURE_LADYBUG").is_some();
    let linux = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux");
    if ladybug && linux {
        println!("cargo:rustc-link-arg-bins=-Wl,--allow-multiple-definition");
    }
}

/// Stamp the binary with the harness source revision so every report says
/// which harness produced it. A tree with uncommitted changes gets a
/// `-dirty` suffix; a checkout without git reports `unknown`.
fn record_git_revision() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let revision = match git(&["rev-parse", "HEAD"]) {
        Some(head) => match git(&["status", "--porcelain"]) {
            Some(status) if status.is_empty() => head,
            _ => format!("{head}-dirty"),
        },
        None => "unknown".to_string(),
    };
    println!("cargo:rustc-env=AG_GIT_REV={revision}");
}
