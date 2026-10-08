//! **Put what only an old export still has back into the database.**
//!
//! [`crate::compare`] finds them; this copies each one in, as the export wrote
//! it, and records two things about it:
//!
//! - **deleted**, as of the import. The database had already finished a walk
//!   of the chat and not seen it, which is the conclusion a sync draws about a
//!   message it stops seeing — so a chat that never finished a sync is refused,
//!   because there "missing" only means "not read yet".
//! - **recovered**, by merging at the `started` of a `runs` row whose mode is
//!   `import`. See [`tgx_archive::Store::recovered`] for why that is not a
//!   column.
//!
//! The bytes of any media are not brought in: Desktop's JSON has no file id
//! to key a blob on. The payload keeps its paths, so the export folder is
//! still where the file is.

use crate::compare::{self, Skipped, Verdict};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What happened to one chat.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Copied in, or with `dry_run`, would have been.
    Imported {
        messages: usize,
    },
    /// The database never finished a sync, so nothing missing can be told
    /// apart from not read yet.
    NotSynced,
    Failed(String),
}

#[derive(Debug)]
pub struct ChatImport {
    pub chat_id: i64,
    pub title: String,
    pub database: PathBuf,
    pub outcome: Outcome,
}

/// Import everything `verdicts` found missing, reading the messages back out
/// of `files` — the same `result.json` files the comparison read.
pub async fn import(
    verdicts: &[Verdict],
    files: &[PathBuf],
    dry_run: bool,
    now: i64,
) -> (Vec<ChatImport>, Vec<Skipped>) {
    let mut out = Vec::new();
    let mut wanted: HashMap<i64, HashSet<i64>> = HashMap::new();
    for v in verdicts {
        let Some(database) = &v.database else {
            continue;
        };
        if v.missing.is_empty() {
            continue;
        }
        let entry = |outcome| ChatImport {
            chat_id: v.chat_id,
            title: v.title.clone(),
            database: database.clone(),
            outcome,
        };
        if v.synced.is_none() {
            out.push(entry(Outcome::NotSynced));
        } else if dry_run {
            out.push(entry(Outcome::Imported {
                messages: v.missing.len(),
            }));
        } else {
            wanted.insert(v.chat_id, v.missing.iter().map(|m| m.id).collect());
        }
    }
    if wanted.is_empty() {
        return (out, Vec::new());
    }

    let (payloads, skipped) = {
        let files = files.to_vec();
        let wanted = wanted.clone();
        tokio::task::spawn_blocking(move || compare::payloads(&files, &wanted))
            .await
            .unwrap_or_default()
    };

    for v in verdicts {
        let Some(ids) = wanted.get(&v.chat_id) else {
            continue;
        };
        let Some(database) = &v.database else {
            continue;
        };
        let root = v.export_files.first().and_then(|f| f.parent());
        let mut ids: Vec<i64> = ids.iter().copied().collect();
        ids.sort_unstable();
        let outcome = match into_store(database, v.chat_id, &ids, &payloads, root, now).await {
            Ok(n) => Outcome::Imported { messages: n },
            Err(e) => Outcome::Failed(e.to_string()),
        };
        out.push(ChatImport {
            chat_id: v.chat_id,
            title: v.title.clone(),
            database: database.clone(),
            outcome,
        });
    }
    (out, skipped)
}

async fn into_store(
    database: &Path,
    chat_id: i64,
    ids: &[i64],
    payloads: &HashMap<(i64, i64), compare::Payload>,
    root: Option<&Path>,
    now: i64,
) -> Result<usize, tgx_archive::Error> {
    let store = tgx_archive::Store::open(database).await?;
    store.begin_batch().await?;
    let run = store.start_run(chat_id, "import", root, "{}", now).await?;
    let mut added = 0;
    for id in ids {
        let Some(p) = payloads.get(&(chat_id, *id)) else {
            continue;
        };
        // `merge` and not a bare insert: should the message have arrived since
        // the comparison, this is a version or a no-op rather than a clash.
        if store
            .merge(chat_id, *id, p.topic_id, &p.message, now)
            .await?
            == tgx_archive::Merge::Inserted
        {
            store.mark_deleted(chat_id, *id, now).await?;
            added += 1;
        }
    }
    // `reached_end` false: an import read no chat, and a sync is what that
    // column means to the window's "synced" caption.
    store.finish_run(run, added, 0, 0, 0, false, now).await?;
    store.commit_batch().await?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tgx-import-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn database(dir: &Path, synced: bool) -> PathBuf {
        let path = dir.join("db").join("Dev").join("Dev.sqlite");
        let store = tgx_archive::Store::open(&path).await.unwrap();
        store
            .upsert_chat(7, "Dev", "private_group", 1)
            .await
            .unwrap();
        let msg = serde_json::json!({"id": 2, "type": "message", "date": "d", "text": ""});
        store
            .merge(7, 2, None, msg.as_object().unwrap(), 1)
            .await
            .unwrap();
        let run = store.start_run(7, "database", None, "{}", 1).await.unwrap();
        store.finish_run(run, 1, 0, 0, 0, synced, 2).await.unwrap();
        path
    }

    fn export(dir: &Path) -> Vec<PathBuf> {
        let export = dir.join("export");
        std::fs::create_dir_all(&export).unwrap();
        std::fs::write(
            export.join("result.json"),
            r#"{"name":"Dev","type":"private_group","id":7,"messages":[
                {"id":1,"type":"message","date":"2024-05-01T00:00:00","text":"gone"},
                {"id":2,"type":"message","date":"2024-05-02T00:00:00","text":""}]}"#,
        )
        .unwrap();
        compare::find_results(&[export], 4)
    }

    async fn run(dir: &Path, files: &[PathBuf], dry_run: bool, now: i64) -> Vec<ChatImport> {
        let (chats, _) = compare::read_exports(files);
        let (verdicts, _) = compare::compare(chats, &dir.join("db")).await.unwrap();
        import(&verdicts, files, dry_run, now).await.0
    }

    #[tokio::test]
    async fn a_missing_message_comes_back_deleted_and_recovered_once() {
        let dir = tmp("once");
        let path = database(&dir, true).await;
        let files = export(&dir);

        let dry = run(&dir, &files, true, 50).await;
        assert_eq!(dry[0].outcome, Outcome::Imported { messages: 1 });
        let store = tgx_archive::Store::open(&path).await.unwrap();
        assert!(store.get(7, 1).await.unwrap().is_none(), "a dry run wrote");
        drop(store);

        let done = run(&dir, &files, false, 100).await;
        assert_eq!(done[0].outcome, Outcome::Imported { messages: 1 });
        let store = tgx_archive::Store::open(&path).await.unwrap();
        let m = store.get(7, 1).await.unwrap().expect("imported");
        assert_eq!(m.payload["text"], "gone");
        assert_eq!(m.deleted_seen, Some(100));
        assert_eq!(store.recovered(7).await.unwrap(), HashSet::from([1]));
        drop(store);

        // Nothing is missing any more, so a second import has nothing to do.
        assert!(run(&dir, &files, false, 200).await.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_database_that_never_finished_a_sync_is_refused() {
        let dir = tmp("unsynced");
        let path = database(&dir, false).await;
        let files = export(&dir);
        let done = run(&dir, &files, false, 100).await;
        assert_eq!(done[0].outcome, Outcome::NotSynced);
        let store = tgx_archive::Store::open(&path).await.unwrap();
        assert!(store.get(7, 1).await.unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
