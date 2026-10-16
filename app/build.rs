//! Sets OTOPI_VERSION: from the environment (Buildroot passes `git describe`,
//! because its copy of the source has no .git), else from `git describe`
//! here, else the crate version.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=OTOPI_GIT_VERSION");
    let git = Path::new("../.git");
    if git.exists() {
        for f in ["HEAD", "index", "refs"] {
            println!("cargo:rerun-if-changed=../.git/{f}");
        }
    }

    let version = std::env::var("OTOPI_GIT_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(git_describe)
        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
    println!("cargo:rustc-env=OTOPI_VERSION={version}");
}

fn git_describe() -> Option<String> {
    let out = Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .ok()?;
    let v = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}
