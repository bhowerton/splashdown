//! Links clang's runtime for local speech-to-text on macOS. transcribe.cpp's
//! Metal backend (ggml's) uses `@available` checks, which call into
//! compiler-rt when it's built for an older macOS than the SDK (our minimum
//! is 13). rustc links with -nodefaultlibs, so the runtime isn't included
//! otherwise. (The splashdown-llm helper does the same for llama.cpp.)

use std::env;
use std::process::Command;

fn main() {
    let macos = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    let local_speech = env::var_os("CARGO_FEATURE_LOCAL_SPEECH").is_some();
    if !(macos && local_speech) {
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
