use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[cfg(target_os = "windows")]
use winres::WindowsResource;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/tags");

    let describe = Command::new("git")
        .args(["describe", "--tags", "--abbrev=0"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() { String::from_utf8(output.stdout).ok() } else { None }
        })
        .map(|raw| raw.trim().to_string())
        .filter(|value| !value.is_empty());

    let version = describe.unwrap_or_else(|| {
        std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string())
    });

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::copy("assets/icon.png", out_dir.join("icon.png")).expect(
        "Failed to copy icon.png to output directory"
    );
    println!("cargo:rerun-if-changed=assets/icon.png");

    compile_windows_resources();

    println!("cargo:rustc-env=SC_LOG_ANALYZER_VERSION={}", version);
}

#[cfg(target_os = "windows")]
fn compile_windows_resources() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    let mut res = WindowsResource::new();
    res.set_icon("assets/icon.ico");
    res.compile().expect("Failed to embed Windows resources");
}

#[cfg(not(target_os = "windows"))]
fn compile_windows_resources() {}
