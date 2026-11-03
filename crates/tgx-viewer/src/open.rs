//! Handing a stored file to Windows, for the Viewer's Open.

use std::path::Path;

/// What Open hands to Windows to open: pictures, video, sound, PDFs, text
/// and Office's macro-free documents. Anything else — an `.exe`, a script, a
/// shortcut, a macro document somebody posted — would run with this
/// account's rights on one click, so its folder opens instead.
const OPENABLE: [&str; 27] = [
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "heic", "mp4", "mov", "m4v", "webm",
    "mkv", "avi", "mp3", "m4a", "ogg", "oga", "opus", "wav", "flac", "pdf", "txt", "docx", "xlsx",
    "pptx",
];

/// Whether a file of this name is opened, rather than shown in its folder.
pub fn openable(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| OPENABLE.iter().any(|o| o.eq_ignore_ascii_case(e)))
}

/// Hand a folder or a file to the system: a folder opens in the file manager,
/// a file in whatever is registered for it — which is how Open shows a photo
/// without this app drawing one.
pub fn path(path: &Path) -> Result<(), String> {
    // Absolute, because CreateProcess searches the calling process's own
    // directory before PATH, and both programs are built to be copied into
    // arbitrary folders — TelegramViewer.exe into a folder of other people's
    // chats. A planted explorer.exe beside it would run with the user's rights.
    #[cfg(windows)]
    let spawned = std::process::Command::new(windows_dir().join("explorer.exe"))
        .arg(path)
        .spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(path).spawn();
    spawned
        .map(|_| ())
        .map_err(|e| format!("opening {}: {e}", path.display()))
}

#[cfg(windows)]
fn windows_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_type_that_cannot_run_is_opened() {
        for name in ["photo.JPG", "clip.mp4", "Zapisnik 01.12.docx", "scan.pdf"] {
            assert!(openable(name), "{name}");
        }
        for name in [
            "invoice.exe",
            "x.js",
            "x.hta",
            "report.pdf.lnk",
            "run.bat",
            "macro.docm",
            "old.doc",
            "page.html",
            "README",
        ] {
            assert!(!openable(name), "{name}");
        }
    }
}
