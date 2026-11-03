//! **TelegramViewer.exe beside the databases** — Settings › What an export
//! contains › Viewer beside the databases.
//!
//! The export folder is the thing that gets copied to a drive or handed to
//! somebody, and a database alone is a file nothing on their machine opens.
//! With the setting on, the folder carries its own reader: the small exe that
//! is the Viewer and nothing else (`crates/tt-viewer`), which opens on the
//! databases beside it.
//!
//! **Copied from beside this exe, never built or downloaded.** `save.bat
//! build` and the release put the two side by side; a TelegramTools.exe
//! copied somewhere on its own has nothing to copy, and says so.

use crate::bridge::{Event, Events};
use std::path::{Path, PathBuf};

/// The viewer's file name, here and in the export folder.
pub const VIEWER_EXE: &str = "TelegramViewer.exe";

/// What [`place`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Placed {
    /// Copied in, new or replacing an older one.
    Copied(PathBuf),
    /// The file there is already this one, byte for byte.
    Current,
}

/// Put `source` into `dir` as [`VIEWER_EXE`], unless the same bytes are
/// already there. The folder is made if it is not there yet: an export
/// folder that has never been written to is still where the databases go.
pub fn place(source: &Path, dir: &Path) -> Result<Placed, String> {
    if !source.is_file() {
        return Err(format!(
            "{VIEWER_EXE} is not beside TelegramTools.exe, so there is none to put in the \
             export folder. It ships in the same folder; copy it next to TelegramTools.exe."
        ));
    }
    let target = dir.join(VIEWER_EXE);
    if same_file_contents(source, &target) {
        return Ok(Placed::Current);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    // A copy still running from the folder holds its file open, and Windows
    // will not overwrite it; that is the likely cause, so it is named.
    std::fs::copy(source, &target).map_err(|e| {
        format!(
            "could not put {VIEWER_EXE} in {}: {e} — if it is open, close it and the next \
             sync will try again",
            dir.display()
        )
    })?;
    Ok(Placed::Copied(target))
}

/// Whether `b` exists and holds exactly `a`'s bytes. Sizes first, so a
/// different version is told apart without reading either.
fn same_file_contents(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    matches!((std::fs::read(a), std::fs::read(b)), (Ok(x), Ok(y)) if x == y)
}

/// Where the viewer this program ships with is: beside it.
pub fn shipped() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.with_file_name(VIEWER_EXE))
}

/// [`place`] off the window's thread. A copy is logged; a file already
/// current says nothing; a failure is a warning.
pub async fn place_in(output_dir: String, tx: Events) {
    let Some(source) = shipped() else {
        let _ = tx.send(Event::Warn(format!(
            "Could not tell where TelegramTools.exe is, so {VIEWER_EXE} was not copied"
        )));
        return;
    };
    let dir = PathBuf::from(&output_dir);
    let placed = tokio::task::spawn_blocking(move || place(&source, &dir))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    let _ = tx.send(match placed {
        Ok(Placed::Copied(to)) => Event::Log(format!("Put {VIEWER_EXE} in {}", parent_of(&to))),
        Ok(Placed::Current) => return,
        Err(e) => Event::Warn(e),
    });
}

fn parent_of(path: &Path) -> String {
    path.parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-place-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_viewer_is_copied_once_and_replaced_when_it_changes() {
        let root = tmp("copy");
        let source = root.join("ship").join(VIEWER_EXE);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"viewer v1").unwrap();
        // Into a folder that does not exist yet.
        let exports = root.join("Exports");
        let target = exports.join(VIEWER_EXE);
        assert_eq!(place(&source, &exports), Ok(Placed::Copied(target.clone())));
        assert_eq!(std::fs::read(&target).unwrap(), b"viewer v1");
        assert_eq!(place(&source, &exports), Ok(Placed::Current));
        // The same size, different bytes: still a new version.
        std::fs::write(&source, b"viewer v2").unwrap();
        assert_eq!(place(&source, &exports), Ok(Placed::Copied(target.clone())));
        assert_eq!(std::fs::read(&target).unwrap(), b"viewer v2");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_viewer_beside_the_exe_is_said_and_nothing_is_made() {
        let root = tmp("missing");
        let exports = root.join("Exports");
        let e = place(&root.join(VIEWER_EXE), &exports).unwrap_err();
        assert!(e.contains("not beside TelegramTools.exe"), "{e}");
        assert!(!exports.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
