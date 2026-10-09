//! An export that is a SQLite file rather than a folder.
//!
//! TelegramExporter gained a third output format: one `telegram.sqlite` that
//! accumulates across runs instead of a fresh folder per pass. A run with only
//! that format on writes **no `result.json` at all**, so such an export was
//! invisible to an analyser that looks for one — `load` failed with "No
//! result.json under …" and there was nothing else to try.
//!
//! **This is a different container, not a different format.** `messages.payload`
//! is the same message map `result.json` carries, and `topics.head` is the same
//! header minus the three keys the tables hold instead (`name`, `type`, `id`).
//! So this crate reads rows, rebuilds those maps, and hands them to
//! [`tga_read::one`] and [`tga_read::header_topic`]. Nothing about a message,
//! a clock or a thread is decided twice.
//!
//! What the database has that a folder never did — `deleted_seen` on a message
//! Telegram no longer returns, the `versions` of an edited one, and the `runs`
//! table — is read but not yet carried into the model. See `PLAN.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value};
use tga_read::{Export, Progress};

/// What the exporter named the file while it kept every chat in one. It now
/// writes `<title>/<title>.sqlite` per chat; see [`find`].
pub const FILE_NAME: &str = "telegram.sqlite";

/// The only shape this build knows how to read.
///
/// Refused rather than guessed at, matching the exporter's own rule: a file
/// from a later build may have moved a column this reader depends on, and a
/// report drawn from a misread table is worse than no report.
pub const SCHEMA_VERSION: i64 = 1;

/// One chat in the file.
#[derive(Debug, Clone)]
pub struct Chat {
    pub id: i64,
    pub title: String,
    pub kind: String,
    pub messages: usize,
}

/// The database an export root holds, or `None` if it holds none.
///
/// Takes a path to the file itself as readily as the folder around it, because
/// both are things somebody will drop on the window: the exporter writes
/// `<output_dir>/telegram.sqlite`, and `<output_dir>` is what its own settings
/// call the export.
///
/// Only the root and its immediate children are searched. Deeper would mean
/// walking a media tree to find a file that is always at the top.
///
/// **The exporter stopped writing one flat file.** It now writes one database
/// per chat, each in a folder of its own — `<output_dir>/<title>/<title>.sqlite`
/// — so the folder its settings call the export holds no database at the top at
/// all. One level down is therefore searched too, but only answers when it
/// finds exactly one: a folder of sixteen chats is not one export, and picking
/// the alphabetically first would write a report about a chat nobody chose.
/// [`find_all`] is the question for that folder.
pub fn find(root: &Path) -> Option<PathBuf> {
    if root.is_file() {
        return is_sqlite(root).then(|| root.to_path_buf());
    }
    let named = root.join(FILE_NAME);
    if named.is_file() {
        return Some(named);
    }
    // Sorted so a folder with two of them picks the same one every run rather
    // than whatever the directory happened to yield first.
    if let Some(first) = sqlite_files(root).into_iter().next() {
        return Some(first);
    }
    let mut below = subfolders(root).into_iter().flat_map(|d| chat_files(&d));
    match (below.next(), below.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// Every database under an export root: the root's own, and one in each folder
/// directly inside it — the exporter's per-chat layout.
///
/// A flat `telegram.sqlite` from before the split is listed beside the per-chat
/// files rather than in place of them. `tgx split` renames it to `.split` only
/// once every chat has been copied out, so until then it is still somebody's
/// only copy of a chat, and leaving it out would hide that.
pub fn find_all(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return if is_sqlite(root) {
            vec![root.to_path_buf()]
        } else {
            Vec::new()
        };
    }
    let mut found = sqlite_files(root);
    for dir in subfolders(root) {
        found.extend(chat_files(&dir));
    }
    found
}

/// The per-chat databases in one folder: `.sqlite` only, the extension the
/// exporter writes.
///
/// **Narrower than the top level on purpose.** One folder down is where a
/// folder export keeps its topics, and a topic folder holding somebody's
/// attachment called `contacts.db` must not turn a folder export into a
/// database that then fails on its schema.
fn chat_files(dir: &Path) -> Vec<PathBuf> {
    sqlite_files(dir)
        .into_iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sqlite"))
        .collect()
}

/// The last time anything was written to a database, or `None` if it cannot
/// be told.
///
/// **The `-wal` file is read as well as the database.** A database in WAL mode
/// takes its writes in the side file and only folds them back at a checkpoint,
/// so the main file's time can sit hours behind a sync that has finished.
/// Opening read-only, which is all this crate does, changes neither.
pub fn modified(path: &Path) -> Option<std::time::SystemTime> {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    [path.to_path_buf(), PathBuf::from(wal)]
        .iter()
        .filter_map(|p| p.metadata().and_then(|m| m.modified()).ok())
        .max()
}

fn sqlite_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_sqlite(p))
        .collect();
    found.sort();
    found
}

fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    found.sort();
    found
}

fn is_sqlite(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("sqlite") | Some("sqlite3") | Some("db")
    )
}

/// Open read-only.
///
/// **Read-only is the point, not a precaution.** The window and the exporter
/// can be pointed at one file at the same time, and opening a SQLite database
/// for writing takes a lock and can create `-wal` and `-shm` files beside it.
/// An analyser has no business doing either to somebody's archive.
fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {}", path.display()))?;

    let found: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .with_context(|| format!("{} is not a TelegramExporter database", path.display()))?
        .trim()
        .parse()
        .unwrap_or(-1);
    if found != SCHEMA_VERSION {
        bail!(
            "{} is schema version {found}; this build reads version {SCHEMA_VERSION}",
            path.display()
        );
    }
    Ok(conn)
}

/// Every chat in the file, most messages first.
pub fn chats(path: &Path) -> Result<Vec<Chat>> {
    chats_in(&open(path)?)
}

/// Just the names, without counting anything.
///
/// **Separate from [`chats`] because the window calls this on every keystroke**
/// in the path field. Counting messages per chat is an index scan over the
/// whole table; on a 450,000-message archive that is long enough to feel in a
/// text field, and the field only needs to say what the file is.
pub fn chat_titles(path: &Path) -> Result<Vec<(i64, String)>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare("SELECT id, title FROM chats ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn chats_in(conn: &Connection) -> Result<Vec<Chat>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.title, c.type, COUNT(m.id)
           FROM chats c LEFT JOIN messages m ON m.chat_id = c.id
          GROUP BY c.id ORDER BY COUNT(m.id) DESC, c.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Chat {
            id: r.get(0)?,
            title: r.get(1)?,
            kind: r.get(2)?,
            messages: r.get::<_, i64>(3)?.max(0) as usize,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Read one chat out of the database into an [`Export`].
///
/// `wanted` picks the chat by id or by title when the file holds more than one.
/// The exporter is moving to one database per chat, so the ordinary case is a
/// file with exactly one and no argument; a file with several is read as its
/// largest, and the caller is expected to say which others it passed over.
pub fn load(
    path: &Path,
    wanted: Option<&str>,
    mut progress: Option<Progress<'_>>,
) -> Result<Export> {
    let conn = open(path)?;
    let all = chats_in(&conn)?;
    anyhow::ensure!(!all.is_empty(), "{} holds no chats", path.display());

    let chat = match wanted {
        None => all[0].clone(),
        Some(want) => all
            .iter()
            .find(|c| c.id.to_string() == want || c.title.eq_ignore_ascii_case(want))
            .cloned()
            .with_context(|| {
                let names: Vec<String> = all
                    .iter()
                    .map(|c| format!("{} ({})", c.title, c.id))
                    .collect();
                format!(
                    "no chat {want:?} in this database; it holds {}",
                    names.join(", ")
                )
            })?,
    };

    // The folder the file sits in. Nothing is read from it — a database export
    // has no media tree — but `Topic::folder` is what an attachment path is
    // resolved against, and pointing it at the database's own folder makes a
    // missing file miss honestly instead of resolving somewhere absurd.
    let folder = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let mut export = Export {
        root: path.to_path_buf(),
        // The chat's real title, which a folder export could only ever get from
        // whatever the folder happened to be called.
        name: chat.title.clone(),
        ..Default::default()
    };

    let heads = topics_of(&conn, chat.id)?;
    let total = heads.len();
    for (index, (topic_id, head)) in heads.into_iter().enumerate() {
        // Back into the map a `result.json` would have carried: the head holds
        // everything except the three keys the `chats` and `topics` rows own.
        let mut header = head;
        header.insert(
            "name".into(),
            Value::String(title_of(&conn, chat.id, topic_id)?),
        );
        header.insert("type".into(), Value::String(chat.kind.clone()));
        header.insert("id".into(), Value::Number(chat.id.into()));
        let header = Value::Object(header);

        let mut topic = tga_read::header_topic(&header, index, folder.clone());
        if export.members_count.is_none() {
            export.members_count = header.get("members_count").and_then(Value::as_i64);
        }

        // Ordered by id, which is the order `result.json` lists them in and
        // what `Topic::root` and the reply rule both assume.
        //
        // **`deleted_seen` is not filtered on.** Keeping a message Telegram
        // has dropped is the whole reason this format exists; leaving it out
        // of the figures would make the archive agree with Telegram, which is
        // the one thing it is meant not to do. A message that was said is part
        // of the history whether or not it can still be fetched. The count is
        // available separately, through `deleted`, so a caller can say how
        // many of the figures rest on messages nobody can see any more.
        let mut stmt = conn.prepare(
            "SELECT payload FROM messages
              WHERE chat_id = ?1 AND topic_id = ?2
              ORDER BY id",
        )?;
        let mut rows = stmt.query(rusqlite::params![chat.id, topic_id])?;
        while let Some(row) = rows.next()? {
            let text: String = row.get(0)?;
            let Ok(raw) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(msg) = tga_read::one(&raw, index, topic.root) {
                export.msgs.push(msg);
                topic.messages += 1;
            }
        }

        let label = topic.name.clone();
        export.topics.push(topic);
        if let Some(report) = progress.as_deref_mut() {
            report(index + 1, total, &label);
        }
    }

    (export.roster, export.roster_complete, export.roster_capped) = roster(&conn, chat.id);

    export
        .msgs
        .sort_by(|a, b| a.unix.cmp(&b.unix).then_with(|| a.id.cmp(&b.id)));
    Ok(export)
}

/// `(topic_id, head)` for one chat, in the order the topics were created.
fn topics_of(conn: &Connection, chat_id: i64) -> Result<Vec<(i64, Map<String, Value>)>> {
    let mut stmt = conn.prepare("SELECT id, head FROM topics WHERE chat_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([chat_id], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, head) = row?;
        let head = match serde_json::from_str::<Value>(&head) {
            Ok(Value::Object(map)) => map,
            // A head that will not parse costs the topic its metadata, not its
            // messages. The `topics` row still names it and the payloads are
            // untouched.
            _ => Map::new(),
        };
        out.push((id, head));
    }
    Ok(out)
}

fn title_of(conn: &Connection, chat_id: i64, topic_id: i64) -> Result<String> {
    Ok(conn.query_row(
        "SELECT title FROM topics WHERE chat_id = ?1 AND id = ?2",
        rusqlite::params![chat_id, topic_id],
        |r| r.get(0),
    )?)
}

/// The roster, out of the `participants` table.
///
/// **`capped` is always `None` here, and that is the honest answer.** The
/// exporter writes `complete` and `capped` into `participants.json`, but only
/// `members_complete` reaches the topic head, so a database has no record of
/// whether the member fetch stopped at a limit. `roster_capped` is an `Option`
/// for exactly this: absent means the export never said, which is a different
/// fact from "it said no".
fn roster(conn: &Connection, chat_id: i64) -> (Vec<tga_read::Member>, Option<bool>, Option<bool>) {
    let complete = head_flag(conn, chat_id, "members_complete");

    let Ok(mut stmt) =
        conn.prepare("SELECT info FROM participants WHERE chat_id = ?1 ORDER BY peer")
    else {
        return (Vec::new(), complete, None);
    };
    let Ok(rows) = stmt.query_map([chat_id], |r| r.get::<_, String>(0)) else {
        return (Vec::new(), complete, None);
    };

    let people = rows
        .flatten()
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .filter_map(|entry| tga_read::member(&entry))
        .collect();
    (people, complete, None)
}

/// One boolean out of any of a chat's topic heads.
///
/// Any of them: the exporter writes the same roster block into every topic's
/// head of a chat, so the first that carries the key answers for all.
fn head_flag(conn: &Connection, chat_id: i64, key: &str) -> Option<bool> {
    let mut stmt = conn
        .prepare("SELECT head FROM topics WHERE chat_id = ?1 ORDER BY id")
        .ok()?;
    let rows = stmt.query_map([chat_id], |r| r.get::<_, String>(0)).ok()?;
    for text in rows.flatten() {
        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) {
            match map.get(key) {
                None | Some(Value::Null) => continue,
                Some(v) => return Some(v != &Value::Bool(false)),
            }
        }
    }
    None
}

/// What the file says about messages Telegram no longer returns.
///
/// Not in the [`Export`] yet — the model has nowhere to put it and the report
/// has no section for it. Exposed so a caller can say the number out loud
/// rather than let it pass unmentioned, because "17 of these were deleted and
/// kept" is the one thing this format knows that a folder never could.
pub fn deleted(path: &Path, chat_id: i64) -> Result<usize> {
    let conn = open(path)?;
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE chat_id = ?1 AND deleted_seen IS NOT NULL",
        [chat_id],
        |r| r.get(0),
    )?;
    Ok(n.max(0) as usize)
}

/// How many stored messages have an earlier version kept, and how many
/// versions there are in total.
pub fn edits(path: &Path, chat_id: i64) -> Result<(usize, usize)> {
    let conn = open(path)?;
    let (msgs, rows): (i64, i64) = conn.query_row(
        "SELECT COUNT(DISTINCT id), COUNT(*) FROM versions WHERE chat_id = ?1",
        [chat_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((msgs.max(0) as usize, rows.max(0) as usize))
}

/// The bytes of one stored file, if the database has them.
///
/// Keyed `(file_id, kind)` because that is the shipped schema's primary key on
/// `blobs`; `media` is what maps a message to the pair.
pub fn blob(path: &Path, file_id: i64, kind: &str) -> Result<Option<Vec<u8>>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare("SELECT bytes FROM blobs WHERE file_id = ?1 AND kind = ?2")?;
    let mut rows = stmt.query(rusqlite::params![file_id, kind])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

/// `(message_id, role) -> (file_id, kind)` for one chat.
///
/// The link table, read whole. It is one row per message that carries media —
/// 263 rows for a 7,077-message archive — so a map costs less than a query per
/// message.
///
/// Unused so far: reading an attachment out of `blobs` needs `tga-docs` to
/// take bytes where it now takes a path. Here because it is the half that
/// belongs to this crate, and because the key is `(file_id, kind)` — the
/// shipped schema's, which is *not* the `(family, file_id)` the exporter's own
/// `DATABASE.md` specifies.
pub fn media_index(path: &Path, chat_id: i64) -> Result<HashMap<(i64, String), (i64, String)>> {
    let conn = open(path)?;
    let mut stmt =
        conn.prepare("SELECT message_id, role, file_id, kind FROM media WHERE chat_id = ?1")?;
    let rows = stmt.query_map([chat_id], |r| {
        Ok((
            (r.get::<_, i64>(0)?, r.get::<_, String>(1)?),
            (r.get::<_, i64>(2)?, r.get::<_, String>(3)?),
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A folder export and a database holding **the same JSON**, so the two
    /// readers can be compared on identical input.
    ///
    /// This is the test the crate exists for. The claim in the module doc is
    /// that a database is a different container and not a different format; if
    /// that is true the two `Export`s agree, and if it ever stops being true
    /// this is where it shows.
    struct Both {
        folder: PathBuf,
        db: PathBuf,
    }

    fn header() -> Value {
        json!({
            "name": "ćaskanje",
            "type": "private_supergroup",
            "id": 3586682625i64,
            "topic_id": 12,
            "topic_created": "2025-12-14T16:41:26",
            "members_count": 45,
            "members_complete": true
        })
    }

    fn messages() -> Vec<Value> {
        vec![
            // The topic's own root. Every top-level message in a forum topic
            // is sent as a reply to it, and both readers must drop that.
            json!({"id": 12, "type": "message", "date": "2025-12-14T16:41:26",
                   "date_unixtime": "1765727286", "from": "Ana", "from_id": "user1",
                   "text": "pocetak"}),
            json!({"id": 13, "type": "message", "date": "2025-12-14T16:50:59",
                   "date_unixtime": "1765727459", "from": "Bob", "from_id": "user2",
                   "reply_to_message_id": 12, "text": "top level, not a reply"}),
            json!({"id": 14, "type": "message", "date": "2025-12-14T16:52:00",
                   "date_unixtime": "1765727520", "from": "Ana", "from_id": "user1",
                   "reply_to_message_id": 13, "text": "a real reply",
                   "file": "files/zapisnik.docx", "file_name": "zapisnik 06.07.2026.docx",
                   "file_size": 19053, "mime_type":
                   "application/vnd.openxmlformats-officedocument.wordprocessingml.document"}),
            json!({"id": 15, "type": "service", "date": "2025-12-14T17:00:00",
                   "date_unixtime": "1765728000", "actor": "Ana", "actor_id": "user1",
                   "action": "invite_members", "text": ""}),
        ]
    }

    fn members() -> Vec<Value> {
        vec![
            json!({"id": "user1", "name": "Ana", "username": "@ana", "bot": false}),
            json!({"id": "user2", "name": "Bob", "role": "admin", "bot": false}),
        ]
    }

    fn build(tag: &str) -> Both {
        let base = std::env::temp_dir().join(format!("tga-db-test-{tag}"));
        let _ = std::fs::remove_dir_all(&base);
        let folder = base.join("folder");
        std::fs::create_dir_all(&folder).expect("temp dir");

        let mut body = header();
        body["messages"] = Value::Array(messages());
        std::fs::write(
            folder.join("result.json"),
            serde_json::to_string_pretty(&body).expect("json"),
        )
        .expect("write result.json");
        std::fs::write(
            folder.join("participants.json"),
            serde_json::to_string_pretty(&json!({"members": members(), "complete": true}))
                .expect("json"),
        )
        .expect("write participants.json");

        // The head is the header minus the three keys the tables own, which is
        // exactly what the exporter stores.
        let mut head = header();
        for key in ["name", "type", "id"] {
            head.as_object_mut().expect("object").remove(key);
        }

        let db = base.join(FILE_NAME);
        let conn = Connection::open(&db).expect("create database");
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE chats (id INTEGER PRIMARY KEY, title TEXT NOT NULL, type TEXT NOT NULL,
                                 first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL);
             CREATE TABLE topics (chat_id INTEGER NOT NULL, id INTEGER NOT NULL, title TEXT NOT NULL,
                                  head TEXT NOT NULL, first_seen INTEGER NOT NULL,
                                  last_seen INTEGER NOT NULL, PRIMARY KEY (chat_id, id));
             CREATE TABLE messages (chat_id INTEGER NOT NULL, id INTEGER NOT NULL,
                                    topic_id INTEGER NOT NULL, payload TEXT NOT NULL,
                                    type TEXT NOT NULL, date_unixtime INTEGER NOT NULL,
                                    from_id TEXT, version INTEGER NOT NULL DEFAULT 1,
                                    first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL,
                                    deleted_seen INTEGER, PRIMARY KEY (chat_id, id));
             CREATE TABLE versions (chat_id INTEGER NOT NULL, id INTEGER NOT NULL,
                                    version INTEGER NOT NULL, payload TEXT NOT NULL,
                                    seen INTEGER NOT NULL, PRIMARY KEY (chat_id, id, version));
             CREATE TABLE participants (chat_id INTEGER NOT NULL, peer TEXT NOT NULL,
                                        info TEXT NOT NULL, first_seen INTEGER NOT NULL,
                                        last_seen INTEGER NOT NULL, PRIMARY KEY (chat_id, peer));
             CREATE TABLE blobs (file_id INTEGER NOT NULL, kind TEXT NOT NULL, mime TEXT NOT NULL,
                                 file_name TEXT, size INTEGER NOT NULL, bytes BLOB NOT NULL,
                                 first_seen INTEGER NOT NULL, PRIMARY KEY (file_id, kind));
             CREATE TABLE media (chat_id INTEGER NOT NULL, message_id INTEGER NOT NULL,
                                 role TEXT NOT NULL, file_id INTEGER NOT NULL, kind TEXT NOT NULL,
                                 path TEXT NOT NULL, PRIMARY KEY (chat_id, message_id, role));",
        )
        .expect("schema");
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', '1')",
            [],
        )
        .expect("meta");
        conn.execute(
            "INSERT INTO chats VALUES (3586682625, 'UA KOLAB', 'private_supergroup', 0, 0)",
            [],
        )
        .expect("chat");
        conn.execute(
            "INSERT INTO topics VALUES (3586682625, 12, 'ćaskanje', ?1, 0, 0)",
            [head.to_string()],
        )
        .expect("topic");
        for m in messages() {
            conn.execute(
                "INSERT INTO messages (chat_id, id, topic_id, payload, type, date_unixtime,
                                       from_id, first_seen, last_seen)
                 VALUES (3586682625, ?1, 12, ?2, ?3, ?4, ?5, 0, 0)",
                rusqlite::params![
                    m["id"].as_i64().expect("id"),
                    m.to_string(),
                    m["type"].as_str().unwrap_or(""),
                    m["date_unixtime"].as_str().unwrap_or("0"),
                    m.get("from_id").and_then(Value::as_str),
                ],
            )
            .expect("message");
        }
        for p in members() {
            conn.execute(
                "INSERT INTO participants VALUES (3586682625, ?1, ?2, 0, 0)",
                rusqlite::params![p["id"].as_str().expect("peer"), p.to_string()],
            )
            .expect("participant");
        }
        drop(conn);

        Both { folder, db }
    }

    #[test]
    fn a_database_and_a_folder_of_the_same_json_read_the_same() {
        let both = build("same");
        let from_folder = tga_read::load(&both.folder, None).expect("folder");
        let from_db = load(&both.db, None, None).expect("database");

        assert_eq!(from_db.msgs.len(), from_folder.msgs.len(), "message count");
        for (a, b) in from_db.msgs.iter().zip(&from_folder.msgs) {
            assert_eq!(a.id, b.id, "order");
            assert_eq!(a.when, b.when, "wall clock, #{}", a.id);
            assert_eq!(a.unix, b.unix, "monotonic clock, #{}", a.id);
            assert_eq!(a.sender, b.sender, "peer key, #{}", a.id);
            assert_eq!(a.text, b.text, "text, #{}", a.id);
            assert_eq!(a.service, b.service, "service, #{}", a.id);
            assert_eq!(a.file_name, b.file_name, "file name, #{}", a.id);
            // The one that would break silently and change every median.
            assert_eq!(a.reply_to, b.reply_to, "reply, #{}", a.id);
        }

        assert_eq!(from_db.topics.len(), 1);
        assert_eq!(from_db.topics[0].name, from_folder.topics[0].name);
        assert_eq!(from_db.topics[0].root, from_folder.topics[0].root);
        assert_eq!(from_db.topics[0].chat_id, from_folder.topics[0].chat_id);
        assert_eq!(from_db.topics[0].chat_type, from_folder.topics[0].chat_type);
        assert_eq!(from_db.topics[0].created, from_folder.topics[0].created);
        assert_eq!(from_db.topics[0].messages, from_folder.topics[0].messages);
        assert_eq!(from_db.members_count, from_folder.members_count);

        let keys = |e: &Export| e.roster.iter().map(|m| m.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&from_db), keys(&from_folder), "roster");
        let ana = |e: &Export| e.roster.iter().find(|m| m.key == "user1").cloned();
        assert_eq!(
            ana(&from_db).map(|m| m.username),
            ana(&from_folder).map(|m| m.username),
            "the @ strip"
        );
        assert_eq!(from_db.roster_complete, from_folder.roster_complete);
    }

    #[test]
    fn the_export_is_named_for_the_chat_not_for_a_folder() {
        // The one place the two readers deliberately differ. A folder export
        // can only be named after whatever the folder was called; the database
        // knows the chat's real title.
        let both = build("named");
        assert_eq!(
            load(&both.db, None, None).expect("database").name,
            "UA KOLAB"
        );
        assert_eq!(
            tga_read::load(&both.folder, None).expect("folder").name,
            "folder"
        );
    }

    #[test]
    fn a_deleted_message_is_still_in_the_figures_and_still_counted() {
        let both = build("deleted");
        let conn = Connection::open(&both.db).expect("open");
        conn.execute("UPDATE messages SET deleted_seen = 99 WHERE id = 13", [])
            .expect("mark");
        drop(conn);

        // Keeping what Telegram dropped is the whole point of the format.
        // Filtering it out here would make the archive agree with Telegram.
        let export = load(&both.db, None, None).expect("database");
        assert_eq!(export.msgs.len(), 4);
        assert!(export.msgs.iter().any(|m| m.id == 13));
        assert_eq!(deleted(&both.db, 3586682625).expect("count"), 1);
    }

    #[test]
    fn a_chat_is_picked_by_id_or_by_title_and_a_bad_one_names_the_others() {
        let both = build("pick");
        assert_eq!(
            load(&both.db, Some("3586682625"), None)
                .expect("by id")
                .name,
            "UA KOLAB"
        );
        assert_eq!(
            load(&both.db, Some("ua kolab"), None)
                .expect("by title")
                .name,
            "UA KOLAB"
        );
        let err = load(&both.db, Some("nope"), None).expect_err("no such chat");
        assert!(
            err.to_string().contains("UA KOLAB"),
            "the error should say what is in the file: {err}"
        );
    }

    #[test]
    fn a_file_from_a_later_schema_is_refused_rather_than_misread() {
        let both = build("schema");
        let conn = Connection::open(&both.db).expect("open");
        conn.execute(
            "UPDATE meta SET value = '2' WHERE key = 'schema_version'",
            [],
        )
        .expect("bump");
        drop(conn);
        let err = load(&both.db, None, None).expect_err("refused");
        assert!(err.to_string().contains("schema version 2"), "{err}");
    }

    #[test]
    fn the_database_is_found_from_the_folder_around_it_or_named_directly() {
        let both = build("find");
        let parent = both.db.parent().expect("parent");
        assert_eq!(find(parent).as_deref(), Some(both.db.as_path()));
        assert_eq!(find(&both.db).as_deref(), Some(both.db.as_path()));
        // A folder export is not a database, and must not be mistaken for one.
        assert_eq!(find(&both.folder), None);
    }

    #[test]
    fn the_per_chat_layout_is_found_one_folder_down_and_only_when_unambiguous() {
        // `<output_dir>/<title>/<title>.sqlite`, which is what the exporter
        // writes now. The output folder holds no database at its own top.
        let base = std::env::temp_dir().join("tga-db-test-layout");
        let _ = std::fs::remove_dir_all(&base);
        let one = base.join("UA KOLAB").join("UA KOLAB.sqlite");
        std::fs::create_dir_all(one.parent().expect("parent")).expect("mkdir");
        std::fs::write(&one, b"").expect("touch");

        assert_eq!(find(&base).as_deref(), Some(one.as_path()));
        assert_eq!(find_all(&base), vec![one.clone()]);

        // A second chat makes the folder a collection, not an export: `find`
        // stops guessing and `find_all` lists both.
        let two = base.join("Telegram").join("Telegram.sqlite");
        std::fs::create_dir_all(two.parent().expect("parent")).expect("mkdir");
        std::fs::write(&two, b"").expect("touch");
        assert_eq!(find(&base), None);
        assert_eq!(find_all(&base), vec![two.clone(), one.clone()]);

        // A flat file from before the split is still a database and still
        // listed; `tgx split`'s leftover `.split` is not.
        std::fs::write(base.join(FILE_NAME), b"").expect("touch");
        std::fs::write(base.join("old.sqlite.split"), b"").expect("touch");
        assert_eq!(find_all(&base).len(), 3);
        assert_eq!(find(&base), Some(base.join(FILE_NAME)));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_database_is_dated_by_its_wal_when_that_is_newer() {
        let base = std::env::temp_dir().join("tga-db-test-modified");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("mkdir");
        let db = base.join("a.sqlite");
        std::fs::write(&db, b"").expect("touch");
        let main = modified(&db).expect("dated");

        let wal = base.join("a.sqlite-wal");
        let file = std::fs::File::create(&wal).expect("wal");
        let later = main + std::time::Duration::from_secs(3600);
        file.set_modified(later).expect("set time");
        drop(file);
        assert_eq!(modified(&db), Some(later));
        assert_eq!(modified(&base.join("missing.sqlite")), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn opening_a_database_leaves_no_wal_or_shm_beside_it() {
        // Read-only, because the exporter may be writing this very file and an
        // analyser has no business taking a lock on somebody's archive.
        let both = build("readonly");
        let _ = load(&both.db, None, None).expect("database");
        let parent = both.db.parent().expect("parent");
        for tail in ["telegram.sqlite-wal", "telegram.sqlite-shm"] {
            assert!(!parent.join(tail).exists(), "{tail} was created");
        }
    }
}
