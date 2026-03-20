use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=CHVM_GIT_HASH");

    if let Some(git_dir) = git_dir() {
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join("packed-refs").display()
        );
        println!("cargo:rerun-if-changed={}", git_dir.join("index").display());
    }

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let git_hash = git_hash().unwrap_or_else(|| "unknown".to_string());
    let dirty_suffix = if git_is_dirty().unwrap_or(false) {
        "-dirty"
    } else {
        ""
    };
    let full_version = format!("{version} ({git_hash}{dirty_suffix})");

    println!("cargo:rustc-env=CHVM_GIT_HASH={git_hash}");
    println!("cargo:rustc-env=CHVM_FULL_VERSION={full_version}");
}

fn git_dir() -> Option<PathBuf> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Some(path)
    } else {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
        Some(PathBuf::from(manifest_dir).join(path))
    }
}

fn git_hash() -> Option<String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

fn git_is_dirty() -> Option<bool> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(!output.stdout.is_empty())
}
