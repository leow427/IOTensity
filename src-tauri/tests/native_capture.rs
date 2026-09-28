#![cfg(target_os = "macos")]

#[test]
fn screen_capture_bridge_validates_status_samples_and_terminal_states() {
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("capture-test");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/capture.m");
    let compiled = std::process::Command::new("xcrun")
        .args([
            "clang",
            "-fobjc-arc",
            "-fblocks",
            "-Werror",
            "-mmacosx-version-min=12.3",
        ])
        .arg(source)
        .args([
            "-framework",
            "Foundation",
            "-framework",
            "AppKit",
            "-framework",
            "ScreenCaptureKit",
            "-framework",
            "CoreGraphics",
            "-framework",
            "CoreVideo",
            "-framework",
            "CoreMedia",
            "-o",
        ])
        .arg(&binary)
        .output()
        .expect("Xcode command-line tools are required for macOS native tests");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let result = std::process::Command::new(binary).output().unwrap();
    assert!(
        result.status.success(),
        "native capture regression failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
