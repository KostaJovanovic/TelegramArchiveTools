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
    /// `2025-12-18`, the day separator's key.
    pub day: String,
    /// `16:00`, the time in the corner of a bubble.
    pub time: String,
    /// The sender, or for a service message the actor. Empty when the payload
    /// names neither.
    pub from: String,
    /// `user123` — what groups consecutive messages into one run of bubbles,
    /// since two people can share a display name.
    pub from_id: String,
    /// Desktop's userpic colour for the sender, 1 to 8 — the same colour the
    /// HTML export paints, from the same rule.
    pub colour: u8,
    /// One or two letters for the sender's userpic.
    pub initials: String,
    /// The whole text, entities flattened. For a service message, Desktop's
    /// sentence for it ("Ivana pinned this message").
    pub text: String,
    /// The first non-empty line of the text, clipped. For a service message,
    /// its action in brackets.
    pub preview: String,
    /// `[photo]`, `[voice message]`, `[report.pdf]` — what the message
    /// carried, from the payload, whether or not the bytes are stored.
    pub media: Option<String>,
    /// What else is known of it: `1280×720`, `0:42`, a file's name.
    pub media_detail: Option<String>,
    pub reply_to: Option<i64>,
    /// What the message carries besides its text, in a shape the bubble can
    /// draw: a picture, a player, a card. `None` for plain text.
    pub attachment: Option<Attachment>,
    /// The export left the file out — Desktop's `(File not included…)` in
    /// place of a path — so there are no bytes anywhere to show.
    pub file_skipped: bool,
    /// Each reaction as its emoji and how many gave it.
    pub reactions: Vec<(String, i64)>,
    /// "Forwarded from" whom.
    pub forwarded: Option<String>,
    /// The payload's own `edited` stamp — Telegram's, so a real edit, unlike
    /// [`Row::version`], which also moves when reactions do.
    pub edited_at: Option<String>,
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
    /// Edited: Telegram stamped it as edited, or the database has seen it
    /// change since it was first stored. The bubble's "edited" marker, the
    /// Edited filter and the count beneath the list all ask this, so the
    /// three cannot disagree.
    pub fn edited(&self) -> bool {
        self.version > 1 || self.edited_at.is_some()
    }
}

/// What a message carries, one variant per way Telegram draws it.
///
/// **Every media key the export writes lands in one of these**, so nothing a
/// message held reaches the screen as a bare label. Read off Desktop's keys —
/// the payload — and not off the `media` table, so a message whose file was
/// never stored still says what it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// Something to look at. The bytes come from [`picture_bytes`].
    Visual {
        kind: Visual,
        /// `[width, height]` as the payload gives them; `[0, 0]` when it does
        /// not, until the picture's own size is known.
        size: [u32; 2],
        duration: Option<i64>,
    },
    Voice {
        duration: Option<i64>,
    },
    Audio {
        title: String,
        performer: String,
        duration: Option<i64>,
    },
    /// Any other file: its name, its size in bytes, its extension.
    Document {
        name: String,
        size: Option<i64>,
        ext: String,
    },
    Poll(Poll),
    Location {
        title: String,
        address: String,
        /// As the export wrote them, so the text matches the HTML.
        lat: String,
        lon: String,
        live: bool,
    },
    Contact {
        name: String,
        /// For the userpic, by Desktop's rule.
        initials: String,
        phone: String,
    },
    Game {
        title: String,
        description: String,
    },
    Invoice {
        title: String,
        description: String,
        amount: String,
    },
    Dice {
        emoji: String,
        value: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visual {
    Photo,
    /// An image sent as a file.
    Image,
    Sticker,
    Video,
    Gif,
    /// A round video message.
    Round,
}

/// A poll as the export recorded it: Desktop's own fields, nothing derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Poll {
    pub question: String,
    pub answers: Vec<PollAnswer>,
    pub total: i64,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollAnswer {
    pub text: String,
    pub voters: i64,
    /// The exporting account's own vote.
    pub chosen: bool,
}

/// One forum topic that holds messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topic {
    pub id: i64,
    pub title: String,
}

/// A chat as the viewer holds it: every message, and the topics they are
/// filed under — more than one only for a forum exported split by topic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    pub rows: Vec<Row>,
    pub topics: Vec<Topic>,
}

/// Every message of `chat_id` in the database at `path`, oldest first.
pub async fn load(path: &Path, chat_id: i64) -> Result<Loaded, tgx_archive::Error> {
    let path = path.to_path_buf();
    off_the_runtime(async move {
        let store = tgx_archive::Store::open_read(&path).await?;
        let recovered = store.recovered(chat_id).await?;
        let messages = store.all_messages(chat_id).await?;
        // Every topic a message is filed under, named where a run listed it.
        // A topic with messages and no row of its own is still a topic.
        let named: std::collections::HashMap<i64, String> = store
            .topics_of(chat_id)
            .await?
            .into_iter()
            .map(|t| (t.id, t.title))
            .collect();
        let topics = store
            .message_topics(chat_id)
            .await?
            .into_iter()
            .map(|id| Topic {
                id,
                title: named.get(&id).cloned().unwrap_or_else(|| match id {
                    1 => "General".into(),
                    _ => format!("Topic {id}"),
                }),
            })
            .collect();
        // The topic id alone covers a forum exported split; this covers one
        // exported unsplit, where every message is filed under no topic.
        let roots: std::collections::HashSet<i64> = messages
            .iter()
            .filter(|m| creates_topic(&m.payload))
            .map(|m| m.id)
            .collect();
        let rows = messages
            .into_iter()
            .map(|m| {
                let mut row = row_of(m, &recovered);
                if row.reply_to.is_some_and(|id| roots.contains(&id)) {
                    row.reply_to = None;
                }
                row
            })
            .collect();
        Ok(Loaded { rows, topics })
    })
    .await
}

/// The message this one answers, or `None` if it only sits in a topic.
///
/// **In a forum every message "replies" to its topic.** Telegram files an
/// ordinary topic message as a reply to the topic's own *created topic*
/// message, and Desktop's export records it so — 2,463 of 2,541 messages in
/// one reference topic. Telegram's apps draw no quote for it, and neither may
/// we: it made most bubbles in a forum open on "[topic created]". The export
/// keeps the field, because it is Desktop's; only the viewer reads past it.
///
/// A topic's id is its creating message's id. General (1) is not created by a
/// message, so a reply to message 1 there is a real one.
fn reply_of(p: &Map<String, Value>, topic_id: i64) -> Option<i64> {
    p.get("reply_to_message_id")
        .and_then(Value::as_i64)
        .filter(|&id| topic_id == 1 || id != topic_id)
}

fn creates_topic(p: &Map<String, Value>) -> bool {
    p.get("action").and_then(Value::as_str) == Some("topic_created")
}

fn row_of(m: tgx_archive::StoredMessage, recovered: &std::collections::HashSet<i64>) -> Row {
    let p = &m.payload;
    let date = local_date(p);
    let from = sender(p);
    let from_id = p
        .get("from_id")
        .or_else(|| p.get("actor_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let service = is_service(p);
    Row {
        id: m.id,
        topic_id: m.topic_id,
        day: date.get(..10).unwrap_or("").to_string(),
        time: date.get(11..16).unwrap_or("").to_string(),
        date,
        colour: colour_of(&from_id),
        // A sender written as their `@handle` (Settings → own names) would
        // otherwise be "@" on every userpic.
        initials: tgx_format::peer::initials_from_name(from.trim_start_matches('@')),
        from,
        from_id,
        text: if service { sentence(p) } else { text_of(p) },
        preview: preview(p),
        media: media_label(p),
        media_detail: media_detail(p),
        reply_to: reply_of(p, m.topic_id),
        attachment: attachment_of(p),
        file_skipped: ["photo", "file"].iter().any(|k| {
            p.get(*k)
                .and_then(Value::as_str)
                .is_some_and(|v| v.starts_with("(File"))
        }),
        reactions: reactions_of(p),
        forwarded: p
            .get("forwarded_from")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        edited_at: p.get("edited").and_then(Value::as_str).map(tidy_date),
        deleted: m.deleted_seen.map(unix_date),
        version: m.version,
        recovered: recovered.contains(&m.id),
        service,
    }
}

/// Desktop's userpic colour for a `from_id` such as `user123`: its digits,
/// through the same rule the HTML export uses.
fn colour_of(from_id: &str) -> u8 {
    let digits: String = from_id.chars().filter(char::is_ascii_digit).collect();
    tgx_format::peer::userpic_colour(digits.parse().unwrap_or(0))
}

/// A service message as Desktop's sentence, in plain text: the HTML writer's
/// own wording, its markup and entities taken out.
fn sentence(p: &Map<String, Value>) -> String {
    let html = tgx_html::service::service_text(p, None);
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&laquo;", "«")
        .replace("&raquo;", "»")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// Dimensions, a duration, or a file's name — whichever the payload has.
fn media_detail(p: &Map<String, Value>) -> Option<String> {
    let int = |k: &str| p.get(k).and_then(Value::as_i64);
    if let Some(secs) = int("duration_seconds") {
        return Some(format!("{}:{:02}", secs / 60, secs % 60));
    }
    if let (Some(w), Some(h)) = (int("width"), int("height")) {
        return Some(format!("{w}×{h}"));
    }
    p.get("file_name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
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

/// Each person's profile picture, `(sender key, bytes)`: the one they wear
/// now, or if the last run did not see them wear any, the newest held. Empty
/// for a database that keeps none.
pub async fn userpics(
    path: &Path,
    chat_id: i64,
) -> Result<Vec<(String, Vec<u8>)>, tgx_archive::Error> {
    let path = path.to_path_buf();
    off_the_runtime(async move {
        let store = tgx_archive::Store::open_read(&path).await?;
        let mut out: Vec<(String, Vec<u8>)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        // `userpics_of` lists the worn ones first, then each person's newest,
        // so the first sighting of a person is the one to show.
        for pic in store.userpics_of(chat_id).await? {
            if !seen.insert(pic.peer.clone()) {
                continue;
            }
            if let Some(bytes) = store.blob(pic.photo_id, tgx_archive::USERPIC_KIND).await? {
                out.push((pic.peer, bytes));
            }
        }
        Ok(out)
    })
    .await
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
        let mut reply_to = reply_of(p, m.topic_id);
        if let Some(target) = reply_to {
            if let Some(t) = store.get(chat_id, target).await? {
                if creates_topic(&t.payload) {
                    reply_to = None;
                }
            }
        }
        Ok(Some(Detail {
            id,
            date: local_date(p),
            from: sender(p),
            text: text_of(p),
            reply_to,
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
    let dir = temp_dir().join(media.file_id.to_string());
    let out = dir.join(safe_name(&media.name, media.file_id));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    save(path, media, &out).await?;
    Ok(out)
}

/// Write one stored file to `to`, a path the user chose — the reader's Save
/// as. Unlike [`extract`] it is not cleared on exit: keeping it is the point.
pub async fn save(path: &Path, media: &Media, to: &Path) -> Result<(), String> {
    let (path, file_id, kind, to) = (
        path.to_path_buf(),
        media.file_id,
        media.kind.clone(),
        to.to_path_buf(),
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
        std::fs::write(&to, bytes).map_err(|e| format!("{}: {e}", to.display()))
    })
    .await
}

/// The last component of `name`, and only that.
///
/// The name comes out of the database, and a database is a file somebody may
/// have handed over: `..\..\Startup\x.bat` must land inside the temp folder
/// as `x.bat`, not two levels above it.
///
/// The rest is the export's own filename rule,
/// [`tgx_media::names::sanitize_filename`], rather than a second copy of it:
/// that one also defuses Windows' device names, so a stored `NUL.jpg` or
/// `CON` is written as `_NUL.jpg` and not handed to a device that swallows the
/// bytes.
fn safe_name(name: &str, file_id: i64) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    // Leading dots too: `..` is not a name, and a file that hides itself is
    // not what Open or Save meant to hand anyone.
    let last = last.trim_start_matches(['.', ' ']);
    tgx_media::names::sanitize_filename(last, &format!("file_{file_id}"))
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
        ("game_title", "[game]"),
        ("invoice_information", "[invoice]"),
        ("dice_emoji", "[dice]"),
    ] {
        if p.contains_key(key) {
            return Some(label.into());
        }
    }
    None
}

fn str_of(m: &Map<String, Value>, k: &str) -> String {
    match m.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        // A poll's question and answers can carry entities, like a message.
        Some(v @ Value::Array(_)) => tgx_archive::plain_text(v),
        _ => String::new(),
    }
}

fn int_of(m: &Map<String, Value>, k: &str) -> Option<i64> {
    match m.get(k)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// What the message carries, as the bubble draws it. See [`Attachment`].
fn attachment_of(p: &Map<String, Value>) -> Option<Attachment> {
    let dim = |k: &str| {
        int_of(p, k)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0)
    };
    let size = [dim("width"), dim("height")];
    let duration = int_of(p, "duration_seconds");
    if p.contains_key("photo") {
        return Some(Attachment::Visual {
            kind: Visual::Photo,
            size,
            duration: None,
        });
    }
    if p.contains_key("file") {
        let mime = str_of(p, "mime_type");
        let visual = match p.get("media_type").and_then(Value::as_str) {
            Some("sticker") => Some(Visual::Sticker),
            Some("video_file") => Some(Visual::Video),
            Some("animation") => Some(Visual::Gif),
            Some("video_message") => Some(Visual::Round),
            Some("voice_message") => return Some(Attachment::Voice { duration }),
            Some("audio_file") => {
                return Some(Attachment::Audio {
                    title: str_of(p, "title"),
                    performer: str_of(p, "performer"),
                    duration,
                })
            }
            _ if mime.starts_with("image/") => Some(Visual::Image),
            _ if mime.starts_with("video/") => Some(Visual::Video),
            _ => None,
        };
        if let Some(kind) = visual {
            return Some(Attachment::Visual {
                kind,
                size,
                duration,
            });
        }
        let name = str_of(p, "file_name");
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_uppercase())
            .filter(|e| !e.is_empty() && e.len() <= 5)
            .unwrap_or_else(|| "FILE".into());
        return Some(Attachment::Document {
            name: if name.is_empty() { "File".into() } else { name },
            size: int_of(p, "file_size"),
            ext,
        });
    }
    if let Some(Value::Object(poll)) = p.get("poll") {
        return Some(Attachment::Poll(Poll {
            question: str_of(poll, "question"),
            closed: poll.get("closed").and_then(Value::as_bool).unwrap_or(false),
            total: int_of(poll, "total_voters").unwrap_or(0),
            answers: poll
                .get("answers")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_object)
                        .map(|a| PollAnswer {
                            text: str_of(a, "text"),
                            voters: int_of(a, "voters").unwrap_or(0),
                            chosen: a.get("chosen").and_then(Value::as_bool).unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }));
    }
    if let Some(Value::Object(loc)) = p.get("location_information") {
        let live = p.contains_key("live_location_period_seconds");
        let place = str_of(p, "place_name");
        return Some(Attachment::Location {
            title: if !place.is_empty() {
                place
            } else if live {
                "Live location".into()
            } else {
                "Location".into()
            },
            address: str_of(p, "address"),
            lat: str_of(loc, "latitude"),
            lon: str_of(loc, "longitude"),
            live,
        });
    }
    if let Some(Value::Object(c)) = p.get("contact_information") {
        let name = [str_of(c, "first_name"), str_of(c, "last_name")]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        return Some(Attachment::Contact {
            initials: tgx_format::peer::initials_from_name(&name),
            name: if name.is_empty() {
                "Contact".into()
            } else {
                name
            },
            phone: str_of(c, "phone_number"),
        });
    }
    if p.contains_key("game_title") {
        return Some(Attachment::Game {
            title: str_of(p, "game_title"),
            description: str_of(p, "game_description"),
        });
    }
    if let Some(Value::Object(inv)) = p.get("invoice_information") {
        let amount = match (int_of(inv, "amount"), str_of(inv, "currency")) {
            // Telegram's amounts are in the currency's smallest unit.
            (Some(a), c) if !c.is_empty() => format!("{}.{:02} {c}", a / 100, (a % 100).abs()),
            (Some(a), _) => a.to_string(),
            (None, _) => String::new(),
        };
        return Some(Attachment::Invoice {
            title: str_of(inv, "title"),
            description: str_of(inv, "description"),
            amount,
        });
    }
    if p.contains_key("dice_emoji") {
        return Some(Attachment::Dice {
            emoji: str_of(p, "dice_emoji"),
            value: int_of(p, "dice_value").unwrap_or(0),
        });
    }
    None
}

/// Each reaction as its emoji and its count. A custom emoji has no glyph in
/// the export, so it is drawn as Desktop's HTML draws it.
fn reactions_of(p: &Map<String, Value>) -> Vec<(String, i64)> {
    p.get("reactions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_object)
                .map(|r| {
                    let glyph = match r.get("emoji").and_then(Value::as_str) {
                        Some(e) if !e.is_empty() => e.to_string(),
                        _ => tgx_html::reactions::CUSTOM_REACTION_GLYPH.to_string(),
                    };
                    (glyph, int_of(r, "count").unwrap_or(0))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What [`picture_bytes`] found for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pictured {
    pub id: i64,
    /// Encoded images that could stand for it, best first.
    pub candidates: Vec<Vec<u8>>,
    /// The database holds the file itself — so a video with no thumbnail is
    /// still there to play, and is not "not saved".
    pub has_file: bool,
}

/// The largest stored file the viewer will decode into a picture. A photo is
/// a few megabytes; a 200 MB TIFF sent as a document is not worth a stall.
const PICTURE_MAX_BYTES: i64 = 32 * 1024 * 1024;

/// Is this stored file itself a picture, rather than something with one?
fn is_picture_file(m: &tgx_archive::MediaEntry) -> bool {
    match m.kind.as_str() {
        "photos" | "stickers" => true,
        "files" => {
            let name = m.file_name.as_deref().unwrap_or(&m.path).to_lowercase();
            [".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp"]
                .iter()
                .any(|e| name.ends_with(e))
        }
        _ => false,
    }
}

/// For each message, the encoded images that could stand for it, best
/// first: the file itself when it is a picture, then Telegram's thumbnail of
/// it, then the export's own `_thumb`. The window decodes them and keeps the
/// first that decodes — an animated sticker (`.tgs`) is not an image and
/// falls through to its thumbnail.
///
/// **Never a video's bytes**: only its thumbnail is read.
pub async fn picture_bytes(
    path: &Path,
    chat_id: i64,
    ids: Vec<i64>,
) -> Result<Vec<Pictured>, tgx_archive::Error> {
    let path = path.to_path_buf();
    off_the_runtime(async move {
        let store = tgx_archive::Store::open_read(&path).await?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let entries = store.media_of(chat_id, id).await?;
            let mut found = Vec::new();
            let (thumbs, files): (Vec<_>, Vec<_>) =
                entries.iter().partition(|m| m.role == "thumbnail");
            for m in &files {
                if is_picture_file(m) && m.size.is_some_and(|s| s <= PICTURE_MAX_BYTES) {
                    if let Some(b) = store.blob(m.file_id, &m.kind).await? {
                        found.push(b);
                    }
                }
            }
            for m in &files {
                if let Some(b) = store.thumb(m.file_id, &m.kind).await? {
                    found.push(b);
                }
            }
            for m in &thumbs {
                if m.size.is_some_and(|s| s <= PICTURE_MAX_BYTES) {
                    if let Some(b) = store.blob(m.file_id, &m.kind).await? {
                        found.push(b);
                    }
                }
            }
            out.push(Pictured {
                id,
                candidates: found,
                has_file: files.iter().any(|m| m.size.is_some()),
            });
        }
        Ok(out)
    })
    .await
}

/// The file a click on a message's media opens: the file itself, not its
/// thumbnail, and only one whose bytes the database holds.
pub async fn main_file(
    path: &Path,
    chat_id: i64,
    id: i64,
) -> Result<Option<Media>, tgx_archive::Error> {
    let files = media(path, chat_id, id).await?;
    let stored = |m: &&Media| m.size.is_some();
    Ok(files
        .iter()
        .filter(stored)
        .find(|m| m.role != "thumbnail")
        .or_else(|| files.iter().find(stored))
        .cloned())
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

        let loaded = load(&path, 7).await.unwrap();
        let rows = loaded.rows;
        assert_eq!(rows.len(), 3);
        // No forum here: everything is filed under topic 1.
        assert_eq!(
            loaded.topics,
            vec![Topic {
                id: 1,
                title: "General".into()
            }]
        );

        let gone = &rows[0];
        assert_eq!(gone.id, 1);
        assert_eq!(gone.date, "2025-12-11 16:01");
        assert_eq!(gone.from, "Ivana");
        assert_eq!(gone.preview, "ovo će nestati");
        assert_eq!(
            (gone.day.as_str(), gone.time.as_str()),
            ("2025-12-11", "16:01")
        );
        assert_eq!(gone.from_id, "user1");
        assert_eq!(gone.colour, tgx_format::peer::userpic_colour(1));
        assert_eq!(gone.initials, "I");
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
        assert_eq!(
            edited.text, "Druga\nlinija",
            "the whole text, for the bubble"
        );
        assert_eq!(edited.edited_at.as_deref(), Some("2025-12-12 17:00"));
        assert!(edited.deleted.is_none());

        let back = &rows[2];
        assert!(back.recovered && back.service);
        assert_eq!(back.from, "Marko");
        assert_eq!(back.preview, "[pin message]");
        assert!(
            back.text.starts_with("Marko pinned"),
            "Desktop's sentence, without its markup: {:?}",
            back.text
        );
        assert!(!back.text.contains('<'));

        let d = detail(&path, 7, 2).await.unwrap().unwrap();
        assert_eq!(d.text, "Druga\nlinija");
        assert_eq!(d.edited.as_deref(), Some("2025-12-12 17:00"));
        assert_eq!(d.versions.len(), 1);
        assert_eq!(d.versions[0].text, "prva");
        assert!(detail(&path, 7, 99).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn each_person_shows_the_picture_they_wear_now() {
        let path = tmp("userpics");
        {
            let s = tgx_archive::Store::open(&path).await.unwrap();
            let kind = tgx_archive::USERPIC_KIND;
            for (id, bytes) in [(1, b"old"), (2, b"now"), (3, b"one")] {
                s.put_blob(id, kind, "image/jpeg", None, bytes, 1)
                    .await
                    .unwrap();
            }
            s.record_userpic(7, "user1", 1, Some(100), false, 1)
                .await
                .unwrap();
            s.record_userpic(7, "user1", 2, Some(200), true, 1)
                .await
                .unwrap();
            // Nobody saw user2 wear it on the last run: still their newest.
            s.record_userpic(7, "user2", 3, None, false, 1)
                .await
                .unwrap();
        }
        let mut pics = userpics(&path, 7).await.unwrap();
        pics.sort();
        assert_eq!(
            pics,
            vec![
                ("user1".to_string(), b"now".to_vec()),
                ("user2".to_string(), b"one".to_vec())
            ]
        );
        assert!(userpics(&path, 8).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn sitting_in_a_topic_is_not_replying_to_its_creation() {
        let path = tmp("topic-replies");
        {
            let s = tgx_archive::Store::open(&path).await.unwrap();
            s.upsert_chat(7, "Forum", "private_supergroup", 1)
                .await
                .unwrap();
            let mut created = msg(15, json!(""));
            created.insert("type".into(), json!("service"));
            created.insert("action".into(), json!("topic_created"));
            let reply = |id: i64, to: i64| {
                let mut m = msg(id, json!("x"));
                m.insert("reply_to_message_id".into(), json!(to));
                m
            };
            // Split: filed under topic 15, as Desktop records every message.
            s.merge(7, 15, Some(15), &created, 1).await.unwrap();
            s.merge(7, 16, Some(15), &reply(16, 15), 1).await.unwrap();
            // A real reply inside the topic.
            s.merge(7, 17, Some(15), &reply(17, 16), 1).await.unwrap();
            // Unsplit: no topic to compare with, only the target's action.
            s.merge(7, 18, None, &reply(18, 15), 1).await.unwrap();
            // General has no creating message: a reply to 1 there is real.
            s.merge(7, 19, Some(1), &reply(19, 1), 1).await.unwrap();
        }
        let rows = load(&path, 7).await.unwrap().rows;
        let reply = |id: i64| rows.iter().find(|r| r.id == id).unwrap().reply_to;
        assert_eq!(reply(16), None);
        assert_eq!(reply(17), Some(16));
        assert_eq!(reply(18), None);
        assert_eq!(reply(19), Some(1));
        assert_eq!(detail(&path, 7, 16).await.unwrap().unwrap().reply_to, None);
        assert_eq!(detail(&path, 7, 18).await.unwrap().unwrap().reply_to, None);
        assert_eq!(
            detail(&path, 7, 17).await.unwrap().unwrap().reply_to,
            Some(16)
        );
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

    fn attach(v: Value) -> Option<Attachment> {
        attachment_of(v.as_object().unwrap())
    }

    #[test]
    fn every_kind_of_media_the_export_writes_becomes_something_to_draw() {
        let visual = |v: Value| match attach(v) {
            Some(Attachment::Visual { kind, size, .. }) => (kind, size),
            other => panic!("not a picture: {other:?}"),
        };
        assert_eq!(
            visual(json!({"photo": "photos/a.jpg", "width": 800, "height": 600})),
            (Visual::Photo, [800, 600])
        );
        // Skipped by the export still says what it was.
        assert_eq!(
            visual(
                json!({"photo": "(File not included. Change data exporting settings to download.)"})
            )
            .0,
            Visual::Photo
        );
        for (media_type, kind) in [
            ("sticker", Visual::Sticker),
            ("video_file", Visual::Video),
            ("animation", Visual::Gif),
            ("video_message", Visual::Round),
        ] {
            assert_eq!(
                visual(json!({"file": "x", "media_type": media_type})).0,
                kind
            );
        }
        assert_eq!(
            visual(json!({"file": "a.png", "mime_type": "image/png"})).0,
            Visual::Image
        );
        assert_eq!(
            attach(json!({"file": "v.ogg", "media_type": "voice_message", "duration_seconds": 42})),
            Some(Attachment::Voice { duration: Some(42) })
        );
        assert!(matches!(
            attach(json!({"file": "a.mp3", "media_type": "audio_file", "title": "Pesma", "performer": "Bend"})),
            Some(Attachment::Audio { title, performer, .. }) if title == "Pesma" && performer == "Bend"
        ));
        assert_eq!(
            attach(json!({"file": "f", "file_name": "plan.pdf", "file_size": 2048})),
            Some(Attachment::Document {
                name: "plan.pdf".into(),
                size: Some(2048),
                ext: "PDF".into()
            })
        );
        match attach(
            json!({"poll": {"question": "Kad?", "closed": true, "total_voters": 3,
            "answers": [{"text": "sutra", "voters": 2, "chosen": true}, {"text": "nikad", "voters": 1, "chosen": false}]}}),
        ) {
            Some(Attachment::Poll(p)) => {
                assert_eq!(p.question, "Kad?");
                assert!(p.closed);
                assert_eq!(p.total, 3);
                assert_eq!(p.answers.len(), 2);
                assert!(p.answers[0].chosen && p.answers[0].voters == 2);
            }
            other => panic!("not a poll: {other:?}"),
        }
        assert!(matches!(
            attach(json!({"location_information": {"latitude": 44.8, "longitude": 20.4},
                "place_name": "Trg", "address": "Beograd"})),
            Some(Attachment::Location { title, address, lat, live: false, .. })
                if title == "Trg" && address == "Beograd" && lat == "44.8"
        ));
        assert!(matches!(
            attach(json!({"contact_information": {"first_name": "Ana", "last_name": "B", "phone_number": "+381"}})),
            Some(Attachment::Contact { name, phone, .. }) if name == "Ana B" && phone == "+381"
        ));
        assert!(matches!(
            attach(json!({"game_title": "Igra", "game_description": "opis"})),
            Some(Attachment::Game { title, .. }) if title == "Igra"
        ));
        assert!(matches!(
            attach(json!({"invoice_information": {"title": "Račun", "amount": 1250, "currency": "EUR"}})),
            Some(Attachment::Invoice { amount, .. }) if amount == "12.50 EUR"
        ));
        assert_eq!(
            attach(json!({"dice_emoji": "🎲", "dice_value": 5})),
            Some(Attachment::Dice {
                emoji: "🎲".into(),
                value: 5
            })
        );
        assert_eq!(attach(json!({"text": "plain"})), None);
    }

    #[test]
    fn reactions_keep_their_emoji_and_count_and_a_custom_one_gets_a_glyph() {
        let p = json!({"reactions": [
            {"type": "emoji", "count": 3, "emoji": "❤"},
            {"type": "custom_emoji", "count": 1, "document_id": "5"}
        ]});
        let r = reactions_of(p.as_object().unwrap());
        assert_eq!(r[0], ("❤".to_string(), 3));
        assert_eq!(r[1].1, 1);
        assert!(!r[1].0.is_empty());
    }

    #[tokio::test]
    async fn a_picture_is_the_file_itself_then_its_thumbnail_and_never_a_video() {
        let path = tmp("pictures");
        {
            let s = tgx_archive::Store::open(&path).await.unwrap();
            s.merge(7, 1, None, &msg(1, json!("")), 100).await.unwrap();
            s.merge(7, 2, None, &msg(2, json!("")), 100).await.unwrap();
            s.merge(7, 3, None, &msg(3, json!("")), 100).await.unwrap();
            // A photo: its own bytes.
            s.put_blob(10, "photos", "image/jpeg", None, b"photo", 1)
                .await
                .unwrap();
            s.link_media(7, 1, "photo", 10, "photos", "photos/p.jpg")
                .await
                .unwrap();
            // An animated sticker: its bytes first, Telegram's thumbnail next.
            s.put_blob(11, "stickers", "application/x-tgsticker", None, b"tgs", 1)
                .await
                .unwrap();
            s.put_thumb(11, "stickers", b"thumb").await.unwrap();
            s.link_media(7, 2, "file", 11, "stickers", "stickers/s.tgs")
                .await
                .unwrap();
            // A video: only its thumbnail, never the film.
            s.put_blob(12, "video_files", "video/mp4", None, b"film", 1)
                .await
                .unwrap();
            s.put_thumb(12, "video_files", b"frame").await.unwrap();
            s.link_media(7, 3, "file", 12, "video_files", "video_files/v.mp4")
                .await
                .unwrap();
        }
        let got = picture_bytes(&path, 7, vec![1, 2, 3, 4]).await.unwrap();
        let got: Vec<(i64, Vec<Vec<u8>>, bool)> = got
            .into_iter()
            .map(|p| (p.id, p.candidates, p.has_file))
            .collect();
        assert_eq!(got[0], (1, vec![b"photo".to_vec()], true));
        assert_eq!(got[1], (2, vec![b"tgs".to_vec(), b"thumb".to_vec()], true));
        assert_eq!(got[2], (3, vec![b"frame".to_vec()], true));
        assert_eq!(
            got[3],
            (4, vec![], false),
            "a message with no media has none"
        );
        // A click opens the file, not the thumbnail.
        let file = main_file(&path, 7, 3).await.unwrap().unwrap();
        assert_eq!(file.file_id, 12);
        assert!(main_file(&path, 7, 4).await.unwrap().is_none());
    }

    #[test]
    fn a_name_with_nothing_left_after_cleaning_gets_one() {
        assert_eq!(safe_name("photos/", 5), "file_5");
        assert_eq!(safe_name("a:b?.pdf", 5), "a_b_.pdf");
        assert_eq!(safe_name("photos/photo_1.jpg", 5), "photo_1.jpg");
        assert_eq!(safe_name(r"..\..\Startup\x.bat", 5), "x.bat");
        assert_eq!(safe_name("..", 5), "file_5");
        assert_eq!(safe_name(" .hidden. ", 5), "hidden");
    }

    #[test]
    fn a_device_name_is_written_as_a_file() {
        // From a database somebody handed over: `NUL.jpg` opened for writing
        // on Windows is the null device, and the bytes go nowhere.
        assert_eq!(safe_name("NUL.jpg", 5), "_NUL.jpg");
        assert_eq!(safe_name(r"x\con", 5), "_con");
        assert_eq!(safe_name("com1.tar.gz", 5), "_com1.tar.gz");
        assert_eq!(safe_name("CONTRACT.pdf", 5), "CONTRACT.pdf");
    }
}
