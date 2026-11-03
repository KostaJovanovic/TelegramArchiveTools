//! Embed the icon and the Windows version resource into `TelegramViewer.exe`,
//! as `tt-app/build.rs` does for `TelegramTools.exe` — see there for why, and
//! why a failure here is a warning and not a broken build. The icon is
//! TelegramTools' own: the two are one product.

fn main() {
    #[cfg(windows)]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed();
    }
}

#[cfg(windows)]
fn embed() {
    const ICON: &str = "../tt-app/icon/TelegramTools.ico";
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed=build.rs");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(ICON);
    resource.set("ProductName", "TelegramTools");
    resource.set("FileDescription", "TelegramViewer");
    resource.set("LegalCopyright", "Copyright (C) 2026 Kosta Jovanovic");
    resource.set("OriginalFilename", "TelegramViewer.exe");
    if let Err(e) = resource.compile() {
        println!("cargo:warning=no version resource or icon embedded: {e}");
    }
}
