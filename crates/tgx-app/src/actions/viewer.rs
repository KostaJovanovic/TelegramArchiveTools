//! What the Viewer's controls do. **No connection** — every one of these reads
//! a database in the destination, read-only, and nothing else.
//!
//! The answers carry message text, for the window. **None of them is written
//! to the log**, and the failures they report name files and errors, never a
//! message.

use crate::bridge::{Event, Events};
use std::path::PathBuf;
use tgx_tg::viewer::{self, Media};

/// How many hits a search keeps per chat. Enough to page through on screen;
/// past it the answer is "search for something narrower".
pub const SEARCH_LIMIT: usize = 500;

/// The databases in the destination, for the picker.
pub async fn databases(output_dir: String, tx: Events) {
    let found = viewer::databases(std::path::Path::new(&output_dir))
        .await
        .map(|(dbs, _unreadable)| dbs)
        .map_err(|e| format!("could not read {output_dir}: {e}"));
    let _ = tx.send(Event::ViewerDatabases { output_dir, found });
}

/// Every message of one chat, flattened.
pub async fn load(path: PathBuf, chat_id: i64, tx: Events) {
    let rows = viewer::load(&path, chat_id)
        .await
        .map_err(|e| format!("could not read {}: {e}", path.display()));
    let _ = tx.send(Event::ViewerLoaded {
        path,
        chat_id,
        rows,
    });
}

/// One message in full.
pub async fn detail(path: PathBuf, chat_id: i64, id: i64, tx: Events) {
    let detail = viewer::detail(&path, chat_id, id)
        .await
        .map(|d| d.map(Box::new))
        .map_err(|e| e.to_string());
    let _ = tx.send(Event::ViewerDetail {
        chat_id,
        id,
        detail,
    });
}

/// Search one chat — `scope` is its database, its id and its title — or,
/// with no scope, every database in `output_dir`.
pub async fn search(
    output_dir: String,
    scope: Option<(PathBuf, i64, String)>,
    needle: String,
    tx: Events,
) {
    let found = match &scope {
        Some((path, chat_id, title)) => tgx_tg::search::chat(path, *chat_id, &needle, SEARCH_LIMIT)
            .await
            .map(|hits| {
                let mut found = tgx_tg::search::Found::default();
                if !hits.is_empty() {
                    found.chats.push(tgx_tg::search::ChatHits {
                        chat_id: *chat_id,
                        title: title.clone(),
                        path: path.clone(),
                        hits,
                    });
                }
                Box::new(found)
            })
            .map_err(|e| e.to_string()),
        None => tgx_tg::search::across(std::path::Path::new(&output_dir), &needle, SEARCH_LIMIT)
            .await
            .map(Box::new)
            .map_err(|e| format!("could not read {output_dir}: {e}")),
    };
    let _ = tx.send(Event::SearchHits {
        needle,
        scope: scope.map(|(_, id, _)| id),
        found,
    });
}

/// Copy one stored file out to `%TEMP%\tgx-view` and hand it to the system.
///
/// The status line says which file opened; a failure is a warning, naming
/// the file and the reason — a file name is not message text.
pub async fn open_media(path: PathBuf, media: Media, tx: Events) {
    let opened = match viewer::extract(&path, &media).await {
        Ok(file) => super::open_path(&file),
        Err(e) => Err(e),
    };
    let _ = tx.send(match opened {
        Ok(()) => Event::Status(format!("Opened {}", media.name)),
        Err(e) => Event::Warn(format!("Could not open {}: {e}", media.name)),
    });
}
