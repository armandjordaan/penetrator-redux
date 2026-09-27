//! Build script.
//!
//! Its only job is to work around a papercut that stops the game linking on an
//! otherwise perfectly capable Linux machine.
//!
//! macroquad's audio backend links `-lasound`. To resolve that, the linker needs
//! a file called exactly `libasound.so` — and that bare name ships in
//! `libasound2-dev`, not in the runtime package. So a machine that can play
//! sound perfectly well, and has `libasound.so.2` sitting right there, still
//! fails to link with:
//!
//! ```text
//! rust-lld: error: unable to find library -lasound
//! ```
//!
//! Installing the dev package is the tidy fix and needs root. This script does
//! the same thing without it: if the bare `libasound.so` name is missing but the
//! versioned runtime library exists, it drops a symlink under `OUT_DIR` and adds
//! that directory to the link search path.
//!
//! The resulting binary is identical either way — the `SONAME` recorded inside
//! `libasound.so.2` is what ends up in the `DT_NEEDED` entry, not the name of the
//! symlink used to find it.
//!
//! Nothing here runs unless it is needed:
//!
//! * not on Linux — returns immediately;
//! * built with `--no-default-features` — the audio backend is compiled out, so
//!   nothing links `-lasound` and there is nothing to fix;
//! * `libasound2-dev` already installed — the real symlink is found and this
//!   leaves it alone.
//!
//! If the runtime library genuinely is not there, the script says so and points
//! at the two real fixes rather than failing with a linker error nobody can read.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Directories to probe when `ldconfig` is unavailable or unhelpful.
const LIB_DIRS: &[&str] = &[
    "/usr/lib/x86_64-linux-gnu",
    "/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
    "/lib/aarch64-linux-gnu",
    "/usr/lib64",
    "/usr/lib",
    "/lib",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if !cfg!(target_os = "linux") {
        return;
    }
    // Set by Cargo when this crate's `audio` feature is on. Without it macroquad
    // compiles its audio backend out and never asks for `-lasound`.
    if std::env::var_os("CARGO_FEATURE_AUDIO").is_none() {
        return;
    }
    if dev_symlink_present() {
        return;
    }

    let Some(runtime) = find_runtime_library() else {
        // Not an error here — let the linker be the one to complain, but explain
        // it first, because its message on its own is baffling.
        println!(
            "cargo:warning=libasound was not found. Install the ALSA development \
             package (libasound2-dev / alsa-lib-devel) to build with sound, or \
             build with --no-default-features to leave it out."
        );
        return;
    };

    match link_shim(&runtime) {
        Ok(dir) => {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!(
                "cargo:warning=libasound2-dev is not installed; linking against \
                 {} directly. Install the dev package to silence this.",
                runtime.display()
            );
        }
        Err(e) => {
            println!(
                "cargo:warning=could not link against {}: {e}. Install \
                 libasound2-dev, or build with --no-default-features.",
                runtime.display()
            );
        }
    }
}

/// True if the linker can already resolve `-lasound` by itself.
fn dev_symlink_present() -> bool {
    LIB_DIRS
        .iter()
        .any(|dir| Path::new(dir).join("libasound.so").exists())
}

/// Locates the versioned runtime library, preferring `ldconfig`'s answer over
/// guessing at directories.
fn find_runtime_library() -> Option<PathBuf> {
    for ldconfig in ["ldconfig", "/sbin/ldconfig", "/usr/sbin/ldconfig"] {
        let Ok(output) = Command::new(ldconfig).arg("-p").output() else {
            continue;
        };
        let listing = String::from_utf8_lossy(&output.stdout);
        for line in listing.lines() {
            // e.g. "  libasound.so.2 (libc6,x86-64) => /lib/x86_64-linux-gnu/libasound.so.2"
            if !line.contains("libasound.so.2") {
                continue;
            }
            if let Some(path) = line.split("=>").nth(1) {
                let path = PathBuf::from(path.trim());
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }

    LIB_DIRS
        .iter()
        .map(|dir| Path::new(dir).join("libasound.so.2"))
        .find(|p| p.exists())
}

/// Creates `OUT_DIR/libasound.so` pointing at the runtime library, and returns
/// the directory to add to the link search path.
fn link_shim(runtime: &Path) -> std::io::Result<PathBuf> {
    let out_dir = PathBuf::from(
        std::env::var_os("OUT_DIR").ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "OUT_DIR is not set")
        })?,
    );
    let shim = out_dir.join("libasound.so");

    // Rebuilds reuse OUT_DIR, so clear any previous link first.
    if shim.symlink_metadata().is_ok() {
        std::fs::remove_file(&shim)?;
    }
    std::os::unix::fs::symlink(runtime, &shim)?;
    Ok(out_dir)
}
