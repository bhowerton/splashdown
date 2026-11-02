//! Builds the Swift shim (swift/) as a static library and links it, with the
//! Swift runtime and Apple's frameworks. macOS only; elsewhere the crate is
//! empty.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=swift/Package.swift");
    println!("cargo:rerun-if-changed=swift/Sources");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let package = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("swift");
    let scratch = PathBuf::from(env::var("OUT_DIR").unwrap()).join("swift-build");
    let triple = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x86_64-apple-macosx13.0",
        _ => "arm64-apple-macosx13.0",
    };
    let swift_args = |extra: &[&str]| {
        let mut cmd = Command::new("xcrun");
        cmd.arg("swift")
            .arg("build")
            .args(["-c", "release", "--triple", triple])
            .arg("--package-path")
            .arg(&package)
            .arg("--scratch-path")
            .arg(&scratch)
            .args(extra);
        cmd
    };

    let status = swift_args(&[])
        .status()
        .expect("run `swift build` (needs Xcode)");
    assert!(status.success(), "building the Swift shim failed");

    let bin = swift_args(&["--show-bin-path"])
        .output()
        .expect("swift build --show-bin-path");
    let bin = String::from_utf8(bin.stdout).unwrap().trim().to_string();
    println!("cargo:rustc-link-search=native={bin}");
    println!("cargo:rustc-link-lib=static=SplashdownAppleShim");

    // The Swift runtime: the OS copy at run time, the SDK's stubs and the
    // toolchain's compatibility libraries at link time.
    let sdk = xcrun(&["--show-sdk-path"]);
    let swiftc = xcrun(&["--find", "swiftc"]);
    let toolchain = PathBuf::from(swiftc)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    println!(
        "cargo:rustc-link-search=native={}/lib/swift/macosx",
        toolchain.display()
    );
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

    for framework in [
        "Foundation",
        "AVFAudio",
        "Speech",
        "CoreMedia",
        "CoreAudio",
        "AudioToolbox",
        "ScreenCaptureKit",
        "ImageIO",
        "CoreGraphics",
        "UniformTypeIdentifiers",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    // FoundationModels (macOS 26+) is linked weakly by .cargo/config.toml.
}

fn xcrun(args: &[&str]) -> String {
    let out = Command::new("xcrun").args(args).output().expect("xcrun");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
