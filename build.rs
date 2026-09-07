// The `ladybug` feature links liblbug.a (the prebuilt Ladybug engine), which
// bundles its own zstd and simsimd objects; the Lance crates link the same
// libraries through `zstd-sys`/`simsimd`. macOS ld64 silently takes the first
// definition, but GNU ld / lld on Linux reject the duplicates. Let the first
// copy (liblbug's, it is pulled in with --whole-archive) win for the binary
// only; every dependency crate is built exactly as before.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    record_git_revision();
    record_grust_pin();
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
    // Dirty means the build could differ from the commit: any tracked change,
    // or an untracked file where the build reads (sources, scripts, the
    // manifests, the compose file). Untracked logs and report directories
    // in the checkout never touch the binary and do not count. The paths
    // that made a build dirty travel with it, so a report says what differed.
    let (revision, dirty_paths) = match git(&["rev-parse", "HEAD"]) {
        Some(head) => match git(&["status", "--porcelain", "--untracked-files=all"]) {
            Some(status) => {
                let relevant: Vec<String> = status
                    .lines()
                    .filter_map(|line| {
                        let (code, path) = line.split_at(2.min(line.len()));
                        let path = path.trim().rsplit(" -> ").next().unwrap_or("").to_string();
                        let untracked = code.starts_with("??");
                        let build_input = path.starts_with("src/")
                            || path.starts_with("scripts/")
                            || path.starts_with("scenarios/")
                            || matches!(
                                path.as_str(),
                                "Cargo.toml"
                                    | "Cargo.lock"
                                    | "build.rs"
                                    | "compose.yaml"
                                    | "datasets/MANIFEST.json"
                            );
                        (!untracked || build_input).then_some(path)
                    })
                    .collect();
                if relevant.is_empty() {
                    (head, String::new())
                } else {
                    (format!("{head}-dirty"), relevant.join(","))
                }
            }
            None => (format!("{head}-dirty"), String::new()),
        },
        None => ("unknown".to_string(), String::new()),
    };
    println!("cargo:rustc-env=AG_GIT_REV={revision}");
    println!("cargo:rustc-env=AG_GIT_DIRTY_PATHS={dirty_paths}");
}

/// Stamp the binary with the Grust it was built against, read from
/// Cargo.lock: the `grust-graph` facade version and the source every
/// `grust-core` copy resolves to (a registry version or a pinned git
/// revision), so a report names the exact adapter code it measured.
fn record_grust_pin() {
    println!("cargo:rerun-if-changed=Cargo.lock");
    let lock = std::fs::read_to_string("Cargo.lock").unwrap_or_default();
    let field = |package: &str, key: &str| {
        lock.split("[[package]]")
            .find(|block| block.contains(&format!("name = \"{package}\"")))
            .and_then(|block| {
                block
                    .lines()
                    .find_map(|line| line.trim().strip_prefix(&format!("{key} = ")))
            })
            .map(|value| value.trim_matches('"').to_string())
            .unwrap_or_else(|| "unknown".to_string())
    };
    println!(
        "cargo:rustc-env=AG_GRUST_GRAPH_VERSION={}",
        field("grust-graph", "version")
    );
    println!(
        "cargo:rustc-env=AG_GRUST_CORE_SOURCE={}",
        field("grust-core", "source")
    );
}
