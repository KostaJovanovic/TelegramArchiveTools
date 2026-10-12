//! Embed the icon and the Windows version resource into `TelegramAnalyser.exe`.
//!
//! Without this the Properties dialog shows an exe with no product name, no
//! version and no copyright, and Explorer draws the generic binary glyph. The
//! icon is `icon/TelegramAnalyser.ico`, drawn by `tools/make_icon.py` from the
//! design tokens — regenerate it there rather than editing the file.
//!
//! The exporter's `build.rs`, nearly line for line, for the same reasons:
//! unsigned on purpose, and never a hard failure.

fn main() {
    // Two guards for two directions of cross-compilation. `cfg(windows)` here
    // is the host, and matches the Cargo.toml gate that decides whether the
    // crate exists; `CARGO_CFG_WINDOWS` is the target, and a Windows host
    // building for Linux has no PE to put a resource in.
    #[cfg(windows)]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed();
    }
}

#[cfg(windows)]
fn embed() {
    println!("cargo:rerun-if-changed=icon/TelegramAnalyser.ico");
    println!("cargo:rerun-if-changed=build.rs");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("icon/TelegramAnalyser.ico");
    // FileVersion and ProductVersion come from CARGO_PKG_VERSION already: two
    // places to change a version number is one place to forget one.
    resource.set("ProductName", "Telegram Export Analyser");
    resource.set("FileDescription", "Telegram Export Analyser");
    resource.set("LegalCopyright", "Copyright (C) 2026 Kosta Jovanovic");
    resource.set("OriginalFilename", "TelegramAnalyser.exe");

    // Never fail the build over this. Compiling a resource needs `rc.exe` from
    // the Windows SDK, and a developer with a toolchain but no SDK still has to
    // be able to `cargo build`: an exe with no icon runs exactly as well.
    if let Err(e) = resource.compile() {
        println!("cargo:warning=no version resource or icon embedded: {e}");
    }
}
