//! **Which chats have a database in a folder**, read from inside the files.
//!
//! Here rather than in the engine because two programs ask it: the exporter,
//! to sync exactly the chats that already have a file, and the Viewer — the
//! window's and `TelegramViewer.exe`, which links no Telegram client at all
//! and so cannot reach anything in `tgx-tg`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What [`archived_chats`] found in a folder.
#[derive(Debug, Clone, Default)]
pub struct Archived {
    /// Every chat a database there records, as `(id, title)`, once each.
    pub chats: Vec<(i64, String)>,
    /// Which file each of those came from.
    pub files: HashMap<i64, PathBuf>,
    /// When each last finished a sync that read to the end — see
    /// [`crate::ChatEntry::synced`]. Absent for one never finished.
    pub synced: HashMap<i64, i64>,
    /// The databases that could not be read, and why — another schema
    /// version, a file that is not ours, a lock that outlasted the wait.
    pub unreadable: Vec<(PathBuf, String)>,
}

/// **The chats that already have a database in `dir`**, so a run can sync
/// exactly those.
///
/// Looks where the exporter puts a file — `<dir>/<name>/<name>.sqlite` — and
/// at the flat `<dir>/<name>.sqlite` an earlier build wrote, which a run then
/// moves into its folder. Nothing deeper: a Classic export folder holds no
/// `.sqlite` of ours, and walking every topic folder of every export to prove
/// it would be the slowest part of the button.
///
/// The chat comes from **inside** the file, never its name. Two chats can
/// sanitise to one name, and a title changed on Telegram since the last run
/// would otherwise match nothing.
pub async fn archived_chats(dir: &Path) -> std::io::Result<Archived> {
    let found = database_files(dir)?;
    let mut out = Archived::default();
    let mut seen = HashSet::new();
    for path in found {
        match crate::Store::chats_in(&path).await {
            // **The one-file-for-everything build's `telegram.sqlite`.** A run
            // never writes into such a file again — the chat's own name is
            // looked for instead — so offering its chats would sync them into
            // fresh files while appearing to update this one.
            Ok(chats) if chats.len() > 1 => out.unreadable.push((
                path,
                format!(
                    "holds {} chats — a file from before one database per chat, \
                     which no run writes into",
                    chats.len()
                ),
            )),
            // **Several files for one chat: the last one synced wins.** Builds
            // before the window looked a chat's file up by id opened the
            // current title's instead, so a chat renamed between syncs left a
            // database per name. The newest is the one still moving; the
            // alphabetically first was an old name, and syncing into it would
            // move the chat back to a folder the user stopped looking at.
            Ok(chats) => {
                for chat in chats {
                    if seen.insert(chat.id) {
                        out.files.insert(chat.id, path.clone());
                        if let Some(at) = chat.synced {
                            out.synced.insert(chat.id, at);
                        }
                        out.chats.push((chat.id, chat.title));
                    } else if chat.synced > out.synced.get(&chat.id).copied() {
                        out.files.insert(chat.id, path.clone());
                        out.synced.extend(chat.synced.map(|at| (chat.id, at)));
                        if let Some(row) = out.chats.iter_mut().find(|(id, _)| *id == chat.id) {
                            row.1 = chat.title;
                        }
                    }
                }
            }
            Err(e) => out.unreadable.push((path, e.to_string())),
        }
    }
    Ok(out)
}

/// The database files directly in `dir` and one folder down, where the
/// exporter puts them, sorted — so the queue comes out in the same order on
/// every press, and a tie between two files goes the same way.
pub fn database_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let ext = crate::FILE_EXT;
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                let inner = path.join(format!("{name}.{ext}"));
                if inner.is_file() {
                    found.push(inner);
                }
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}
