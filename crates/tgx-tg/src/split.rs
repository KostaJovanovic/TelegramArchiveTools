//! **Split a database holding several chats into one file per chat.**
//!
//! Builds before one-database-per-chat wrote every chat into one
//! `telegram.sqlite`. No run writes into that file any more, so its history —
//! the earlier versions and the deletions it alone saw — sits beside the
//! per-chat files without reaching them. This copies each chat into the file a
//! sync of it would open, merging into one that already exists (what that file
//! has wins), checks nothing was left behind, and only then renames the
//! original to `<name>.sqlite.split`. It never deletes it.

use std::path::{Path, PathBuf};

/// Where one chat goes.
#[derive(Debug, Clone)]
pub struct Planned {
    pub chat_id: i64,
    pub title: String,
    pub dest: PathBuf,
    /// The destination already exists, so this is a merge into it.
    pub merge: bool,
}

/// One chat after the copy: how many of its messages the destination still
/// lacks, which is 0 unless something went wrong.
#[derive(Debug)]
pub struct Done {
    pub planned: Planned,
    pub lacking: Result<i64, String>,
}

/// Where each chat in `file` would go, under `into`. Writes nothing.
///
/// The name is the one [`crate::engine`] opens for the chat — its sanitised
/// title, or `<title> (<id>)` when that file belongs to somebody else — so the
/// next sync finds the history this put there.
///
/// **A name an earlier chat of this same plan took is taken too**, though no
/// file holds it yet. The test on disk alone sent "Admini" and "admini" — one
/// folder on Windows — or two chats of one title into the same destination,
/// and `split` then copied both conversations into one database: the mix-up a
/// file per chat exists to prevent. A sync applies the same rule at open time,
/// so it finds the second chat under `<title> (<id>)` exactly as this left it.
pub async fn plan(file: &Path, into: &Path) -> Result<Vec<Planned>, tgx_archive::Error> {
    let ext = tgx_archive::FILE_EXT;
    let mut out = Vec::new();
    // Lower-cased, because the filesystem this runs on compares names that
    // way. On a case-sensitive one it costs a suffix nobody needed.
    let mut planned = std::collections::HashSet::new();
    for chat in tgx_archive::Store::chats_in(file).await? {
        let base = tgx_media::topics::sanitize_component(&chat.title, "chat");
        let mut name = base.clone();
        let own = |name: &str| into.join(name).join(format!("{name}.{ext}"));
        let taken_by_other = planned.contains(&name.to_lowercase())
            || match tgx_archive::Store::chats_in(&own(&name)).await {
                Ok(chats) => chats.iter().any(|c| c.id != chat.id),
                Err(_) => own(&name).exists(),
            };
        // A chat named after the file being split would have it moved into
        // its folder as its own flat database. **Compared as files, not as
        // strings**: chat 777000 is "Telegram", and on Windows
        // `Telegram.sqlite` *is* `telegram.sqlite`.
        let is_source = same_file(&into.join(format!("{name}.{ext}")), file);
        if taken_by_other || is_source {
            name = format!("{base} ({})", chat.id);
        }
        planned.insert(name.to_lowercase());
        let dest = own(&name);
        out.push(Planned {
            chat_id: chat.id,
            title: chat.title,
            merge: dest.exists(),
            dest,
        });
    }
    Ok(out)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Do it. The original is renamed only if every chat came across whole.
pub async fn split(file: &Path, into: &Path) -> Result<Vec<Done>, tgx_archive::Error> {
    let mut out = Vec::new();
    for planned in plan(file, into).await? {
        let lacking = async {
            let name = planned
                .dest
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Moves a flat file of the same name into its folder first, as a
            // sync would.
            let dest = crate::engine::archive_path(into, &name)?;
            let store = tgx_archive::Store::open(&dest).await?;
            store.split_from(file, planned.chat_id).await
        }
        .await
        .map_err(|e: tgx_archive::Error| e.to_string());
        out.push(Done { planned, lacking });
    }
    if out.iter().all(|d| d.lacking == Ok(0)) {
        for suffix in ["", "-wal", "-shm"] {
            let from = PathBuf::from(format!("{}{suffix}", file.display()));
            if from.exists() {
                std::fs::rename(&from, format!("{}.split{suffix}", file.display()))?;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tgx-split-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn each_chat_gets_its_own_file_with_only_its_own_blobs() {
        let dir = tmp("two");
        let old = dir.join("telegram.sqlite");
        {
            let store = tgx_archive::Store::open(&old).await.unwrap();
            // "Telegram" is the service-notifications chat, and on a
            // case-blind filesystem its own flat file would be this one.
            for (chat, title) in [(1, "Dev"), (2, "Mum"), (777000, "Telegram")] {
                store
                    .upsert_chat(chat, title, "private_group", 1)
                    .await
                    .unwrap();
                for id in 1..=3 {
                    let m = json!({"id": id, "type": "message", "date": "d", "text": ""});
                    store
                        .merge(chat, id, None, m.as_object().unwrap(), 1)
                        .await
                        .unwrap();
                }
                store
                    .put_blob(chat * 10, "photos", "image/jpeg", None, b"px", 1)
                    .await
                    .unwrap();
                store
                    .link_media(chat, 1, "photo", chat * 10, "photos", "photos/p.jpg")
                    .await
                    .unwrap();
                let run = store
                    .start_run(chat, "database", None, "{}", 1)
                    .await
                    .unwrap();
                store.finish_run(run, 3, 0, 0, 0, true, 2).await.unwrap();
            }
        }
        // Dev already has a newer database, holding a message the old file
        // also has, in a different version.
        let dev = dir.join("Dev").join("Dev.sqlite");
        {
            let store = tgx_archive::Store::open(&dev).await.unwrap();
            store
                .upsert_chat(1, "Dev", "private_group", 5)
                .await
                .unwrap();
            let m = json!({"id": 3, "type": "message", "date": "d", "text": "newer"});
            store
                .merge(1, 3, None, m.as_object().unwrap(), 5)
                .await
                .unwrap();
        }

        let planned = plan(&old, &dir).await.unwrap();
        assert_eq!(planned.len(), 3);
        if cfg!(windows) {
            let tg = planned.iter().find(|p| p.chat_id == 777000).unwrap();
            assert!(
                tg.dest
                    .ends_with("Telegram (777000)/Telegram (777000).sqlite"),
                "{:?}",
                tg.dest
            );
        }
        assert!(planned.iter().find(|p| p.chat_id == 1).unwrap().merge);
        assert!(!planned.iter().find(|p| p.chat_id == 2).unwrap().merge);

        let done = split(&old, &dir).await.unwrap();
        assert!(done.iter().all(|d| d.lacking == Ok(0)), "{done:?}");
        assert!(!old.exists(), "renamed once whole");
        assert!(dir.join("telegram.sqlite.split").is_file());

        let store = tgx_archive::Store::open(&dev).await.unwrap();
        assert_eq!(store.message_count(1).await.unwrap().0, 3);
        assert_eq!(
            store.get(1, 3).await.unwrap().unwrap().payload["text"],
            "newer",
            "what the newer file has wins"
        );
        assert!(store.has_blob(10, "photos").await.unwrap());
        assert!(!store.has_blob(20, "photos").await.unwrap(), "Mum's blob");
        let chats = tgx_archive::Store::chats_in(&dev).await.unwrap();
        assert_eq!(chats.len(), 1);
        assert!(chats[0].synced.is_some(), "the old file's runs came along");

        let mum = dir.join("Mum").join("Mum.sqlite");
        let chats = tgx_archive::Store::chats_in(&mum).await.unwrap();
        assert_eq!(chats.iter().map(|c| c.id).collect::<Vec<_>>(), vec![2]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn two_chats_of_one_name_get_a_file_each() {
        let dir = tmp("same-name");
        let old = dir.join("telegram.sqlite");
        {
            let store = tgx_archive::Store::open(&old).await.unwrap();
            // One folder on Windows, and an identical title is enough anywhere.
            for (chat, title) in [(1, "Admini"), (2, "admini"), (3, "Admini")] {
                store
                    .upsert_chat(chat, title, "private_group", 1)
                    .await
                    .unwrap();
                let m = json!({"id": 1, "type": "message", "date": "d", "text": ""});
                store
                    .merge(chat, 1, None, m.as_object().unwrap(), 1)
                    .await
                    .unwrap();
            }
        }
        let planned = plan(&old, &dir).await.unwrap();
        let names: std::collections::HashSet<String> = planned
            .iter()
            .map(|p| p.dest.to_string_lossy().to_lowercase())
            .collect();
        assert_eq!(names.len(), 3, "{planned:?}");

        let done = split(&old, &dir).await.unwrap();
        assert!(done.iter().all(|d| d.lacking == Ok(0)), "{done:?}");
        for p in &planned {
            let chats = tgx_archive::Store::chats_in(&p.dest).await.unwrap();
            assert_eq!(
                chats.iter().map(|c| c.id).collect::<Vec<_>>(),
                vec![p.chat_id],
                "{:?} holds another chat",
                p.dest
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
