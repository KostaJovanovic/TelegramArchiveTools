//! **One database per chat again, where renames made several.**
//!
//! Before the window looked a chat's database up by the id inside it, it
//! opened the file the chat's *current* title names, so a group renamed
//! between two syncs started a fresh database under every new name — a folder
//! a name, each holding the stretch of the chat it was synced under. This
//! folds every older file of a chat into the one a sync now writes to (the
//! one [`archived_chats`](crate::engine::archived_chats) picks: the last to
//! finish a sync), checks nothing was left behind, and only then retires the
//! older file: renamed to `<name>.sqlite.merged`, and its folder — reports and
//! summaries with it — moved into `<current folder>/merged/`. It never deletes.

use std::path::{Path, PathBuf};

/// One chat held in more than one file.
#[derive(Debug, Clone)]
pub struct Planned {
    pub chat_id: i64,
    /// The title the file being kept records.
    pub title: String,
    /// The file a sync writes to, which keeps what it says about the present.
    pub into: PathBuf,
    /// The others, the most recently synced first: the order they fold in,
    /// since each puts its history in front of what is already there.
    pub from: Vec<PathBuf>,
}

/// One older file after the fold.
#[derive(Debug)]
pub struct Done {
    pub from: PathBuf,
    pub result: Result<tgx_archive::Absorbed, String>,
    /// Where the file, or its folder, went once it had been folded in whole.
    pub moved_to: Option<PathBuf>,
}

/// Every chat in `dir` with more than one database. Writes nothing.
///
/// A file that will not open, or that holds several chats (the one-file
/// build's, which is `split`'s), is no part of any plan.
pub async fn plan(dir: &Path) -> std::io::Result<Vec<Planned>> {
    // Per chat, in the scan's order.
    let mut by_chat: Vec<(i64, Vec<(PathBuf, tgx_archive::ChatEntry)>)> = Vec::new();
    for path in crate::engine::database_files(dir)? {
        let Ok(chats) = tgx_archive::Store::chats_in(&path).await else {
            continue;
        };
        let Ok([chat]) = <[_; 1]>::try_from(chats) else {
            continue;
        };
        match by_chat.iter_mut().find(|(id, _)| *id == chat.id) {
            Some((_, files)) => files.push((path, chat)),
            None => by_chat.push((chat.id, vec![(path, chat)])),
        }
    }
    let mut out = Vec::new();
    for (chat_id, mut files) in by_chat {
        if files.len() < 2 {
            continue;
        }
        // `archived_chats`' rule: the last synced, and the first of a tie in
        // the scan's order. A stable sort keeps that order among equals.
        files.sort_by_key(|(_, chat)| std::cmp::Reverse(chat.synced));
        let (into, chat) = files.remove(0);
        out.push(Planned {
            chat_id,
            title: chat.title,
            into,
            from: files.into_iter().map(|(p, _)| p).collect(),
        });
    }
    Ok(out)
}

/// Do it, chat by chat. A file is retired only once it is folded in whole; a
/// chat whose fold fails stops there, its remaining files left as they were.
pub async fn merge(dir: &Path, planned: &Planned) -> Result<Vec<Done>, tgx_archive::Error> {
    let store = tgx_archive::Store::open(&planned.into).await?;
    let mut folded = Vec::new();
    for from in &planned.from {
        let result = store
            .absorb(from, planned.chat_id)
            .await
            .map_err(|e| e.to_string())
            .and_then(|a| match a.lacking {
                0 => Ok(a),
                n => Err(format!(
                    "{n} rows did not come across; the file is left as it was"
                )),
            });
        let failed = result.is_err();
        folded.push((from.clone(), result));
        if failed {
            break;
        }
    }
    // Closed before anything is moved: the folders being moved go *into* the
    // folder this file is in, and nothing should hold a file mid-move.
    drop(store);

    let shelf = planned.into.parent().unwrap_or(dir).join("merged");
    let mut out = Vec::new();
    for (from, result) in folded {
        let moved_to = match &result {
            Ok(_) => Some(retire(dir, &from, &shelf)?),
            Err(_) => None,
        };
        out.push(Done {
            from,
            result,
            moved_to,
        });
    }
    Ok(out)
}

/// Rename a folded file to `.merged`, its `-wal` and `-shm` with it, and move
/// it onto the shelf: the whole folder when it is one of ours,
/// `<dir>/<name>/<name>.sqlite`, else the file alone.
fn retire(dir: &Path, file: &Path, shelf: &Path) -> std::io::Result<PathBuf> {
    // `-wal` after the name, as SQLite looks for it: `x.sqlite.merged-wal`
    // belongs to `x.sqlite.merged`, so the pair can never be split up.
    for suffix in ["", "-wal", "-shm"] {
        let from = PathBuf::from(format!("{}{suffix}", file.display()));
        if from.exists() {
            std::fs::rename(&from, format!("{}.merged{suffix}", file.display()))?;
        }
    }
    std::fs::create_dir_all(shelf)?;
    let folder = file.parent().filter(|p| {
        p.parent() == Some(dir) && p.file_name().map(Path::new) == file.file_stem().map(Path::new)
    });
    match folder {
        Some(folder) => {
            let name = folder.file_name().unwrap_or_default().to_string_lossy();
            let to = free(shelf, &name);
            std::fs::rename(folder, &to)?;
            Ok(to)
        }
        None => {
            let name = file.file_name().unwrap_or_default().to_string_lossy();
            let to = free(shelf, &format!("{name}.merged"));
            for suffix in ["", "-wal", "-shm"] {
                let from = PathBuf::from(format!("{}.merged{suffix}", file.display()));
                if from.exists() {
                    std::fs::rename(&from, format!("{}{suffix}", to.display()))?;
                }
            }
            Ok(to)
        }
    }
}

/// `name` on the shelf, or `name (2)`, `(3)`… if a folder there has it.
fn free(shelf: &Path, name: &str) -> PathBuf {
    let mut to = shelf.join(name);
    let mut n = 2;
    while to.exists() {
        to = shelf.join(format!("{name} ({n})"));
        n += 1;
    }
    to
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tgx-merge-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A database for chat `chat` under `name`, holding `ids`, synced at
    /// `synced` if that is given.
    async fn file(dir: &Path, name: &str, chat: i64, ids: &[i64], synced: Option<i64>) {
        let path = crate::engine::archive_path(dir, name).unwrap();
        let s = tgx_archive::Store::open(&path).await.unwrap();
        let at = synced.unwrap_or(1);
        s.upsert_chat(chat, name, "private_supergroup", at)
            .await
            .unwrap();
        let run = s.start_run(chat, "full", None, "{}", at).await.unwrap();
        for &id in ids {
            // Each file's text an edit Telegram dates at that file's sync.
            let m = json!({"id": id, "type": "message", "date": "d", "text": name,
                           "edited": "e", "edited_unixtime": at.to_string()});
            s.merge(chat, id, None, m.as_object().unwrap(), at)
                .await
                .unwrap();
        }
        if synced.is_some() {
            s.finish_run(run, ids.len(), 0, 0, 0, true, at)
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn every_name_a_chat_was_synced_under_folds_into_the_last() {
        let dir = tmp("renamed");
        // One group under three names, and a chat that was never renamed.
        file(&dir, "mlin mleje", 7, &[1, 2], Some(100)).await;
        file(&dir, "izlapeli vinjak", 7, &[2, 3], Some(200)).await;
        file(&dir, "studenti", 7, &[3, 4], Some(300)).await;
        file(&dir, "Mum", 8, &[1], Some(100)).await;
        // Something of the old folder's own, which must go where it goes.
        std::fs::write(dir.join("mlin mleje").join("report.html"), "r").unwrap();

        let planned = plan(&dir).await.unwrap();
        assert_eq!(planned.len(), 1, "{planned:?}");
        let p = &planned[0];
        assert_eq!(p.chat_id, 7);
        assert_eq!(p.into, dir.join("studenti").join("studenti.sqlite"));
        assert_eq!(
            p.from,
            vec![
                dir.join("izlapeli vinjak").join("izlapeli vinjak.sqlite"),
                dir.join("mlin mleje").join("mlin mleje.sqlite"),
            ],
            "newest first"
        );

        let done = merge(&dir, p).await.unwrap();
        assert!(done.iter().all(|d| d.result.is_ok()), "{done:?}");

        let store = tgx_archive::Store::open(&p.into).await.unwrap();
        assert_eq!(store.message_count(7).await.unwrap().0, 4);
        // #2 as each file last saw it: the oldest text first.
        let two: Vec<String> = store
            .versions_of(7, 2)
            .await
            .unwrap()
            .into_iter()
            .map(|v| v.payload["text"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(two, vec!["mlin mleje"]);
        drop(store);

        // Retired, not deleted; and a sync finds one file for the chat.
        let shelf = dir.join("studenti").join("merged");
        assert!(shelf
            .join("mlin mleje")
            .join("mlin mleje.sqlite.merged")
            .is_file());
        assert!(shelf.join("mlin mleje").join("report.html").is_file());
        assert!(shelf.join("izlapeli vinjak").is_dir());
        assert!(!dir.join("mlin mleje").exists());
        let found = crate::engine::archived_chats(&dir).await.unwrap();
        assert_eq!(found.files[&7], p.into);
        assert!(plan(&dir).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
