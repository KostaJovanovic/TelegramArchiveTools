//! Hand a finished report to whatever opens HTML.
//!
//! **The Windows path is absolute, and that is a security decision rather than
//! a tidiness one.** `CreateProcess` searches the calling process's own
//! directory before `PATH`, and this binary is built to be copied into
//! arbitrary folders — an `explorer.exe` planted beside it would run with the
//! user's rights. The exporter learned the same lesson twice: once porting
//! `ensure_data_dir` (a bare `icacls.exe`), and once with `open_folder`, which
//! looked in System32 for a binary that lives directly in `%SystemRoot%` and so
//! silently did nothing on every press.

use std::path::{Path, PathBuf};

/// `%SystemRoot%\<name>`, falling back to the usual location.
///
/// The environment variable is read rather than assumed, because a machine
/// with Windows on another volume is unusual and not impossible, and the
/// fallback is only there so this cannot return nothing.
#[cfg(windows)]
fn system_root(name: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join(name)
}

/// Open `file` with the system's default handler.
///
/// Returns what went wrong, if anything. **The result is not discarded**: a
/// press that spawns nothing and reports nothing is indistinguishable from a
/// press that was not registered, which is exactly the bug the exporter shipped
/// for a while.
pub fn open(file: &Path) -> Result<(), String> {
    if !file.is_file() {
        return Err(format!("{} is not there any more", file.display()));
    }

    // `explorer.exe <file>` is what `os.startfile` reduces to: it hands the
    // path to the shell, which opens it with the registered handler — a
    // browser, for HTML.
    #[cfg(windows)]
    let spawned = std::process::Command::new(system_root("explorer.exe"))
        .arg(file)
        .spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(file).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(file).spawn();

    spawned
        .map(|_| ())
        .map_err(|e| format!("opening {}: {e}", file.display()))
}

/// Show a folder in the system's file manager.
///
/// Its own function rather than a relaxed [`open`], because `open` refusing
/// anything that is not a file is the check that stops a missing report from
/// spawning a shell dialog this app did not write.
pub fn reveal(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("{} is not there any more", dir.display()));
    }
    #[cfg(windows)]
    let spawned = std::process::Command::new(system_root("explorer.exe"))
        .arg(dir)
        .spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(dir).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(dir).spawn();

    spawned
        .map(|_| ())
        .map_err(|e| format!("opening {}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_that_is_no_longer_there_says_so_rather_than_spawning() {
        // The report can be deleted or the drive unplugged between the run and
        // the press. Spawning the shell on a missing path opens a dialog the
        // app did not write and cannot explain.
        let missing = std::env::temp_dir().join("tga-app-no-such-report.html");
        let _ = std::fs::remove_file(&missing);
        let error = open(&missing).expect_err("a missing file must not spawn anything");
        assert!(error.contains("not there any more"), "{error}");
    }

    #[cfg(windows)]
    #[test]
    fn the_shell_is_named_by_absolute_path() {
        // A bare `explorer.exe` would let a copy planted beside this binary run
        // with the user's rights.
        let path = system_root("explorer.exe");
        assert!(path.is_absolute(), "{}", path.display());
        assert!(path.ends_with("explorer.exe"));
    }
}
