fn main() {
    let mut revision = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into());
    if std::process::Command::new("git")
        .args(["diff-index", "--quiet", "HEAD", "--"])
        .status()
        .is_ok_and(|status| status.code() == Some(1))
    {
        revision.push_str("-dirty");
    }
    println!("cargo:rustc-env=PENNEL_BUILD_REVISION={revision}");
    println!("cargo:rerun-if-changed=../../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../../.git/refs/heads");
    link_clang_runtime_on_macos();
    build_macos_platform();
    tauri_build::build()
}

fn xcrun_output(args: &[&str]) -> String {
    let output = std::process::Command::new("xcrun")
        .args(args)
        .output()
        .expect("Xcode command-line tools are required for the macOS platform library");
    assert!(
        output.status.success(),
        "xcrun {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Xcode paths must be UTF-8")
        .trim()
        .to_owned()
}

fn build_macos_platform() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest.join("../../macos-platform/Sources/PennelMacPlatform");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let archive = output.join("libPennelMacPlatform.a");
    println!("cargo:rerun-if-changed={}", source.display());

    let architecture = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => panic!("Unsupported macOS architecture: {other:?}"),
    };
    let sdk = xcrun_output(&["--sdk", "macosx", "--show-sdk-path"]);
    let compiler = std::path::PathBuf::from(xcrun_output(&["--find", "swiftc"]));
    let mut sources: Vec<_> = std::fs::read_dir(&source)
        .expect("Missing PennelMacPlatform Swift sources")
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "swift")
        })
        .collect();
    sources.sort();
    let status = std::process::Command::new(&compiler)
        .args([
            "-parse-as-library",
            "-swift-version",
            "5",
            "-O",
            "-emit-library",
            "-static",
            "-module-name",
            "PennelMacPlatform",
            "-target",
            &format!("{architecture}-apple-macosx14.4"),
            "-sdk",
            &sdk,
        ])
        .args(sources)
        .arg("-o")
        .arg(&archive)
        .status()
        .expect("Failed to start Swift compiler");
    assert!(
        status.success(),
        "PennelMacPlatform Swift compilation failed"
    );

    let runtime = compiler
        .parent()
        .and_then(std::path::Path::parent)
        .expect("Invalid Swift compiler path")
        .join("lib/swift/macosx");
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-search=native={}", runtime.display());
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    println!("cargo:rustc-link-lib=static=PennelMacPlatform");
    // Foundation Models is optional at runtime so the same bundle still starts on macOS 14.4.
    println!("cargo:rustc-link-arg=-Wl,-weak_framework,FoundationModels");
    // The toolchain's Swift import libraries can use @rpath install names. A -L path only
    // helps the linker; cargo test executables and installed bundles need runtime paths too.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    for framework in [
        "AppKit",
        "ApplicationServices",
        "AVFoundation",
        "CoreAudio",
        "Contacts",
        "ContactsUI",
        "CryptoKit",
        "EventKit",
        "Speech",
        "Security",
        "LocalAuthentication",
        "Foundation",
        "UniformTypeIdentifiers",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

/// The Metal backend of transcribe.cpp uses `@available` checks, which need `___isPlatformVersionAtLeast`
/// from clang's runtime. Xcode links it for Swift targets; a plain cargo link does not.
fn link_clang_runtime_on_macos() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let Ok(output) = std::process::Command::new("clang")
        .arg("--print-runtime-dir")
        .output()
    else {
        return;
    };
    let dir = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !dir.is_empty() {
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
    }
}
