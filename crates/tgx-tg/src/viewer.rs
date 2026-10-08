//! **What the window's Viewer reads out of a database**, already flattened.
//!
//! The viewer paints a list of every message in a chat — tens of thousands of
//! rows — from inside a frame, sixty times a second. Parsing a JSON payload
//! there would be the whole chat's worth of `serde_json` per frame, so the
//! payload is read **once, here, on the blocking pool**, and the window gets
//! [`Row`]s: a date, a sender, a first line, the flags. Nothing a frame touches
//! is JSON.
//!
//! The same goes for the layering: `tgx-app` may not depend on `tgx-archive`,
//! so everything it needs from a database is a plain struct defined here.
//!
//! **Read-only throughout** — [`tgx_archive::Store::open_read`]. Looking at a
//! database must never be the second writer to one an export has open, and a
//! viewer pointed at a file must leave it as it found it.
//!
//! The rows carry message text, for the screen. **None of it goes to the log.**

use crate::search::off_the_runtime;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// How much of a message's first line a row keeps. The row clips to its width
/// anyway; this bounds what a hundred thousand rows hold in memory.
const PREVIEW_CHARS: usize = 200;

/// One database the viewer can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Database {
    pub chat_id: i64,
    pub title: String,
    pub path: PathBuf,
}

/// The databases in `dir`, by title, and the files that could not be read.
pub async fn databases(dir: &Path) -> std::io::Result<(Vec<Database>, Vec<(PathBuf, String)>)> {
    let found = crate::engine::archived_chats(dir).await?;
    let mut out: Vec<Database> = found
        .chats
        .into_iter()
        .filter_map(|(chat_id, title)| {
            let path = found.files.get(&chat_id)?.clone();
            Some(Database {
                chat_id,
                title,
                path,
            })
        })
        .collect();
    out.sort_by_key(|d| d.title.to_lowercase());
    Ok((out, found.unreadable))
}

/// One message as the viewer's list paints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: i64,
    pub topic_id: i64,
    /// `2025-12-18 16:00`, from the payload's own local `date`.
    pub date: String,
    /// The sender, or for a service message the actor. Empty when the payload
    /// names neither.
    pub from: String,
    /// The first non-empty line of the text, clipped. For a service message,
    /// its action in brackets.
    pub preview: String,
    /// `[photo]`, `[voice message]`, `[report.pdf]` — what the message
    /// carried, from the payload, whether or not the bytes are stored.
    pub media: Option<String>,
    /// When a run first found it gone from Telegram, formatted. `None` while
    /// Telegram still has it.
    pub deleted: Option<String>,
    /// The payload's version: 1 for a message never seen to change. Every
    /// change counts — an edit, but also reactions or counters moving.
    pub version: i64,
    /// Put back from an old export by `tgx import` rather than read from
    /// Telegram. See [`tgx_archive::Store::recovered`].
    pub recovered: bool,
    pub service: bool,
}

impl Row {
    /// The payload has changed since it was first stored.
    pub fn edited(&self) -> bool {
        self.version > 1
    }
}

/// Every message of `chat_id` in the database at `path`, oldest first.
pub async fn load(path: &Path, chat_id: i64) -> Result<Vec<Row>, tgx_archive::Error> {
    let path = path.to_path_buf();
    off_the_runtime(async move {
        let store = tgx_archive::Store::open_read(&path).await?;
        let recovered = store.recovered(chat_id).await?;
        let messages = store.all_messages(chat_id).await?;
        Ok(messages
            .into_iter()
            .map(|m| Row {
                id: m.id,
                topic_id: m.topic_id,
                date: local_date(&m.payload),
                from: sender(&m.payload),
                preview: preview(&m.payload),
                media: media_label(&m.payload),
                deleted: m.deleted_seen.map(unix_date),
                version: m.version,
                recovered: recovered.contains(&m.id),
                service: is_service(&m.payload),
            })
            .collect())
    })
    .await
}

/// One superseded version, for the detail pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub version: i64,
    /// When a run found it replaced, formatted.
    pub replaced: String,
    pub text: String,
    /// The payload's own `edited` stamp, when Telegram gave one.
    pub edited: Option<String>,
}

/// One file of a message, for the detail pane's Open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub role: String,
    pub file_id: i64,
    pub kind: String,
    /// What to call the file on disk: Telegram's name for it, else the last
    /// part of the name the folder export used.
    pub name: String,
    /// `None` when the bytes are not in the database, so there is nothing to
    /// open.
    pub size: Option<i64>,
}

/// Everything the detail pane shows of one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub id: i64,
    pub date: String,
    pub from: String,
    /// The whole text, every line.
    pub text: String,
    pub reply_to: Option<i64>,
    pub edited: Option<String>,
    pub deleted: Option<String>,
    pub recovered: bool,
    /// A service message's action, `pin_message` and the like.
    pub action: Option<String>,
    pub versions: Vec<Version>,
    pub media: Vec<Media>,
}

/// One message in full, or `None` if the database does not hold it.
pub async fn detail(
    path: &Path,
    chat_id: i64,
    id: i64,
) -> Result<Option<Detail>, tgx_archive::Error> {
    let path = path.to_path_buf();
    off_the_runtime(async move {
        let store = tgx_archive::Store::open_read(&path).await?;
        let Some(m) = store.get(chat_id, id).await? else {
            return Ok(None);
        };
        let recovered = store.recovered(chat_id).await?.contains(&id);
        let versions = versions_in(&store, chat_id, id).await?;
        let media = media_in(&store, chat_id, id).await?;
        let p = &m.payload;
        Ok(Some(Detail {
            id,
            date: local_date(p),
            from: sender(p),
            text: text_of(p),
            reply_to: p.get("reply_to_message_id").and_then(Value::as_i64),
            edited: p.get("edited").and_then(Value::as_str).map(tidy_date),
            deleted: m.deleted_seen.map(unix_date),
            recovered,
            action: is_service(p)
                .then(|| p.get("action").and_then(Value::as_str).map(str::to_string))
                .flatten(),
            versions,
            media,
        }))
    })
    .await
}

/// Every superseded version of one message, oldest first.
pub async fn versions(
    path: &Path,
    chat_id: i64,
    id: i64,
) -> Result<Vec<Version>, tgx_archive::Error> {
    let path = path.to_path_buf();
    off_the_runtime(async move {
        let store = tgx_archive::Store::open_read(&path).await?;
        versions_in(&store, chat_id, id).await
    })
    .await
}

/// The files one message points at.
pub async fn media(path: &Path, chat_id: i64, id: i64) -> Result<Vec<Media>, tgx_archive::Error> {
    let path = path.to_path_buf();
    off_the_runtime(async move {
        let store = tgx_archive::Store::open_read(&path).await?;
        media_in(&store, chat_id, id).await
    })
    .await
}

async fn versions_in(
    store: &tgx_archive::Store,
    chat_id: i64,
    id: i64,
) -> Result<Vec<Version>, tgx_archive::Error> {
    Ok(store
        .versions_of(chat_id, id)
        .await?
        .into_iter()
        .map(|v| Version {
            version: v.version,
            replaced: unix_date(v.seen),
            text: text_of(&v.payload),
            edited: v
                .payload
                .get("edited")
                .and_then(Value::as_str)
                .map(tidy_date),
        })
        .collect())
}

async fn media_in(
    store: &tgx_archive::Store,
    chat_id: i64,
    id: i64,
) -> Result<Vec<Media>, tgx_archive::Error> {
    Ok(store
        .media_of(chat_id, id)
        .await?
        .into_iter()
        .map(|m| Media {
            name: safe_name(m.file_name.as_deref().unwrap_or(&m.path), m.file_id),
            role: m.role,
            file_id: m.file_id,
            kind: m.kind,
            size: m.size,
        })
        .collect())
}

/// Where Open media puts a file for the operating system to open:
/// `%TEMP%\tgx-view`.
///
/// **Emptied when the window opens and again when it closes** — see
/// [`clear_temp`]. These are other people's photos and documents, copied out
/// of a database the user chose to keep somewhere in particular, into a folder
/// every program on the machine can read.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("tgx-view")
}

/// Remove everything [`extract`] wrote. Quiet: a file still open in another
/// program cannot be removed on Windows, and that is no reason to complain.
pub fn clear_temp() {
    let _ = std::fs::remove_dir_all(temp_dir());
}

/// Write one stored file to `temp_dir()/<file_id>/<name>` and return where.
///
/// A folder per file, so two attachments both called `image.jpg` do not
/// overwrite each other — and so the file keeps its own name, which is what
/// the program it opens in will show.
pub async fn extract(path: &Path, media: &Media) -> Result<PathBuf, String> {
    let (path, file_id, kind, name) = (
        path.to_path_buf(),
        media.file_id,
        media.kind.clone(),
        safe_name(&media.name, media.file_id),
    );
    off_the_runtime(async move {
        let store = tgx_archive::Store::open_read(&path)
            .await
            .map_err(|e| e.to_string())?;
        let bytes = store
            .blob(file_id, &kind)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "the database holds no bytes for this file".to_string())?;
        let dir = temp_dir().join(file_id.to_string());
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let out = dir.join(name);
        std::fs::write(&out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
        Ok(out)
    })
    .await
}

/// The last component of `name`, and only that.
///
/// The name comes out of the database, and a database is a file somebody may
/// have handed over: `..\..\Startup\x.bat` must land inside the temp folder
/// as `x.bat`, not two levels above it.
fn safe_name(name: &str, file_id: i64) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let cleaned: String = last
        .chars()
        .map(|c| match c {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        format!("file_{file_id}")
    } else {
        cleaned
    }
}

// --- flattening ---------------------------------------------------------------

fn is_service(p: &Map<String, Value>) -> bool {
    p.get("type").and_then(Value::as_str) == Some("service")
}

fn sender(p: &Map<String, Value>) -> String {
    p.get("from")
        .or_else(|| p.get("actor"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn text_of(p: &Map<String, Value>) -> String {
    p.get("text")
        .map(tgx_archive::plain_text)
        .unwrap_or_default()
}

fn preview(p: &Map<String, Value>) -> String {
    if is_service(p) {
        let action = p.get("action").and_then(Value::as_str).unwrap_or("service");
        return format!("[{}]", action.replace('_', " "));
    }
    let text = text_of(p);
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    match line {
        Some(l) if l.chars().count() > PREVIEW_CHARS => {
            let cut: String = l.chars().take(PREVIEW_CHARS).collect();
            format!("{cut}…")
        }
        Some(l) => l.to_string(),
        None => String::new(),
    }
}

/// What a message carried, as a bracketed label.
///
/// Read off the payload's keys — the ones Desktop writes — rather than the
/// `media` table, so a message whose file was too large to store still says
/// it had one.
fn media_label(p: &Map<String, Value>) -> Option<String> {
    if p.contains_key("photo") {
        return Some("[photo]".into());
    }
    if p.contains_key("file") {
        if let Some(kind) = p.get("media_type").and_then(Value::as_str) {
            return Some(format!("[{}]", kind.replace('_', " ")));
        }
        let name = p
            .get("file_name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .unwrap_or("file");
        return Some(format!("[{name}]"));
    }
    for (key, label) in [
        ("poll", "[poll]"),
        ("location_information", "[location]"),
        ("contact_information", "[contact]"),
    ] {
        if p.contains_key(key) {
            return Some(label.into());
        }
    }
    None
}

/// Desktop's `2025-12-18T16:00:00` as `2025-12-18 16:00`.
fn tidy_date(s: &str) -> String {
    let s = s.replacen('T', " ", 1);
    match s.len() {
        19 => s[..16].to_string(),
        _ => s,
    }
}

fn local_date(p: &Map<String, Value>) -> String {
    p.get("date")
        .and_then(Value::as_str)
        .map(tidy_date)
        .unwrap_or_default()
}

/// A unix second as local `2025-12-18 16:00`.
fn unix_date(at: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(at, 0).single() {
        Some(t) => t.format("%Y-%m-%d %H:%M").to_string(),
        None => at.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tgx-viewer-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("Dev.sqlite")
    }

    fn msg(id: i64, text: Value) -> Map<String, Value> {
        json!({
            "id": id, "type": "message",
            "date": format!("2025-12-1{id}T16:0{id}:00"),
            "date_unixtime": format!("{}", 1_766_000_000 + id),
            "from": "Ivana", "from_id": "user1",
            "text": text,
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[tokio::test]
    async fn load_flattens_a_deleted_an_edited_and_a_recovered_message() {
        let path = tmp("load");
        {
            let s = tgx_archive::Store::open(&path).await.unwrap();
            s.upsert_chat(7, "Dev", "private_group", 1).await.unwrap();
            // Plain, then deleted.
            s.merge(7, 1, None, &msg(1, json!("ovo će nestati")), 100)
                .await
                .unwrap();
            s.mark_deleted(7, 1, 1_766_100_000).await.unwrap();
            // Edited: two versions, the first kept, text given as entities.
            s.merge(7, 2, None, &msg(2, json!("prva")), 100)
                .await
                .unwrap();
            let mut edited = msg(2, json!([{"type": "bold", "text": "Druga"}, "\nlinija"]));
            edited.insert("edited".into(), json!("2025-12-12T17:00:00"));
            edited.insert("photo".into(), json!("photos/photo_1.jpg"));
            s.merge(7, 2, None, &edited, 200).await.unwrap();
            // Recovered: merged at an import run's start, as `tgx import` does.
            let run = s.start_run(7, "import", None, "{}", 300).await.unwrap();
            let mut svc = msg(3, json!(""));
            svc.insert("type".into(), json!("service"));
            svc.remove("from");
            svc.insert("actor".into(), json!("Marko"));
            svc.insert("action".into(), json!("pin_message"));
            s.merge(7, 3, None, &svc, 300).await.unwrap();
            s.finish_run(run, 1, 0, 0, 0, true, 301).await.unwrap();
        }

        let rows = load(&path, 7).await.unwrap();
        assert_eq!(rows.len(), 3);

        let gone = &rows[0];
        assert_eq!(gone.id, 1);
        assert_eq!(gone.date, "2025-12-11 16:01");
        assert_eq!(gone.from, "Ivana");
        assert_eq!(gone.preview, "ovo će nestati");
        assert!(gone.deleted.is_some());
        assert!(!gone.edited() && !gone.recovered);

        let edited = &rows[1];
        assert!(edited.edited());
        assert_eq!(edited.version, 2);
        assert_eq!(
            edited.preview, "Druga",
            "the first line, entities flattened"
        );
        assert_eq!(edited.media.as_deref(), Some("[photo]"));
        assert!(edited.deleted.is_none());

        let back = &rows[2];
        assert!(back.recovered && back.service);
        assert_eq!(back.from, "Marko");
        assert_eq!(back.preview, "[pin message]");

        let d = detail(&path, 7, 2).await.unwrap().unwrap();
        assert_eq!(d.text, "Druga\nlinija");
        assert_eq!(d.edited.as_deref(), Some("2025-12-12 17:00"));
        assert_eq!(d.versions.len(), 1);
        assert_eq!(d.versions[0].text, "prva");
        assert!(detail(&path, 7, 99).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_stored_file_is_extracted_under_its_own_name_and_no_higher() {
        let path = tmp("extract");
        {
            let s = tgx_archive::Store::open(&path).await.unwrap();
            s.merge(7, 1, None, &msg(1, json!("x")), 100).await.unwrap();
            s.put_blob(
                991,
                "files",
                "text/plain",
                Some("..\\..\\evil.txt"),
                b"hi",
                100,
            )
            .await
            .unwrap();
            s.link_media(7, 1, "file", 991, "files", "files/evil.txt")
                .await
                .unwrap();
        }
        let files = media(&path, 7, 1).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "evil.txt");
        let out = extract(&path, &files[0]).await.unwrap();
        assert_eq!(out, temp_dir().join("991").join("evil.txt"));
        assert_eq!(std::fs::read(&out).unwrap(), b"hi");
        let _ = std::fs::remove_dir_all(temp_dir().join("991"));
    }

    #[test]
    fn a_name_with_nothing_left_after_cleaning_gets_one() {
        assert_eq!(safe_name("photos/", 5), "file_5");
        assert_eq!(safe_name("a:b?.pdf", 5), "a_b_.pdf");
        assert_eq!(safe_name("photos/photo_1.jpg", 5), "photo_1.jpg");
    }
}
