//! Links clang's runtime on macOS: llama.cpp's Metal backend has
//! `@available` checks, which call into compiler-rt when built for an older
//! macOS than the SDK (our minimum is 13), and rustc links with
//! -nodefaultlibs. (splashdown-providers' build script does the same for
//! whisper.cpp.)

use std::env;
use std::process::Command;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let clang = Command::new("xcrun")
        .args(["--find", "clang"])
        .output()
        .expect("xcrun (needs the Xcode command-line tools)");
    let clang = String::from_utf8(clang.stdout).unwrap();
    let resource_dir = Command::new(clang.trim())
        .arg("--print-resource-dir")
        .output()
        .expect("clang --print-resource-dir");
    let resource_dir = String::from_utf8(resource_dir.stdout).unwrap();
    println!(
        "cargo:rustc-link-search=native={}/lib/darwin",
        resource_dir.trim()
    );
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
