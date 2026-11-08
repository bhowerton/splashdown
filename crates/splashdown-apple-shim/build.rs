//! Builds the Swift shim (swift/) as a static library and links it, with the
//! Swift runtime and Apple's frameworks, and extracts its App Intents
//! (Shortcuts, Spotlight and Siri actions) into `Metadata.appintents` for
//! the app bundle. macOS only; elsewhere the crate is empty.

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
        Ok("x86_64") => "x86_64-apple-macosx12.0",
        _ => "arm64-apple-macosx12.0",
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

    // The compiler records the App Intents' constant values (titles,
    // parameters, phrases) for the metadata step below. Release builds
    // compile the module as a whole, so there's one file.
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let protocols = out.join("app-intents-protocols.json");
    std::fs::write(&protocols, APP_INTENTS_PROTOCOLS).unwrap();
    let const_values = out.join("SplashdownAppleShim.swiftconstvalues");
    let const_values_arg = const_values.display().to_string();
    let protocols_arg = protocols.display().to_string();
    let status = swift_args(&[
        "-Xswiftc",
        "-emit-const-values-path",
        "-Xswiftc",
        &const_values_arg,
        "-Xswiftc",
        "-Xfrontend",
        "-Xswiftc",
        "-const-gather-protocols-file",
        "-Xswiftc",
        "-Xfrontend",
        "-Xswiftc",
        &protocols_arg,
    ])
    .status()
    .expect("run `swift build` (needs Xcode)");
    assert!(status.success(), "building the Swift shim failed");

    let bin = swift_args(&["--show-bin-path"])
        .output()
        .expect("swift build --show-bin-path");
    let bin = String::from_utf8(bin.stdout).unwrap().trim().to_string();
    println!("cargo:rustc-link-search=native={bin}");
    // The whole library: nothing in Rust refers to the App Intents' types,
    // and the system finds them in the binary by name.
    println!("cargo:rustc-link-lib=static:+whole-archive=SplashdownAppleShim");

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
        "ImageIO",
        "CoreGraphics",
        "UniformTypeIdentifiers",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    // FoundationModels, AppIntents and ScreenCaptureKit, newer than the
    // app's minimum, are linked weakly by .cargo/config.toml.

    // `toolchain` is …/XcodeDefault.xctoolchain/usr.
    let toolchain_dir = toolchain.parent().unwrap();
    app_intents_metadata(&package, &const_values, &sdk, toolchain_dir, triple);
}

/// The protocols whose conformances the compiler records constant values
/// for: the list Xcode passes for App Intents.
const APP_INTENTS_PROTOCOLS: &str = r#"["AppIntent","EntityQuery","AppEntity","TransientEntity","AppEnum","AppShortcutProviding","AppShortcutsProvider","AnyResolverProviding","AppIntentsPackage","DynamicOptionsProvider","_IntentValueRepresentable","_AssistantIntentsProvider","_GenerativeFunctionExtractable","IntentValueQuery"]"#;

/// Runs Xcode's `appintentsmetadataprocessor` on the shim's sources and
/// constant values. The result goes to `target/appintents/
/// Metadata.appintents` (a fixed place, so the bundle steps can find it:
/// tauri.release.conf.json and scripts/run-mac-app.sh), and the app bundle
/// carries it in Contents/Resources. It's the same for both halves of a
/// universal build.
fn app_intents_metadata(
    package: &std::path::Path,
    const_values: &std::path::Path,
    sdk: &str,
    toolchain: &std::path::Path,
    triple: &str,
) {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut sources: Vec<String> = std::fs::read_dir(package.join("Sources/SplashdownAppleShim"))
        .expect("read the Swift sources")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "swift"))
        .map(|p| p.display().to_string())
        .collect();
    sources.sort();
    let source_list = out.join("app-intents-sources.txt");
    std::fs::write(&source_list, sources.join("\n") + "\n").unwrap();
    let values_list = out.join("app-intents-values.txt");
    std::fs::write(&values_list, format!("{}\n", const_values.display())).unwrap();
    let empty_list = out.join("app-intents-none.txt");
    std::fs::write(&empty_list, "").unwrap();
    let staged = out.join("app-intents");
    let _ = std::fs::remove_dir_all(&staged);
    std::fs::create_dir_all(&staged).unwrap();

    let xcode = xcode_build_version();
    let status = Command::new("xcrun")
        .arg("appintentsmetadataprocessor")
        .arg("--output")
        .arg(&staged)
        .arg("--toolchain-dir")
        .arg(toolchain)
        .args(["--module-name", "SplashdownAppleShim"])
        .args(["--sdk-root", sdk])
        .args(["--xcode-version", &xcode])
        .args(["--platform-family", "macOS"])
        .args(["--deployment-target", "12.0"])
        .args(["--target-triple", triple])
        .arg("--source-file-list")
        .arg(&source_list)
        .arg("--swift-const-vals-list")
        .arg(&values_list)
        .arg("--metadata-file-list")
        .arg(&empty_list)
        .arg("--static-metadata-file-list")
        .arg(&empty_list)
        .args([
            "--compile-time-extraction",
            "--force",
            "--no-app-shortcuts-localization",
        ])
        .output()
        .expect("run appintentsmetadataprocessor (needs Xcode)");
    if !status.status.success() {
        panic!(
            "extracting the App Intents metadata failed:\n{}",
            String::from_utf8_lossy(&status.stderr)
        );
    }
    let built = staged.join("Metadata.appintents");
    assert!(
        built.join("extract.actionsdata").is_file(),
        "appintentsmetadataprocessor wrote no Metadata.appintents"
    );

    // <workspace>/target/appintents, or under CARGO_TARGET_DIR.
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../target")
        });
    let dest = target.join("appintents/Metadata.appintents");
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).unwrap();
    for entry in std::fs::read_dir(&built).unwrap().flatten() {
        std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
    }
}

/// Xcode's build number (e.g. 17B100), which the metadata records.
fn xcode_build_version() -> String {
    let out = Command::new("xcodebuild")
        .arg("-version")
        .output()
        .expect("xcodebuild -version");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("Build version "))
        .unwrap_or("17A000")
        .trim()
        .to_string()
}

fn xcrun(args: &[&str]) -> String {
    let out = Command::new("xcrun").args(args).output().expect("xcrun");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
