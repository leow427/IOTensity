fn main() {
    println!("cargo:rerun-if-changed=src/sync/capture.m");
    println!("cargo:rerun-if-changed=src/overlay.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/sync/capture.m")
            .file("src/overlay.m")
            .flag("-fobjc-arc")
            .flag("-mmacosx-version-min=12.3")
            .compile("iotensity_capture");
        for framework in [
            "Foundation",
            "AppKit",
            "ScreenCaptureKit",
            "CoreGraphics",
            "CoreMedia",
            "CoreVideo",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
    tauri_build::build()
}
