//! **Search every database in a folder** for a piece of text.
//!
//! Offline: it reads the `.sqlite` files [`crate::engine::archived_chats`]
//! finds, read-only, and nothing else. The matching itself is
//! [`tgx_archive::Store::search`] — current text, earlier versions and deleted
//! messages, with the case folded in Rust because SQLite folds ASCII only.
//!
//! **What this returns carries the matched text, and it must not reach the
//! log.** The CLI prints it to the terminal of the person who asked, and the
//! viewer shows it in the window; neither is the file beside the executable
//! that gets attached to a bug report.

use std::path::{Path, PathBuf};
/// Re-exported because `tgx-app` may not depend on `tgx-archive`: the window
/// reaches the store's types through this crate or not at all.
pub use tgx_archive::Hit;

/// One chat's hits.
#[derive(Debug, Clone)]
pub struct ChatHits {
    pub chat_id: i64,
    pub title: String,
    /// The database they are in — what the viewer opens on a click.
    pub path: PathBuf,
    pub hits: Vec<Hit>,
}

/// What [`across`] found.
#[derive(Debug, Clone, Default)]
pub struct Found {
    /// Chats with at least one hit, in the order the folder lists them.
    pub chats: Vec<ChatHits>,
    /// The databases that could not be searched, and why.
    pub unreadable: Vec<(PathBuf, String)>,
}

impl Found {
    pub fn total(&self) -> usize {
        self.chats.iter().map(|c| c.hits.len()).sum()
    }
}

/// Search one chat's database. `limit` caps the hits.
pub async fn chat(
    path: &Path,
    chat_id: i64,
    needle: &str,
    limit: usize,
) -> Result<Vec<Hit>, tgx_archive::Error> {
    let (path, needle) = (path.to_path_buf(), needle.to_string());
    off_the_runtime(async move {
        let store = tgx_archive::Store::open_read(&path).await?;
        store.search(chat_id, &needle, limit).await
    })
    .await
}

/// Search every chat with a database in `dir`. `limit` caps the hits **per
/// chat**, so one chat full of the word cannot crowd every other out of the
/// answer.
pub async fn across(dir: &Path, needle: &str, limit: usize) -> std::io::Result<Found> {
    let archived = crate::engine::archived_chats(dir).await?;
    let mut found = Found {
        unreadable: archived.unreadable,
        ..Default::default()
    };
    for (chat_id, title) in archived.chats {
        let Some(path) = archived.files.get(&chat_id).cloned() else {
            continue;
        };
        match chat(&path, chat_id, needle, limit).await {
            Ok(hits) if hits.is_empty() => {}
            Ok(hits) => found.chats.push(ChatHits {
                chat_id,
                title,
                path,
                hits,
            }),
            Err(e) => found.unreadable.push((path, e.to_string())),
        }
    }
    Ok(found)
}

/// Run a store's work on tokio's blocking pool rather than on a worker.
///
/// **The store's methods are `async` because libsql's API is, and they block
/// underneath** — a local SQLite file is read on the calling thread. A search
/// scans every message of a chat and a viewer load parses every payload, and
/// done inline either would sit on one of the window's two worker threads for
/// as long as it takes, with the other one carrying an export. The blocking
/// pool exists for exactly this.
///
/// A panic inside is carried back out rather than turned into an error: it is
/// a bug, and the caller's task is the right place for it to surface.
pub(crate) async fn off_the_runtime<F, T>(work: F) -> T
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handle = tokio::runtime::Handle::current();
    match tokio::task::spawn_blocking(move || handle.block_on(work)).await {
        Ok(out) => out,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tgx-search-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn database(dir: &Path, chat: i64, title: &str, texts: &[&str]) {
        let path = dir.join(title).join(format!("{title}.sqlite"));
        let store = tgx_archive::Store::open(&path).await.unwrap();
        store
            .upsert_chat(chat, title, "private_group", 1)
            .await
            .unwrap();
        for (i, text) in texts.iter().enumerate() {
            let id = i as i64 + 1;
            let m = json!({"id": id, "type": "message", "date": format!("2025-01-0{id}T10:00:00"),
                           "date_unixtime": format!("{}", 1_735_725_600 + id), "text": text});
            store
                .merge(chat, id, None, m.as_object().unwrap(), 1)
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn hits_come_back_grouped_by_the_chat_they_are_in() {
        let dir = tmp("across");
        database(&dir, 1, "Dev", &["Ђорђе kasni", "ništa"]).await;
        database(&dir, 2, "Mum", &["zdravo", "javi ђорђу", "ђорђе je stigao"]).await;
        database(&dir, 3, "Quiet", &["ništa ovde"]).await;

        let found = across(&dir, "ЂОРЂЕ", 50).await.unwrap();
        assert!(found.unreadable.is_empty(), "{:?}", found.unreadable);
        let by: Vec<(&str, Vec<i64>)> = found
            .chats
            .iter()
            .map(|c| (c.title.as_str(), c.hits.iter().map(|h| h.id).collect()))
            .collect();
        assert_eq!(by, vec![("Dev", vec![1]), ("Mum", vec![3])]);
        assert_eq!(found.total(), 2);
        assert!(found.chats[0].path.ends_with("Dev/Dev.sqlite"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
