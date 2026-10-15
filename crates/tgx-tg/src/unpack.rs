//! **Write a database back out as a Classic folder, offline.**
//!
//! A Database run keeps one accumulating `<chat>.sqlite` and writes no folder,
//! which is the right trade for syncing and the wrong one for handing somebody
//! a chat they can open in a browser. This turns the file back into what a
//! Classic run of the same chat would have produced — `result.json` and the
//! pages, one folder per topic with an `export_results.html` for a forum — out
//! of what the database already holds, with no connection.
//!
//! **Through the same [`Output`] the engine writes with**, fed the payloads the
//! database stored, which are the maps the engine handed `Output::add` minus
//! `_p`. So the JSON is Desktop's bytes by the same argument the archive leg
//! makes: the store gives back exactly what it was given, and `Output` is what
//! wrote those bytes in the first place.
//!
//! **What it cannot reproduce is what the database never kept.** `_p` is
//! stripped before the merge, so the pages are rebuilt from the JSON alone —
//! see [`presentation`] — and the media is whatever bytes the database holds:
//! a file over the blob ceiling, one an earlier run skipped, the rendered
//! `_thumb` previews and the stripped blur thumbnails were never stored, and
//! are left out rather than invented.
//!
//! **Every message, deleted ones included.** That is the point of the archive:
//! a message Telegram no longer returns is still a message somebody sent.

use crate::config::Settings;
use crate::output::Output;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use tgx_archive::{MediaRow, Store, StoredTopic};
use tgx_media::topics::{topic_dirname, GENERAL_TITLE, GENERAL_TOPIC_ID};

/// One chat written out.
#[derive(Debug, Clone, Default)]
pub struct Unpacked {
    pub chat_id: i64,
    pub title: String,
    /// The folder this chat went into — fresh, `(2)`-suffixed on a clash.
    pub root: PathBuf,
    /// Topic folders written; 1 for a chat that is not split by topic.
    pub topics: usize,
    pub messages: usize,
    /// Of `messages`, how many Telegram had stopped returning.
    pub deleted: usize,
    /// Media files and thumbnails written out of the database.
    pub files: usize,
    /// Files a message points at whose bytes the database does not hold.
    pub missing: usize,
    /// Media rows whose recorded path would land outside the topic folder.
    pub refused: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum UnpackError {
    #[error(transparent)]
    Archive(#[from] tgx_archive::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("nothing to write: HTML and JSON are both off")]
    NothingToWrite,
    #[error("{} records no chat", .0.display())]
    NoChat(PathBuf),
}

/// Write every chat `db` records into a folder of its own under `out_dir`.
///
/// `settings` decides the formats (`export_html`, `export_json`) and the page
/// size. **`export_db` is ignored**: this *is* the folder output, whatever mode
/// the settings were saved in, so it is forced off here rather than letting
/// [`Settings::writes_folders`] answer "no folder" for a Database user.
///
/// The file is asked [`Store::chats_in`] first, which opens it read-only and
/// refuses a schema this build does not know — so a `.sqlite` that is not one
/// of ours is refused before [`Store::open`] gets the chance to write into it.
pub async fn write_classic(
    db: &Path,
    out_dir: &Path,
    settings: &Settings,
) -> Result<Vec<Unpacked>, UnpackError> {
    let mut settings = settings.clone();
    settings.export_db = false;
    if !settings.writes_folders() {
        return Err(UnpackError::NothingToWrite);
    }
    let chats = Store::chats_in(db).await?;
    if chats.is_empty() {
        return Err(UnpackError::NoChat(db.to_path_buf()));
    }
    let store = Store::open(db).await?;
    let mut out = Vec::new();
    for chat in chats {
        out.push(unpack_chat(&store, chat.id, &chat.title, out_dir, &settings).await?);
    }
    Ok(out)
}

/// Is this chat laid out one folder per topic?
///
/// **Decided from what the topics recorded, not from a setting**, because the
/// setting that mattered was the one in force when the database was filled. A
/// split run writes `topic_id` into every topic's head (`engine::topic_head`);
/// an unsplit one records topic 1 with the chat's own head and files every
/// message under it. A message filed anywhere but 1 is a split run's work
/// even if its topic row was somehow never written.
fn is_split(topics: &[StoredTopic], message_topics: &[i64]) -> bool {
    topics.iter().any(|t| t.head.contains_key("topic_id"))
        || message_topics.iter().any(|&t| t != GENERAL_TOPIC_ID)
}

async fn unpack_chat(
    store: &Store,
    chat_id: i64,
    listed_title: &str,
    out_dir: &Path,
    settings: &Settings,
) -> Result<Unpacked, UnpackError> {
    let (title, kind) = store
        .chat(chat_id)
        .await?
        .unwrap_or_else(|| (listed_title.to_string(), String::new()));
    let rows = store.topics_of(chat_id).await?;
    let message_topics = store.message_topics(chat_id).await?;
    let split = is_split(&rows, &message_topics);

    let mut media: HashMap<i64, Vec<MediaRow>> = HashMap::new();
    for row in store.media_rows_of(chat_id).await? {
        media.entry(row.message_id).or_default().push(row);
    }

    let root = crate::engine::unique_dir(out_dir, &title)?;
    let mut done = Unpacked {
        chat_id,
        title: title.clone(),
        root: root.clone(),
        ..Unpacked::default()
    };
    // Who wrote what, by `from_id`, for the index's "opened by". The engine
    // has the roster for this; offline, the messages are what is left.
    let mut names: HashMap<String, String> = HashMap::new();

    if !split {
        let head = rows
            .iter()
            .find(|t| t.id == GENERAL_TOPIC_ID)
            .map(|t| t.head.clone())
            .unwrap_or_default();
        // An empty chat still gets its folder and an empty `result.json`,
        // exactly as a Classic run of it would: an empty export, not none.
        let mut output = Output::new(
            &root,
            &title,
            &kind,
            chat_id,
            settings,
            None,
            (!head.is_empty()).then_some(head),
        )?;
        let written = write_topic(
            store,
            chat_id,
            GENERAL_TOPIC_ID,
            &root,
            &mut output,
            &media,
            &mut done,
            &mut names,
        )
        .await;
        output.close()?;
        written?;
        done.topics = 1;
        log_done(&done);
        return Ok(done);
    }

    let mut ids: Vec<i64> = rows.iter().map(|t| t.id).collect();
    ids.extend(&message_topics);
    ids.sort_unstable();
    ids.dedup();

    let mut entries = Vec::new();
    let mut chat_head = Map::new();
    for id in ids {
        let row = rows.iter().find(|t| t.id == id);
        let topic_title = match row {
            Some(t) => t.title.clone(),
            None if id == GENERAL_TOPIC_ID => GENERAL_TITLE.to_string(),
            None => format!("Topic {id}"),
        };
        let head = match row {
            Some(t) => t.head.clone(),
            None => {
                let mut h = Map::new();
                h.insert("topic_id".into(), json!(id));
                h
            }
        };
        if chat_head.is_empty() {
            chat_head = head.clone();
        }
        let dirname = topic_dirname(id, &topic_title);
        let dir = root.join(&dirname);
        let before = done.messages;
        let mut output = Output::new(
            &dir,
            &topic_title,
            &kind,
            chat_id,
            settings,
            Some("../export_results.html".to_string()),
            Some(head.clone()),
        )?;
        let written = write_topic(
            store,
            chat_id,
            id,
            &dir,
            &mut output,
            &media,
            &mut done,
            &mut names,
        )
        .await;
        output.close()?;
        written?;
        let messages = done.messages - before;
        // A topic with nothing in it gets no folder, as in `close_all`.
        if messages == 0 {
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        }
        done.topics += 1;
        entries.push(tgx_html::index::IndexEntry {
            href: format!("{dirname}/messages.html"),
            title: topic_title.clone(),
            initials: tgx_format::peer::initials_from_title(&topic_title).unwrap_or_default(),
            colour: tgx_html::userpic::userpic_class(&id.to_string(), None),
            detail: topic_state(&head),
            subname: topic_origin(&head, &names),
            messages,
        });
    }

    if settings.export_html {
        let mut parts = vec![
            tgx_html::index::plural(entries.len(), "topic"),
            tgx_html::index::plural(done.messages, "message"),
        ];
        // The roster is not re-read here; its size is in the header the run
        // that fetched it wrote.
        if let Some(n) = chat_head.get("members_listed").and_then(Value::as_u64) {
            if n > 0 {
                parts.push(tgx_html::index::plural(n as usize, "member"));
            }
        }
        let about = (chat_head.contains_key("invite_link")
            || chat_head.contains_key("invite_links"))
        .then_some("Invite link: [INVITE LINK] — redacted here, kept in result.json");
        tgx_html::index::write_index(&root, &title, &parts.join(", "), about, &entries)?;
    }
    log_done(&done);
    Ok(done)
}

/// Ids and counts only — the log sits beside the executable.
fn log_done(d: &Unpacked) {
    log::info!(
        "unpack: chat {} → {} topic folder(s), {} messages ({} deleted), {} files, {} missing",
        d.chat_id,
        d.topics,
        d.messages,
        d.deleted,
        d.files,
        d.missing
    );
}

/// One topic's messages, oldest first, with their files, into `output`.
#[allow(clippy::too_many_arguments)]
async fn write_topic(
    store: &Store,
    chat_id: i64,
    topic_id: i64,
    dir: &Path,
    output: &mut Output,
    media: &HashMap<i64, Vec<MediaRow>>,
    done: &mut Unpacked,
    names: &mut HashMap<String, String>,
) -> Result<(), UnpackError> {
    for m in store.messages_of(chat_id, topic_id).await? {
        if let (Some(id), Some(name)) = (
            m.payload.get("from_id").and_then(Value::as_str),
            m.payload.get("from").and_then(Value::as_str),
        ) {
            names
                .entry(id.to_string())
                .or_insert_with(|| name.to_string());
        }
        if let Some(rows) = media.get(&m.id) {
            write_media(store, dir, rows, &m.payload, done).await?;
        }
        // **After the media**, because the preview is checked against what
        // is actually on disk.
        let mut payload = m.payload;
        if let Some(p) = presentation(&payload, dir) {
            payload.insert("_p".into(), Value::Object(p));
        }
        output.add(&payload)?;
        done.messages += 1;
        if m.deleted_seen.is_some() {
            done.deleted += 1;
        }
    }
    Ok(())
}

/// A stored path, if it stays inside the folder it is relative to.
///
/// **The database is a file somebody can hand you**, and `media.path` is text
/// in it. A path of `..\..\Startup\x.exe` joined onto the topic folder would
/// write wherever it pointed, so anything but plain names is refused.
fn contained(path: &str) -> Option<&Path> {
    let p = Path::new(path);
    (!path.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))).then_some(p)
}

/// Write one message's files out of the database, and the thumbnail its
/// payload names.
///
/// **A file already on disk is not written again**: two messages forwarding
/// the same photo share one path, exactly as the folder export wrote it once.
async fn write_media(
    store: &Store,
    dir: &Path,
    rows: &[MediaRow],
    payload: &Map<String, Value>,
    done: &mut Unpacked,
) -> Result<(), UnpackError> {
    for row in rows {
        let Some(rel) = contained(&row.path) else {
            log::warn!(
                "unpack: message {} names a media path outside its folder; skipped",
                row.message_id
            );
            done.refused += 1;
            continue;
        };
        let dest = dir.join(rel);
        if dest.exists() {
            continue;
        }
        match store.blob(row.file_id, &row.kind).await? {
            Some(bytes) => {
                write_file(&dest, &bytes)?;
                done.files += 1;
            }
            // Over the blob ceiling, or a link whose bytes never arrived. The
            // JSON still names it, as the folder export's would have.
            None => done.missing += 1,
        }
    }

    // Telegram's own thumbnail, stored against the file it belongs to. Only a
    // document has one — a photo *is* its own image.
    let thumb = payload
        .get("thumbnail")
        .and_then(Value::as_str)
        .filter(|t| !tgx_format::placeholder::is_skipped(t));
    let owner = rows.iter().find(|r| r.role == "file");
    if let (Some(thumb), Some(owner)) = (thumb, owner) {
        if let Some(rel) = contained(thumb) {
            let dest = dir.join(rel);
            if !dest.exists() {
                if let Some(bytes) = store.thumb(owner.file_id, &owner.kind).await? {
                    write_file(&dest, &bytes)?;
                    done.files += 1;
                }
            }
        }
    }
    Ok(())
}

fn write_file(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, bytes)
}

/// The `_p` map for a stored message: what the pages need that the JSON does
/// not carry, rebuilt from the JSON and from what is on disk in `dir`.
///
/// **Only the preview.** The rest of what the engine puts here cannot be
/// recovered from a payload, and each gap degrades the way the writer already
/// handles a missing key:
///
/// * `from_name` — the writer falls back to the payload's `from`, which is the
///   trimmed form of the same name; filling it in from there would be the same
///   fallback written twice.
/// * `initials` and `colours` — derived from the name and the id instead,
///   which is right for everyone without a self-chosen colour.
/// * `group` — albums are not recorded, so a joined forward in an album
///   repeats its header.
/// * `reactions_chosen` — which reaction was the account's own is not stored.
///
/// The preview is [`tgx_html::preview::preview_of`]'s, **pointed at a file
/// that exists**. The rendered `_thumb` preview Desktop names is an HTML-only
/// artefact the database never stored, so a photo or a static sticker is shown
/// from its own full file instead — a larger download for the browser, and an
/// image rather than a coloured placeholder row. Where neither exists the
/// preview is dropped and the writer draws the row.
pub fn presentation(payload: &Map<String, Value>, dir: &Path) -> Option<Map<String, Value>> {
    let mut pv = tgx_html::preview::preview_of(payload)?;
    let on_disk = |rel: &str| contained(rel).is_some_and(|r| dir.join(r).is_file());
    let src = pv.get("src").and_then(Value::as_str).unwrap_or("");
    if !on_disk(src) {
        let s = |k: &str| payload.get(k).and_then(Value::as_str).unwrap_or("");
        let original = if !s("photo").is_empty() {
            s("photo")
        } else if s("media_type") == "sticker" {
            s("file")
        } else {
            ""
        };
        if !on_disk(original) {
            return None;
        }
        pv["src"] = json!(original);
    }
    let mut p = Map::new();
    p.insert("preview".into(), pv);
    Some(p)
}

/// `pinned`, `closed`, `hidden`, joined — the engine's `topic_state`, read off
/// the head it wrote rather than off a listing.
fn topic_state(head: &Map<String, Value>) -> Option<String> {
    let marks: Vec<&str> = [
        ("topic_pinned", "pinned"),
        ("topic_closed", "closed"),
        ("topic_hidden", "hidden"),
    ]
    .into_iter()
    .filter(|(key, _)| head.get(*key).and_then(Value::as_bool) == Some(true))
    .map(|(_, mark)| mark)
    .collect();
    (!marks.is_empty()).then(|| marks.join(", "))
}

/// `opened by Ivana, 2025-12-14` — or just the date, when nothing this chat
/// wrote names the creator. General carries no date, so no line.
fn topic_origin(head: &Map<String, Value>, names: &HashMap<String, String>) -> Option<String> {
    let created = head.get("topic_created").and_then(Value::as_str)?;
    let date = created.get(..10)?;
    let by = head
        .get("topic_created_by")
        .and_then(Value::as_str)
        .and_then(|k| names.get(k))
        .filter(|n| !n.is_empty());
    Some(match by {
        Some(who) => format!("opened by {who}, {date}"),
        None => format!("opened {date}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tgx-unpack-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    fn json_only() -> Settings {
        Settings {
            export_html: false,
            export_json: true,
            ..Settings::default()
        }
    }

    fn msg(id: i64, text: &str) -> Map<String, Value> {
        obj(json!({
            "id": id,
            "type": "message",
            "date": "2025-12-18T16:00:00",
            "date_unixtime": "1766071072",
            "from": "Ivana",
            "from_id": "user31774388",
            "text": text,
            "text_entities": [{ "type": "plain", "text": text }],
        }))
    }

    /// A forum of two topics, one message of which Telegram no longer has,
    /// and a photo whose bytes the database holds.
    async fn forum(dir: &Path) -> PathBuf {
        let db = dir.join("Dev Team").join("Dev Team.sqlite");
        let store = Store::open(&db).await.unwrap();
        store
            .upsert_chat(7, "Dev Team", "public_supergroup", 100)
            .await
            .unwrap();
        let general = obj(json!({ "topic_id": 1, "about": "a group" }));
        store
            .upsert_topic(7, 1, "General", &general, 100)
            .await
            .unwrap();
        let backend = obj(json!({
            "topic_id": 5,
            "topic_created_by": "user31774388",
            "topic_created": "2025-12-14T10:00:00",
            "topic_pinned": true,
            "about": "a group",
        }));
        store
            .upsert_topic(7, 5, "Backend", &backend, 100)
            .await
            .unwrap();
        store
            .merge(7, 1, Some(1), &msg(1, "hello"), 100)
            .await
            .unwrap();
        store
            .merge(7, 2, Some(5), &msg(2, "gone"), 100)
            .await
            .unwrap();
        let mut photo = msg(3, "look");
        photo.insert("photo".into(), json!("photos/photo_1.jpg"));
        photo.insert("photo_file_size".into(), json!(5));
        photo.insert("width".into(), json!(480));
        photo.insert("height".into(), json!(320));
        let photo = tgx_format::order::ordered(&photo);
        store.merge(7, 3, Some(5), &photo, 100).await.unwrap();
        store
            .put_blob(99, "photos", "image/jpeg", None, b"pixel", 100)
            .await
            .unwrap();
        store
            .link_media(7, 3, "photo", 99, "photos", "photos/photo_1.jpg")
            .await
            .unwrap();
        // Seen by a later finished walk: message 2 went missing on Telegram.
        let seen = [1, 3].into_iter().collect();
        store.mark_unseen_deleted(7, 0, &seen, 200).await.unwrap();
        db
    }

    #[tokio::test]
    async fn a_forum_comes_back_one_folder_per_topic_with_its_deleted_message() {
        let dir = tmp("forum");
        let db = forum(&dir).await;
        let out = dir.join("out");
        let done = write_classic(&db, &out, &json_only()).await.unwrap();
        assert_eq!(done.len(), 1);
        let d = &done[0];
        assert_eq!((d.topics, d.messages, d.deleted), (2, 3, 1));
        assert_eq!(d.root, out.join("Dev Team"));

        let backend = d.root.join("0005 - Backend");
        let raw = std::fs::read_to_string(backend.join("result.json")).unwrap();
        let parsed: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["name"], "Backend");
        assert_eq!(parsed["type"], "public_supergroup");
        assert_eq!(parsed["id"], 7);
        assert_eq!(parsed["topic_id"], 5, "the topic's own head");
        assert_eq!(parsed["about"], "a group", "and the chat's details in it");
        let ids: Vec<i64> = parsed["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, vec![2, 3], "the deleted message is kept");
        assert!(
            !raw.contains("\"_p\"") && !raw.contains("preview"),
            "the rebuilt presentation stays out"
        );

        let general = d.root.join("0001 - General").join("result.json");
        let parsed: Value =
            serde_json::from_str(&std::fs::read_to_string(general).unwrap()).unwrap();
        assert_eq!(parsed["messages"][0]["id"], 1);

        assert_eq!(
            std::fs::read(backend.join("photos").join("photo_1.jpg")).unwrap(),
            b"pixel",
            "the blob lands at the path the message names"
        );
        assert_eq!(d.files, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_pages_and_the_index_come_back_with_the_photo_inline() {
        let dir = tmp("pages");
        let db = forum(&dir).await;
        let s = Settings {
            export_html: true,
            export_json: false,
            ..Settings::default()
        };
        let d = write_classic(&db, &dir.join("out"), &s)
            .await
            .unwrap()
            .remove(0);
        let index = std::fs::read_to_string(d.root.join("export_results.html")).unwrap();
        assert!(index.contains("0005 - Backend/messages.html"), "{index}");
        assert!(index.contains("pinned"));
        assert!(index.contains("opened by Ivana, 2025-12-14"), "{index}");
        let page =
            std::fs::read_to_string(d.root.join("0005 - Backend").join("messages.html")).unwrap();
        // The `_thumb` preview was never stored; the photo itself is shown.
        assert!(page.contains(r#"src="photos/photo_1.jpg""#), "{page}");
        assert!(!d.root.join("0005 - Backend").join("result.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_json_is_what_output_writes_for_the_same_payloads() {
        // The round trip the archive leg proves on the real export, through
        // the writer: Desktop-shaped messages merged into a store and unpacked
        // come back as exactly the bytes `Output` makes of them directly.
        let dir = tmp("roundtrip");
        let mut messages = vec![msg(10, "first"), msg(11, "ćao, šta ima")];
        let mut service = obj(json!({
            "id": 12,
            "type": "service",
            "date": "2025-12-18T16:05:00",
            "date_unixtime": "1766071372",
            "actor": "Ivana",
            "actor_id": "user31774388",
            "action": "pin_message",
            "message_id": 10,
            "text": "",
            "text_entities": [],
        }));
        service = tgx_format::order::ordered(&service);
        messages.push(service);
        let mut reacted = msg(13, "with reactions");
        reacted.insert(
            "reactions".into(),
            json!([{ "type": "emoji", "count": 2, "emoji": "👍" }]),
        );
        messages.push(tgx_format::order::ordered(&reacted));

        let head = obj(json!({ "about": "a description" }));
        let db = dir.join("db").join("Mum.sqlite");
        {
            let store = Store::open(&db).await.unwrap();
            store
                .upsert_chat(42, "Mum", "personal_chat", 100)
                .await
                .unwrap();
            store.upsert_topic(42, 1, "Mum", &head, 100).await.unwrap();
            for m in &messages {
                let id = m["id"].as_i64().unwrap();
                store.merge(42, id, None, m, 100).await.unwrap();
            }
        }

        let direct = dir.join("direct");
        {
            let mut o = Output::new(
                &direct,
                "Mum",
                "personal_chat",
                42,
                &json_only(),
                None,
                Some(head.clone()),
            )
            .unwrap();
            for m in &messages {
                o.add(m).unwrap();
            }
            o.close().unwrap();
        }

        let d = write_classic(&db, &dir.join("out"), &json_only())
            .await
            .unwrap()
            .remove(0);
        assert_eq!(d.topics, 1);
        assert_eq!(d.root, dir.join("out").join("Mum"), "not split: one folder");
        assert_eq!(
            std::fs::read(d.root.join("result.json")).unwrap(),
            std::fs::read(direct.join("result.json")).unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_media_path_that_leaves_the_folder_is_refused() {
        let dir = tmp("escape");
        let db = dir.join("x.sqlite");
        {
            let store = Store::open(&db).await.unwrap();
            store.upsert_chat(1, "X", "private_group", 1).await.unwrap();
            store.merge(1, 1, None, &msg(1, "x"), 1).await.unwrap();
            store
                .put_blob(5, "files", "text/plain", None, b"owned", 1)
                .await
                .unwrap();
            store
                .link_media(1, 1, "file", 5, "files", "../../escaped.txt")
                .await
                .unwrap();
        }
        let d = write_classic(&db, &dir.join("out"), &json_only())
            .await
            .unwrap()
            .remove(0);
        assert_eq!((d.refused, d.files), (1, 0));
        assert!(!dir.join("escaped.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_database_mode_setting_still_writes_a_folder_and_nothing_on_is_refused() {
        let dir = tmp("modes");
        let db = forum(&dir).await;
        let s = Settings {
            export_db: true,
            ..json_only()
        };
        assert!(write_classic(&db, &dir.join("a"), &s).await.is_ok());
        let none = Settings {
            export_html: false,
            export_json: false,
            ..Settings::default()
        };
        assert!(matches!(
            write_classic(&db, &dir.join("b"), &none).await,
            Err(UnpackError::NothingToWrite)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_preview_with_no_file_behind_it_is_dropped() {
        let dir = tmp("preview");
        let m = obj(json!({ "photo": "photos/p.jpg", "width": 480, "height": 320 }));
        assert_eq!(presentation(&m, &dir), None);
        std::fs::create_dir_all(dir.join("photos")).unwrap();
        std::fs::write(dir.join("photos").join("p.jpg"), b"x").unwrap();
        let p = presentation(&m, &dir).unwrap();
        assert_eq!(p["preview"]["src"], "photos/p.jpg");
        assert_eq!(p["preview"]["width"], 240);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
