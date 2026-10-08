//! The SQLite store: chats, topics, messages, versions, blobs, participants,
//! runs.
//!
//! Everything here takes values — a serialised message map, a file id, a
//! timestamp — and nothing here knows what Telegram is. That is what lets
//! `tgx-parity`'s archive leg merge a real Desktop export into a store twice
//! and read Desktop's own bytes back out with no connection.
//!
//! **Every method is `async` because libsql's API is**, not because anything
//! here is concurrent. There is one connection and one caller.

use libsql::params;
use libsql::params::IntoParams;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The extension a chat's archive takes.
///
/// **One database per chat**, named after the chat — `Dev Team.sqlite` beside
/// `Mum.sqlite` — so the only part of the name this crate gets to decide is the
/// suffix. It was the whole name, `telegram.sqlite`, back when one file held
/// every chat exported into a folder: convenient to open, and a single file
/// holding several people's conversations, which is not what someone exporting
/// one chat asked to produce or to hand on.
///
/// Which chat a file belongs to is still recorded *inside* it — every table is
/// keyed by `chat_id` — so nothing about the schema changed, and a file that
/// somehow ends up holding two chats is readable rather than corrupt. See
/// [`Store::holds_another_chat`], which is how the exporter keeps that from
/// happening quietly.
pub const FILE_EXT: &str = "sqlite";

/// Bumped only when the shape below changes in a way another build would
/// misread. A build refuses a file it does not know rather than guessing.
pub const SCHEMA_VERSION: i64 = 1;

/// The largest file that goes *into* the database.
///
/// **Not SQLite's ceiling**, which is 1 GB. The bytes are read whole into a
/// `Vec<u8>` and handed to libsql as another owned value, so the peak is twice
/// the file — and the machine on the other end of that is a laptop exporting
/// somebody's holiday videos. The folder export's own `size_limit_mb` is the
/// real limit and defaults to 20 MB; this exists for the run that sets it to
/// unlimited, so one 3 GB file cannot take the export down with it. Over this
/// the `media` row is still written, so the attachment is recorded rather than
/// silently forgotten, and the bytes stay in the folder.
pub const MAX_BLOB: i64 = 256 * 1024 * 1024;

/// How long to wait for another process holding the write lock.
///
/// The window and the `tgx` CLI can both be pointed at one export root, and
/// this project has made the two-writers mistake before — a second
/// `SqliteSession` opened on a file the first was still writing. With no busy
/// timeout the loser of that race does not wait; it fails instantly with
/// `SQLITE_BUSY`, in the middle of somebody's export.
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// Keys a later run may overwrite on a message it has already stored.
///
/// **Short on purpose.** Everything else — who sent it, what it replied to,
/// which files hang off it — is a fact about a message that happened, and a
/// re-read that rewrote those would let a converter change history.
///
/// Four of these (`views`, `forwards`, `replies_count`, `pinned`) are keys the
/// converter does not write yet. They are listed anyway because `order.rs`
/// already ranks them, and the day one starts being emitted is not the day
/// anybody will remember this list exists.
pub const VOLATILE: &[&str] = &[
    "edited",
    "edited_unixtime",
    "text",
    "text_entities",
    "reactions",
    "views",
    "forwards",
    "replies_count",
    "pinned",
    "poll",
];

/// Media keys, and the prefix of the strings Desktop writes when it saved no
/// file: *"(File not included…"*, *"(File exceeds maximum size…"*.
///
/// A stored placeholder is not history to be preserved — it is a record that
/// the run declined to fetch something, and the moment a later run does fetch
/// it, the placeholder is simply wrong. So these keys are replaceable **only
/// when what is stored is a placeholder**; a real path is never overwritten,
/// which is what keeps a media replacement on an edited message from rewriting
/// what the message originally carried.
///
/// Without this, raising the size limit and re-syncing puts the bytes in
/// `blobs` and leaves the payload still saying the file was too large.
/// `tgx-tg`'s `the_skip_placeholders_keep_the_shape_the_archive_matches_on`
/// ties the prefix here to `plan::NOT_INCLUDED` and `plan::TOO_LARGE`, which
/// this layer may not import.
const MEDIA_KEYS: &[&str] = &["file", "photo", "thumbnail", "file_size", "photo_file_size"];
const SKIPPED_PREFIX: &str = "(File ";

/// A chat with no topics, and a forum's General topic, are both id 1 — mirrors
/// `tgx_media::topics::GENERAL_TOPIC_ID`, which this layer may not depend on.
/// Only ever the value *inserted* when the caller could not resolve a topic;
/// see [`Store::merge`] for why it never overwrites one.
const NO_TOPIC: i64 = 1;

const SCHEMA: &str = "\
CREATE TABLE chats (id INTEGER PRIMARY KEY, title TEXT NOT NULL, type TEXT NOT NULL,
                    first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL);

CREATE TABLE topics (chat_id INTEGER NOT NULL, id INTEGER NOT NULL, title TEXT NOT NULL,
                     head TEXT NOT NULL,
                     first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL,
                     PRIMARY KEY (chat_id, id));

CREATE TABLE messages (chat_id INTEGER NOT NULL, id INTEGER NOT NULL, topic_id INTEGER NOT NULL,
                       payload TEXT NOT NULL,
                       type TEXT NOT NULL, date_unixtime INTEGER NOT NULL, from_id TEXT,
                       version INTEGER NOT NULL DEFAULT 1,
                       first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL,
                       deleted_seen INTEGER,
                       PRIMARY KEY (chat_id, id));
CREATE INDEX messages_topic ON messages (chat_id, topic_id, id);
CREATE INDEX messages_date  ON messages (chat_id, date_unixtime);
CREATE INDEX messages_from  ON messages (chat_id, from_id);

CREATE TABLE versions (chat_id INTEGER NOT NULL, id INTEGER NOT NULL, version INTEGER NOT NULL,
                       payload TEXT NOT NULL, seen INTEGER NOT NULL,
                       PRIMARY KEY (chat_id, id, version));

CREATE TABLE blobs (file_id INTEGER NOT NULL, kind TEXT NOT NULL, mime TEXT NOT NULL,
                    file_name TEXT, size INTEGER NOT NULL, bytes BLOB NOT NULL,
                    first_seen INTEGER NOT NULL,
                    PRIMARY KEY (file_id, kind));

CREATE TABLE thumbs (file_id INTEGER NOT NULL, kind TEXT NOT NULL, bytes BLOB NOT NULL,
                     PRIMARY KEY (file_id, kind));

CREATE TABLE media (chat_id INTEGER NOT NULL, message_id INTEGER NOT NULL, role TEXT NOT NULL,
                    file_id INTEGER NOT NULL, kind TEXT NOT NULL, path TEXT NOT NULL,
                    PRIMARY KEY (chat_id, message_id, role));

CREATE TABLE participants (chat_id INTEGER NOT NULL, peer TEXT NOT NULL, info TEXT NOT NULL,
                           first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL,
                           PRIMARY KEY (chat_id, peer));

CREATE TABLE runs (id INTEGER PRIMARY KEY AUTOINCREMENT, chat_id INTEGER NOT NULL,
                   started INTEGER NOT NULL, finished INTEGER, mode TEXT NOT NULL,
                   root TEXT,
                   new INTEGER NOT NULL DEFAULT 0, changed INTEGER NOT NULL DEFAULT 0,
                   deleted INTEGER NOT NULL DEFAULT 0, blobs_added INTEGER NOT NULL DEFAULT 0,
                   reached_end INTEGER NOT NULL DEFAULT 0, settings TEXT NOT NULL);
";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Sql(#[from] libsql::Error),
    #[error("database payload: {0}")]
    Json(#[from] serde_json::Error),
    #[error("database file: {0}")]
    Io(#[from] std::io::Error),
    /// A file written by a different build of this program.
    ///
    /// Refused in **both** directions. A newer file read by an older build is
    /// the obvious hazard; an older file silently upgraded by a newer one is
    /// the worse one, because the other machine can then no longer open the
    /// export that was just made on this one.
    #[error(
        "this .sqlite is schema version {found}, this build understands {wanted} — use the \
         build that wrote it, or export to a different folder"
    )]
    Schema { found: i64, wanted: i64 },
    #[error("file {file_id} is {size} bytes, over the database's blob ceiling")]
    TooLarge { file_id: i64, size: i64 },
}

/// What [`Store::merge`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merge {
    /// Not in the database before this call.
    Inserted,
    /// Byte-identical to what was already there.
    Unchanged,
    /// Different; what was there has been kept in `versions`.
    Changed { previous_version: i64 },
}

/// One chat a database records, as [`Store::chats_in`] reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatEntry {
    pub id: i64,
    pub title: String,
    /// When the last run that read to the end finished, unix seconds. `None`
    /// for a chat no run has finished.
    pub synced: Option<i64>,
}

/// One message as the database holds it.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: i64,
    pub topic_id: i64,
    pub payload: Map<String, Value>,
    pub version: i64,
    /// `None` while Telegram still returns it.
    pub deleted_seen: Option<i64>,
}

/// One superseded payload of a message, as [`Store::versions_of`] reads it.
#[derive(Debug, Clone)]
pub struct StoredVersion {
    /// 1 for the payload first stored, counting up. The message's current
    /// payload is `StoredMessage::version`, one past the last of these.
    pub version: i64,
    pub payload: Map<String, Value>,
    /// When a run found the message different and kept this one aside, unix
    /// seconds — the date this version **stopped** being current.
    pub seen: i64,
}

/// One file a message points at, as [`Store::media_of`] reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEntry {
    pub role: String,
    pub file_id: i64,
    pub kind: String,
    /// The name the folder export used, or would have, relative to the topic
    /// folder — `photos/photo_1@18-12-2025_16-00-00.jpg`.
    pub path: String,
    /// The name Telegram gave the file, when it had one. Photos have none.
    pub file_name: Option<String>,
    /// `None` when the bytes are not in the database: over [`MAX_BLOB`], or a
    /// run that recorded the link and failed to read the file back.
    pub size: Option<i64>,
}

/// One message [`Store::search`] matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub id: i64,
    pub topic_id: i64,
    /// The payload's own `date`, Desktop's local `2025-12-18T16:00:00`.
    pub date: String,
    /// A run has seen it gone from Telegram.
    pub deleted: bool,
    /// **The text is only in a superseded version**: edited away, so the
    /// message as it reads now does not say it.
    pub in_earlier_version: bool,
    /// The text that matched — the current one, or the earlier version's when
    /// that is the only one that did. **For a screen, never for the log**: it
    /// is other people's conversation, and the log sits beside the executable.
    pub excerpt: String,
}

/// A message's `text` as one plain string.
///
/// Desktop writes `text` as a bare string when the message has no formatting
/// and as an **array** otherwise — plain strings interleaved with entity
/// objects that carry their own `text` (`{"type": "bold", "text": "…"}`). A
/// search over the raw JSON would match key names and miss a word split across
/// two entities, so both shapes are flattened to what a person reads.
pub fn plain_text(text: &Value) -> String {
    match text {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part {
                Value::String(s) => s.as_str(),
                Value::Object(o) => o.get("text").and_then(Value::as_str).unwrap_or(""),
                _ => "",
            })
            .collect(),
        _ => String::new(),
    }
}

/// [`plain_text`] of a stored payload's `text` key.
fn payload_text(payload: &str) -> String {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|v| v.get("text").map(plain_text))
        .unwrap_or_default()
}

/// Merge a freshly read message into one already stored.
///
/// Starts from `stored` and copies [`VOLATILE`] across, removing a key the
/// fresh payload no longer has. Everything else is the stored message,
/// untouched.
///
/// **This is where the database's one real limitation lives.** A key outside
/// the list keeps whatever the build that first saw the message wrote, for
/// good: a later converter fix never reaches an already-stored message, and a
/// new extension key never appears on an old row. That is the price of not
/// letting a re-read rewrite history, and it is recorded in ROADMAP.
///
/// The result goes back through `order::ordered`, which the json leg proves is
/// a no-op on a real export — so a merged message still emits Desktop's bytes.
pub fn merge_volatile(
    stored: &Map<String, Value>,
    fresh: &Map<String, Value>,
) -> Map<String, Value> {
    let mut out = stored.clone();
    for key in VOLATILE {
        match fresh.get(*key) {
            Some(v) => {
                out.insert((*key).to_string(), v.clone());
            }
            None => {
                out.remove(*key);
            }
        }
    }
    // A media key the earlier run declined to fetch. See [`MEDIA_KEYS`]: the
    // placeholder is a record of a refusal, not of what the message carried,
    // and a run that has now fetched the file must be allowed to say so.
    for key in MEDIA_KEYS {
        let was_skipped = out
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|s| s.starts_with(SKIPPED_PREFIX));
        if !was_skipped {
            continue;
        }
        match fresh.get(*key) {
            Some(v) => {
                out.insert((*key).to_string(), v.clone());
            }
            None => {
                out.remove(*key);
            }
        }
    }
    tgx_format::order::ordered(&out)
}

pub struct Store {
    conn: libsql::Connection,
    /// Held so the connection cannot outlive the handle that made it.
    _db: libsql::Database,
    path: PathBuf,
}

impl Store {
    /// Open `path`, creating the file and the schema if they are not there.
    pub async fn open(path: &Path) -> Result<Self, Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = libsql::Builder::new_local(path).build().await?;
        let conn = db.connect()?;
        conn.busy_timeout(BUSY_TIMEOUT)?;

        // **Read back what the pragma actually did.** WAL is what we want — it
        // lets a reader open the file while an export is writing it — but it
        // is not available everywhere, and the case that matters here is real:
        // the output folder is free text and people point it at a network
        // share. SQLite refuses WAL on most of those and stays in the old
        // journal mode, which is correct, and which we would otherwise be
        // recording in the log as WAL.
        let mut rows = conn.query("PRAGMA journal_mode = WAL", ()).await?;
        let mode = match rows.next().await? {
            Some(row) => row.get::<String>(0).unwrap_or_default(),
            None => String::new(),
        };
        if !mode.eq_ignore_ascii_case("wal") {
            log::warn!(
                "{}: journal mode is {mode:?}, not WAL — a reader cannot open this file while \
                 an export is writing to it (a network share does this)",
                path.display()
            );
        }
        conn.execute_batch("PRAGMA synchronous = NORMAL;").await?;

        let store = Self {
            conn,
            _db: db,
            path: path.to_path_buf(),
        };
        store.ensure_schema().await?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Open an existing database **for reading only**.
    ///
    /// For everything that looks rather than syncs — [`Self::chats_in`], the
    /// viewer, a search. Not [`Store::open`], which creates the schema in
    /// whatever it is handed and switches it to WAL: a viewer pointed at the
    /// wrong file must leave it exactly as it found it, and one opened while an
    /// export is writing must not be the second writer. SQLite enforces the
    /// read-only flag, so a write slipped in here fails rather than lands.
    ///
    /// A file this build would not open — another schema version, or not one
    /// of ours at all — answers an error, as `open` does.
    pub async fn open_read(path: &Path) -> Result<Self, Error> {
        let db = libsql::Builder::new_local(path)
            .flags(libsql::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .build()
            .await?;
        let conn = db.connect()?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        let mut rows = conn
            .query("SELECT value FROM meta WHERE key = 'schema_version'", ())
            .await?;
        let found: i64 = match rows.next().await? {
            Some(r) => r.get::<String>(0)?.trim().parse().unwrap_or(-1),
            None => -1,
        };
        drop(rows);
        if found != SCHEMA_VERSION {
            return Err(Error::Schema {
                found,
                wanted: SCHEMA_VERSION,
            });
        }
        Ok(Self {
            conn,
            _db: db,
            path: path.to_path_buf(),
        })
    }

    /// The chats a database at `path` records, **without writing to it**.
    ///
    /// Not [`Store::open`], which creates the schema in whatever it is given and
    /// switches it to WAL: this is asked of every `.sqlite` found in a folder
    /// the user pointed at, and one that is somebody else's file must come out
    /// of the question untouched. A file this build would not open — another
    /// schema version, or not one of ours at all — answers an error.
    pub async fn chats_in(path: &Path) -> Result<Vec<ChatEntry>, Error> {
        let store = Self::open_read(path).await?;
        // `synced` is the last run that *walked to the end*. A cancelled or
        // failed run also writes a `runs` row, and calling that a sync would
        // tell the list a chat is current when its last attempt stopped short.
        let mut rows = store
            .conn
            .query(
                "SELECT c.id, c.title,
                        (SELECT MAX(finished) FROM runs r
                          WHERE r.chat_id = c.id AND r.reached_end = 1)
                   FROM chats c",
                (),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(ChatEntry {
                id: r.get::<i64>(0)?,
                title: r.get::<String>(1)?,
                synced: r.get::<Option<i64>>(2)?,
            });
        }
        Ok(out)
    }

    async fn ensure_schema(&self) -> Result<(), Error> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )
            .await?;
        let found = self
            .row("SELECT value FROM meta WHERE key = 'schema_version'", ())
            .await?
            .map(|r| r.get::<String>(0).unwrap_or_default());
        match found {
            Some(v) => {
                // A value that is not a number is a file this build cannot
                // account for, which is the same answer as a version it does
                // not know: refuse it. `-1` can never equal SCHEMA_VERSION.
                let found: i64 = v.trim().parse().unwrap_or(-1);
                if found != SCHEMA_VERSION {
                    return Err(Error::Schema {
                        found,
                        wanted: SCHEMA_VERSION,
                    });
                }
            }
            None => {
                self.conn.execute_batch(SCHEMA).await?;
                self.conn
                    .execute(
                        "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
                        params![SCHEMA_VERSION.to_string()],
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// The first row of a query, or `None` if it returned no rows.
    async fn row(
        &self,
        sql: &str,
        args: impl IntoParams,
    ) -> Result<Option<libsql::Row>, libsql::Error> {
        let mut rows = self.conn.query(sql, args).await?;
        rows.next().await
    }

    // --- chats and topics ---------------------------------------------------

    /// Does this file already hold a chat that is **not** `chat_id`?
    ///
    /// **The one question a per-chat file cannot answer from its name.** Two
    /// chats can sanitise to the same filename — "Support" and "Support?" both
    /// become `Support` — and the exporter names a file after the chat because
    /// a name that changed between runs would turn every sync into a fresh
    /// archive. So the check is against what the file *contains*: a chat that
    /// finds somebody else in the file it was about to open takes a
    /// distinguished name instead, and keeps taking it on every later run
    /// because the answer here does not change.
    ///
    /// Asked of `chats` rather than `messages`, because a chat is recorded on
    /// the way in — before its first message is merged, and even if it turns
    /// out to have none. A collision detected only once messages existed would
    /// let two chats share the empty file that one of them just created.
    pub async fn holds_another_chat(&self, chat_id: i64) -> Result<bool, Error> {
        let row = self
            .row(
                "SELECT 1 FROM chats WHERE id <> ?1 LIMIT 1",
                params![chat_id],
            )
            .await?;
        Ok(row.is_some())
    }

    /// `first_seen` is set once and never moved; `last_seen` is this run.
    pub async fn upsert_chat(
        &self,
        id: i64,
        title: &str,
        kind: &str,
        now: i64,
    ) -> Result<(), Error> {
        self.conn
            .execute(
                "INSERT INTO chats (id, title, type, first_seen, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(id) DO UPDATE SET title = ?2, type = ?3, last_seen = ?4",
                params![id, title, kind, now],
            )
            .await?;
        Ok(())
    }

    pub async fn upsert_topic(
        &self,
        chat_id: i64,
        id: i64,
        title: &str,
        head: &Map<String, Value>,
        now: i64,
    ) -> Result<(), Error> {
        let head = serde_json::to_string(head)?;
        self.conn
            .execute(
                "INSERT INTO topics (chat_id, id, title, head, first_seen, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT(chat_id, id) DO UPDATE SET title = ?3, head = ?4, last_seen = ?5",
                params![chat_id, id, title, head, now],
            )
            .await?;
        Ok(())
    }

    // --- messages -----------------------------------------------------------

    /// `(everything stored, everything Telegram still has)`.
    ///
    /// The difference is what earlier runs concluded was deleted, which is the
    /// number worth putting in the log: an archive of 6,643 messages of which
    /// 12 no longer exist on Telegram is a different thing from one of 6,631.
    pub async fn message_count(&self, chat_id: i64) -> Result<(i64, i64), Error> {
        let row = self
            .row(
                "SELECT COUNT(*), COUNT(*) FILTER (WHERE deleted_seen IS NULL)
                 FROM messages WHERE chat_id = ?1",
                params![chat_id],
            )
            .await?;
        Ok(match row {
            Some(r) => (r.get::<i64>(0)?, r.get::<i64>(1)?),
            None => (0, 0),
        })
    }

    /// The id to resume a database-only run from: the `keep`-th newest stored
    /// message, or 0 when fewer than that are stored.
    ///
    /// 0 is not a message id, and the read loop treats `offset_id = 0` as "from
    /// the beginning" — so an archive smaller than the window is walked whole,
    /// which is what it should be.
    pub async fn window_start(&self, chat_id: i64, keep: usize) -> Result<i64, Error> {
        let row = self
            .row(
                "SELECT id FROM messages WHERE chat_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2",
                params![chat_id, keep as i64],
            )
            .await?;
        Ok(match row {
            Some(r) => r.get::<i64>(0)?,
            None => 0,
        })
    }

    pub async fn get(&self, chat_id: i64, id: i64) -> Result<Option<StoredMessage>, Error> {
        let row = self
            .row(
                "SELECT topic_id, payload, version, deleted_seen FROM messages
                 WHERE chat_id = ?1 AND id = ?2",
                params![chat_id, id],
            )
            .await?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(StoredMessage {
            id,
            topic_id: row.get::<i64>(0)?,
            payload: serde_json::from_str(&row.get::<String>(1)?)?,
            version: row.get::<i64>(2)?,
            deleted_seen: row.get::<Option<i64>>(3)?,
        }))
    }

    /// Insert, or update in place keeping the old payload in `versions`.
    ///
    /// `topic_id` is `None` when the run could not resolve topics — a forum
    /// exported with the topic split turned off reports General for every
    /// message, and letting that overwrite what a split run learned would
    /// flatten the whole archive on one careless export. So an unknown topic
    /// inserts as [`NO_TOPIC`] and never overwrites a stored one.
    ///
    /// **A message Telegram returns is not deleted, whatever an earlier run
    /// concluded**: every present branch clears `deleted_seen`. Deletion is a
    /// thing we infer from an absence, and an inference has to yield to an
    /// observation — the message may have been in a gap a cancelled run never
    /// reached.
    pub async fn merge(
        &self,
        chat_id: i64,
        id: i64,
        topic_id: Option<i64>,
        payload: &Map<String, Value>,
        now: i64,
    ) -> Result<Merge, Error> {
        let text = serde_json::to_string(payload)?;
        let kind = payload.get("type").and_then(Value::as_str).unwrap_or("");
        // Desktop writes `date_unixtime` as a *string* — see
        // `tgx_format::date_pair`. Stored as an integer here because the
        // column is what a date range query would use.
        let date: i64 = payload
            .get("date_unixtime")
            .and_then(|v| match v {
                Value::String(s) => s.parse().ok(),
                Value::Number(n) => n.as_i64(),
                _ => None,
            })
            .unwrap_or(0);
        let from = payload.get("from_id").and_then(Value::as_str);

        let existing = self
            .row(
                "SELECT payload, version, topic_id FROM messages WHERE chat_id = ?1 AND id = ?2",
                params![chat_id, id],
            )
            .await?;

        let Some(row) = existing else {
            self.conn
                .execute(
                    "INSERT INTO messages
                       (chat_id, id, topic_id, payload, type, date_unixtime, from_id,
                        version, first_seen, last_seen, deleted_seen)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8, NULL)",
                    params![
                        chat_id,
                        id,
                        topic_id.unwrap_or(NO_TOPIC),
                        text,
                        kind,
                        date,
                        from,
                        now
                    ],
                )
                .await?;
            return Ok(Merge::Inserted);
        };

        let stored_text = row.get::<String>(0)?;
        let version = row.get::<i64>(1)?;
        let stored_topic = row.get::<i64>(2)?;
        let topic = topic_id.unwrap_or(stored_topic);

        if stored_text == text {
            self.conn
                .execute(
                    "UPDATE messages SET last_seen = ?3, deleted_seen = NULL, topic_id = ?4
                     WHERE chat_id = ?1 AND id = ?2",
                    params![chat_id, id, now, topic],
                )
                .await?;
            return Ok(Merge::Unchanged);
        }

        self.conn
            .execute(
                "INSERT OR REPLACE INTO versions (chat_id, id, version, payload, seen)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![chat_id, id, version, stored_text, now],
            )
            .await?;
        self.conn
            .execute(
                "UPDATE messages
                    SET payload = ?3, type = ?4, date_unixtime = ?5, from_id = ?6,
                        topic_id = ?7, version = version + 1, last_seen = ?8, deleted_seen = NULL
                  WHERE chat_id = ?1 AND id = ?2",
                params![chat_id, id, text, kind, date, from, topic, now],
            )
            .await?;
        Ok(Merge::Changed {
            previous_version: version,
        })
    }

    /// Mark every stored message above `above_id` that this run did not see.
    ///
    /// **Only ever called after a walk that ended normally.** A cancelled or
    /// stalled run cannot tell "Telegram no longer has it" from "we never got
    /// that far", and marking on the second would write a deletion date onto
    /// most of a chat.
    pub async fn mark_unseen_deleted(
        &self,
        chat_id: i64,
        above_id: i64,
        seen: &HashSet<i64>,
        now: i64,
    ) -> Result<usize, Error> {
        // Collected first, then updated. The alternative is an `IN (…)` list of
        // every id the run saw, which for this chat is six thousand of them in
        // one statement.
        let mut rows = self
            .conn
            .query(
                "SELECT id FROM messages
                  WHERE chat_id = ?1 AND id > ?2 AND deleted_seen IS NULL",
                params![chat_id, above_id],
            )
            .await?;
        let mut gone = Vec::new();
        while let Some(row) = rows.next().await? {
            let id = row.get::<i64>(0)?;
            if !seen.contains(&id) {
                gone.push(id);
            }
        }
        for id in &gone {
            self.conn
                .execute(
                    "UPDATE messages SET deleted_seen = ?3 WHERE chat_id = ?1 AND id = ?2",
                    params![chat_id, *id, now],
                )
                .await?;
        }
        Ok(gone.len())
    }

    /// Copy everything `src` records about `chat_id` into this file.
    ///
    /// For the one-file-for-everything `telegram.sqlite` of earlier builds,
    /// which no run writes into: each of its chats becomes a file of its own.
    /// **What this file already has wins** (`INSERT OR IGNORE`), so splitting
    /// into a database a sync has since started adds the older history and the
    /// deletions it alone saw, and changes nothing the newer file holds. Only
    /// the blobs the chat's own media rows point at come across.
    ///
    /// Returns how many of the chat's messages in `src` this file still lacks
    /// afterwards — 0, or the copy did not do what it says.
    pub async fn split_from(&self, src: &Path, chat_id: i64) -> Result<i64, Error> {
        let src = src.to_string_lossy().into_owned();
        self.conn
            .execute("ATTACH DATABASE ?1 AS src", params![src])
            .await?;
        let copied = self.copy_chat(chat_id).await;
        if copied.is_err() && !self.conn.is_autocommit() {
            let _ = self.conn.execute_batch("ROLLBACK;").await;
        }
        let lacking = async {
            copied?;
            let row = self
                .row(
                    "SELECT COUNT(*) FROM src.messages s WHERE s.chat_id = ?1 AND NOT EXISTS
                       (SELECT 1 FROM main.messages m WHERE m.chat_id = s.chat_id AND m.id = s.id)",
                    params![chat_id],
                )
                .await?;
            Ok::<i64, Error>(match row {
                Some(r) => r.get::<i64>(0)?,
                None => 0,
            })
        }
        .await;
        // Detached whatever happened above: a file left attached would be
        // held open by this connection for as long as the store lives.
        self.conn.execute_batch("DETACH DATABASE src;").await?;
        lacking
    }

    async fn copy_chat(&self, chat_id: i64) -> Result<(), Error> {
        self.conn.execute_batch("BEGIN;").await?;
        for sql in [
            "INSERT OR IGNORE INTO main.chats SELECT * FROM src.chats WHERE id = ?1",
            "INSERT OR IGNORE INTO main.topics SELECT * FROM src.topics WHERE chat_id = ?1",
            "INSERT OR IGNORE INTO main.messages SELECT * FROM src.messages WHERE chat_id = ?1",
            "INSERT OR IGNORE INTO main.versions SELECT * FROM src.versions WHERE chat_id = ?1",
            "INSERT OR IGNORE INTO main.media SELECT * FROM src.media WHERE chat_id = ?1",
            "INSERT OR IGNORE INTO main.participants
                    SELECT * FROM src.participants WHERE chat_id = ?1",
            // Not the id: this file numbers its own runs. A run already here
            // — the same chat, start and mode — is not copied twice.
            "INSERT INTO main.runs (chat_id, started, finished, mode, root, new, changed,
                                    deleted, blobs_added, reached_end, settings)
                  SELECT chat_id, started, finished, mode, root, new, changed,
                         deleted, blobs_added, reached_end, settings
                    FROM src.runs s WHERE s.chat_id = ?1 AND NOT EXISTS
                         (SELECT 1 FROM main.runs r WHERE r.chat_id = s.chat_id
                             AND r.started = s.started AND r.mode = s.mode)",
            "INSERT OR IGNORE INTO main.blobs SELECT * FROM src.blobs
                  WHERE (file_id, kind) IN (SELECT file_id, kind FROM src.media WHERE chat_id = ?1)",
            "INSERT OR IGNORE INTO main.thumbs SELECT * FROM src.thumbs
                  WHERE (file_id, kind) IN (SELECT file_id, kind FROM src.media WHERE chat_id = ?1)",
        ] {
            self.conn.execute(sql, params![chat_id]).await?;
        }
        self.conn.execute_batch("COMMIT;").await?;
        Ok(())
    }

    /// Mark one message gone from Telegram as of `now`, unless it already is.
    ///
    /// For a message recovered from an old export by `tgx import`: a finished
    /// walk already went past it and did not see it, which is the same
    /// conclusion [`Self::mark_unseen_deleted`] draws, reached a run later.
    pub async fn mark_deleted(&self, chat_id: i64, id: i64, now: i64) -> Result<(), Error> {
        self.conn
            .execute(
                "UPDATE messages SET deleted_seen = ?3
                  WHERE chat_id = ?1 AND id = ?2 AND deleted_seen IS NULL",
                params![chat_id, id, now],
            )
            .await?;
        Ok(())
    }

    /// The ids recovered from old exports rather than read from Telegram.
    ///
    /// **No column says so, by design** — a schema change would lock every
    /// older build out of the file. An import writes a `runs` row with mode
    /// `import` and merges at that row's `started`, so a recovered message is
    /// one whose `first_seen` is an import's start.
    pub async fn recovered(&self, chat_id: i64) -> Result<HashSet<i64>, Error> {
        let mut rows = self
            .conn
            .query(
                "SELECT id FROM messages
                  WHERE chat_id = ?1 AND first_seen IN
                        (SELECT started FROM runs WHERE chat_id = ?1 AND mode = 'import')",
                params![chat_id],
            )
            .await?;
        let mut out = HashSet::new();
        while let Some(row) = rows.next().await? {
            out.insert(row.get::<i64>(0)?);
        }
        Ok(out)
    }

    /// How many superseded payloads this chat has accumulated.
    ///
    /// One per time a re-read found a message different from the stored one.
    /// The archive leg reads it as an assertion: two merges of an export
    /// nobody edited in between must leave this at zero.
    pub async fn version_count(&self, chat_id: i64) -> Result<i64, Error> {
        let row = self
            .row(
                "SELECT COUNT(*) FROM versions WHERE chat_id = ?1",
                params![chat_id],
            )
            .await?;
        Ok(match row {
            Some(r) => r.get::<i64>(0)?,
            None => 0,
        })
    }

    /// Every message id stored for a chat, and whether a run has seen it
    /// deleted on Telegram — what a comparison against an older export needs,
    /// without parsing a single payload.
    pub async fn message_ids(&self, chat_id: i64) -> Result<Vec<(i64, bool)>, Error> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, deleted_seen IS NOT NULL FROM messages
                  WHERE chat_id = ?1 ORDER BY id",
                params![chat_id],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push((row.get::<i64>(0)?, row.get::<i64>(1)? != 0));
        }
        Ok(out)
    }

    /// What one run found gone and found changed, as `(id, date)` pairs,
    /// oldest first — for the report a sync writes after its counts.
    ///
    /// `at` is the run's one timestamp: [`Self::mark_unseen_deleted`] writes
    /// it as `deleted_seen`, and a merge that kept an earlier version writes it
    /// as that version's `seen`. So both questions are "what carries `at`",
    /// and nothing has to be remembered during the run to answer them.
    ///
    /// Ids and dates, **never text**: the report goes to the log, which sits
    /// beside the executable, and a message is other people's conversation.
    pub async fn changes_at(
        &self,
        chat_id: i64,
        at: i64,
    ) -> Result<(Vec<(i64, String)>, Vec<(i64, String)>), Error> {
        let mut out = (Vec::new(), Vec::new());
        for (sql, into) in [
            (
                "SELECT id, COALESCE(json_extract(payload, '$.date'), '') FROM messages
                  WHERE chat_id = ?1 AND deleted_seen = ?2 ORDER BY id",
                0,
            ),
            (
                "SELECT DISTINCT m.id, COALESCE(json_extract(m.payload, '$.date'), '')
                   FROM versions v JOIN messages m ON m.chat_id = v.chat_id AND m.id = v.id
                  WHERE v.chat_id = ?1 AND v.seen = ?2 ORDER BY m.id",
                1,
            ),
        ] {
            let mut rows = self.conn.query(sql, params![chat_id, at]).await?;
            while let Some(row) = rows.next().await? {
                let pair = (row.get::<i64>(0)?, row.get::<String>(1)?);
                if into == 0 {
                    out.0.push(pair);
                } else {
                    out.1.push(pair);
                }
            }
        }
        Ok(out)
    }

    /// Every message of one topic, oldest first.
    pub async fn messages_of(
        &self,
        chat_id: i64,
        topic_id: i64,
    ) -> Result<Vec<StoredMessage>, Error> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, payload, version, deleted_seen FROM messages
                  WHERE chat_id = ?1 AND topic_id = ?2 ORDER BY id",
                params![chat_id, topic_id],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(StoredMessage {
                id: row.get::<i64>(0)?,
                topic_id,
                payload: serde_json::from_str(&row.get::<String>(1)?)?,
                version: row.get::<i64>(2)?,
                deleted_seen: row.get::<Option<i64>>(3)?,
            });
        }
        Ok(out)
    }

    /// Every message of a chat, every topic, oldest first — what a viewer
    /// lists. Deleted ones included: they are the reason to look.
    pub async fn all_messages(&self, chat_id: i64) -> Result<Vec<StoredMessage>, Error> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, topic_id, payload, version, deleted_seen FROM messages
                  WHERE chat_id = ?1 ORDER BY date_unixtime, id",
                params![chat_id],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(StoredMessage {
                id: row.get::<i64>(0)?,
                topic_id: row.get::<i64>(1)?,
                payload: serde_json::from_str(&row.get::<String>(2)?)?,
                version: row.get::<i64>(3)?,
                deleted_seen: row.get::<Option<i64>>(4)?,
            });
        }
        Ok(out)
    }

    /// Every superseded payload of one message, oldest first. Empty for a
    /// message nobody has edited — or whose reactions never moved, which also
    /// makes a version; see [`Self::changes_at`].
    pub async fn versions_of(&self, chat_id: i64, id: i64) -> Result<Vec<StoredVersion>, Error> {
        let mut rows = self
            .conn
            .query(
                "SELECT version, payload, seen FROM versions
                  WHERE chat_id = ?1 AND id = ?2 ORDER BY version",
                params![chat_id, id],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(StoredVersion {
                version: row.get::<i64>(0)?,
                payload: serde_json::from_str(&row.get::<String>(1)?)?,
                seen: row.get::<i64>(2)?,
            });
        }
        Ok(out)
    }

    /// The messages of a chat whose text contains `needle`, **ignoring case**,
    /// oldest first and at most `limit` of them.
    ///
    /// **Earlier versions and deleted messages are searched too**, because
    /// those are what an archive is for: text edited away is found through the
    /// version that still has it, and a message gone from Telegram is found
    /// like any other. See [`Hit`] for how each is flagged.
    ///
    /// **The case is folded here, in Rust, not by SQLite.** SQLite's `lower()`
    /// and `LIKE` fold ASCII only, and these chats are Serbian: `LIKE '%ђорђе%'`
    /// does not find `Ђорђе`. So the text comes out of the database and is
    /// matched here — a scan of every message, which for a chat of a hundred
    /// thousand is a fraction of a second, and costs no index and no schema
    /// change (an FTS5 table would be both, and would lock older builds out of
    /// the file).
    pub async fn search(
        &self,
        chat_id: i64,
        needle: &str,
        limit: usize,
    ) -> Result<Vec<Hit>, Error> {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        // The versions first: a message matched only there is still listed in
        // its place among the rest, so they are collected and then consulted
        // during the one ordered pass below. The earliest matching version
        // wins the excerpt.
        let mut earlier: std::collections::HashMap<i64, String> = Default::default();
        let mut rows = self
            .conn
            .query(
                "SELECT id, payload FROM versions WHERE chat_id = ?1 ORDER BY id, version",
                params![chat_id],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let id = row.get::<i64>(0)?;
            if earlier.contains_key(&id) {
                continue;
            }
            let text = payload_text(&row.get::<String>(1)?);
            if text.to_lowercase().contains(&needle) {
                earlier.insert(id, text);
            }
        }
        drop(rows);

        let mut rows = self
            .conn
            .query(
                "SELECT id, topic_id, COALESCE(json_extract(payload, '$.date'), ''),
                        deleted_seen IS NOT NULL, payload
                   FROM messages WHERE chat_id = ?1 ORDER BY date_unixtime, id",
                params![chat_id],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let id = row.get::<i64>(0)?;
            let text = payload_text(&row.get::<String>(4)?);
            let now = text.to_lowercase().contains(&needle);
            let excerpt = match (now, earlier.remove(&id)) {
                (true, _) => text,
                (false, Some(old)) => old,
                (false, None) => continue,
            };
            out.push(Hit {
                id,
                topic_id: row.get::<i64>(1)?,
                date: row.get::<String>(2)?,
                deleted: row.get::<i64>(3)? != 0,
                in_earlier_version: !now,
                excerpt,
            });
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    // --- media --------------------------------------------------------------
    //
    // **Keyed on `(file_id, kind)`, not on `file_id` alone.** Telegram's photo
    // ids and document ids are two spaces, and this file dedupes across every
    // chat it has ever seen, for good — so a bare id is a global, permanent
    // assumption that the two never collide. `kind` is the `_LAYOUT` folder the
    // file's *shape* put it in, which is already on the job, so the second half
    // of the key costs nothing. `NameBook::by_id` takes the same risk but only
    // within one folder and only for the length of a run.

    pub async fn has_blob(&self, file_id: i64, kind: &str) -> Result<bool, Error> {
        Ok(self
            .row(
                "SELECT 1 FROM blobs WHERE file_id = ?1 AND kind = ?2",
                params![file_id, kind],
            )
            .await?
            .is_some())
    }

    /// Store the bytes once. A second call for the same file is a no-op.
    pub async fn put_blob(
        &self,
        file_id: i64,
        kind: &str,
        mime: &str,
        file_name: Option<&str>,
        bytes: &[u8],
        now: i64,
    ) -> Result<(), Error> {
        let size = bytes.len() as i64;
        if size > MAX_BLOB {
            return Err(Error::TooLarge { file_id, size });
        }
        self.conn
            .execute(
                "INSERT OR IGNORE INTO blobs (file_id, kind, mime, file_name, size, bytes, first_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    file_id,
                    kind,
                    mime,
                    file_name,
                    size,
                    libsql::Value::Blob(bytes.to_vec()),
                    now
                ],
            )
            .await?;
        Ok(())
    }

    /// Telegram's own thumbnail for a file. The *rendered* preview
    /// (`<stem>_thumb<ext>`) is an HTML-only artefact and is not stored.
    pub async fn put_thumb(&self, file_id: i64, kind: &str, bytes: &[u8]) -> Result<(), Error> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO thumbs (file_id, kind, bytes) VALUES (?1, ?2, ?3)",
                params![file_id, kind, libsql::Value::Blob(bytes.to_vec())],
            )
            .await?;
        Ok(())
    }

    /// Point a message at a blob. `path` is the name the folder export used, or
    /// would have used, and is informational.
    pub async fn link_media(
        &self,
        chat_id: i64,
        message_id: i64,
        role: &str,
        file_id: i64,
        kind: &str,
        path: &str,
    ) -> Result<(), Error> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO media (chat_id, message_id, role, file_id, kind, path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![chat_id, message_id, role, file_id, kind, path],
            )
            .await?;
        Ok(())
    }

    /// The files one message points at, with what the database knows of each.
    /// Ordered by role so a message's photo and its thumbnail come out the
    /// same way round every time.
    pub async fn media_of(&self, chat_id: i64, message_id: i64) -> Result<Vec<MediaEntry>, Error> {
        let mut rows = self
            .conn
            .query(
                "SELECT m.role, m.file_id, m.kind, m.path, b.file_name, b.size
                   FROM media m LEFT JOIN blobs b ON b.file_id = m.file_id AND b.kind = m.kind
                  WHERE m.chat_id = ?1 AND m.message_id = ?2 ORDER BY m.role",
                params![chat_id, message_id],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(MediaEntry {
                role: row.get::<String>(0)?,
                file_id: row.get::<i64>(1)?,
                kind: row.get::<String>(2)?,
                path: row.get::<String>(3)?,
                file_name: row.get::<Option<String>>(4)?,
                size: row.get::<Option<i64>>(5)?,
            });
        }
        Ok(out)
    }

    pub async fn blob(&self, file_id: i64, kind: &str) -> Result<Option<Vec<u8>>, Error> {
        let row = self
            .row(
                "SELECT bytes FROM blobs WHERE file_id = ?1 AND kind = ?2",
                params![file_id, kind],
            )
            .await?;
        Ok(match row {
            Some(r) => Some(r.get::<Vec<u8>>(0)?),
            None => None,
        })
    }

    // --- participants and runs ----------------------------------------------

    /// **Ex-members stay.** `participants.json` is a snapshot of who is in the
    /// chat now; this table is who has ever been seen in it, which is the
    /// question an archive gets asked.
    pub async fn upsert_participant(
        &self,
        chat_id: i64,
        peer: &str,
        info: &Value,
        now: i64,
    ) -> Result<(), Error> {
        let info = serde_json::to_string(info)?;
        self.conn
            .execute(
                "INSERT INTO participants (chat_id, peer, info, first_seen, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(chat_id, peer) DO UPDATE SET info = ?3, last_seen = ?4",
                params![chat_id, peer, info, now],
            )
            .await?;
        Ok(())
    }

    /// Open a write transaction, if one is not open already.
    ///
    /// Six thousand messages is six thousand `INSERT`s, and each one outside a
    /// transaction is its own fsync. The engine commits every couple of hundred
    /// so a crash costs a batch rather than a chat.
    pub async fn begin_batch(&self) -> Result<(), Error> {
        if self.conn.is_autocommit() {
            self.conn.execute_batch("BEGIN;").await?;
        }
        Ok(())
    }

    /// Commit whatever is open. **A no-op when nothing is** — which is what
    /// lets every exit path call it without first asking whether it needs to.
    pub async fn commit_batch(&self) -> Result<(), Error> {
        if !self.conn.is_autocommit() {
            self.conn.execute_batch("COMMIT;").await?;
        }
        Ok(())
    }

    pub async fn start_run(
        &self,
        chat_id: i64,
        mode: &str,
        root: Option<&Path>,
        settings_json: &str,
        now: i64,
    ) -> Result<i64, Error> {
        let root = root.map(|p| p.to_string_lossy().into_owned());
        self.conn
            .execute(
                "INSERT INTO runs (chat_id, started, mode, root, settings)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![chat_id, now, mode, root, settings_json],
            )
            .await?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finish_run(
        &self,
        run_id: i64,
        new: usize,
        changed: usize,
        deleted: usize,
        blobs_added: usize,
        reached_end: bool,
        now: i64,
    ) -> Result<(), Error> {
        self.conn
            .execute(
                "UPDATE runs SET finished = ?2, new = ?3, changed = ?4, deleted = ?5,
                                 blobs_added = ?6, reached_end = ?7
                  WHERE id = ?1",
                params![
                    run_id,
                    now,
                    new as i64,
                    changed as i64,
                    deleted as i64,
                    blobs_added as i64,
                    i64::from(reached_end)
                ],
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One temp file per test, modelled on `tgx_tg::output`'s `tmp`.
    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tgx-archive-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{name}.sqlite"));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        path
    }

    async fn store(name: &str) -> Store {
        Store::open(&tmp(name)).await.expect("opening the store")
    }

    fn msg(id: i64, text: &str) -> Map<String, Value> {
        json!({
            "id": id,
            "type": "message",
            "date": "2025-12-18T16:00:00",
            "date_unixtime": "1766071072",
            "from": "Ivana",
            "from_id": "user31774388",
            "text": text,
            "text_entities": [],
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[tokio::test]
    async fn a_fresh_file_belongs_to_whichever_chat_opens_it() {
        // The question one database per chat rests on. A file that has never
        // been written to must not claim to hold somebody else, or the very
        // first export of every chat would take a suffixed name.
        let s = store("owner").await;
        assert!(!s.holds_another_chat(7).await.unwrap());
        s.upsert_chat(7, "Dev Team", "public_supergroup", 100)
            .await
            .unwrap();
        assert!(!s.holds_another_chat(7).await.unwrap(), "its own chat");
        // And a second chat finds the first, which is what sends it to a file
        // of its own rather than into this one.
        assert!(s.holds_another_chat(8).await.unwrap());
    }

    #[tokio::test]
    async fn the_chats_in_a_file_are_read_without_writing_to_it() {
        let s = store("probe").await;
        s.upsert_chat(7, "Dev Team", "public_supergroup", 100)
            .await
            .unwrap();
        let path = s.path().to_path_buf();
        drop(s);
        assert_eq!(
            Store::chats_in(&path).await.unwrap(),
            vec![ChatEntry {
                id: 7,
                title: "Dev Team".into(),
                synced: None,
            }]
        );

        // A database that is not ours is refused, and left as it was found.
        let foreign = tmp("probe-foreign");
        let db = libsql::Builder::new_local(&foreign).build().await.unwrap();
        db.connect()
            .unwrap()
            .execute_batch("CREATE TABLE notes (body TEXT);")
            .await
            .unwrap();
        drop(db);
        let before = std::fs::read(&foreign).unwrap();
        assert!(Store::chats_in(&foreign).await.is_err());
        assert_eq!(std::fs::read(&foreign).unwrap(), before);
    }

    #[tokio::test]
    async fn a_chat_is_recorded_before_it_has_any_messages() {
        // `holds_another_chat` asks `chats` and not `messages` for exactly
        // this: a chat that produced nothing still owns the file it created,
        // so the next chat with the same sanitised title does not adopt it.
        let s = store("empty-owner").await;
        s.upsert_chat(7, "Support", "private_group", 100)
            .await
            .unwrap();
        assert_eq!(s.message_count(7).await.unwrap(), (0, 0));
        assert!(s.holds_another_chat(8).await.unwrap());
    }

    #[tokio::test]
    async fn a_new_message_is_inserted_and_an_identical_one_is_unchanged() {
        let s = store("insert").await;
        assert_eq!(
            s.merge(1, 10, Some(1), &msg(10, "hi"), 100).await.unwrap(),
            Merge::Inserted
        );
        assert_eq!(
            s.merge(1, 10, Some(1), &msg(10, "hi"), 200).await.unwrap(),
            Merge::Unchanged
        );
        assert_eq!(s.message_count(1).await.unwrap(), (1, 1));
    }

    #[tokio::test]
    async fn a_changed_message_keeps_its_previous_payload_in_versions() {
        let s = store("versions").await;
        s.merge(1, 10, Some(1), &msg(10, "before"), 100)
            .await
            .unwrap();
        let what = s
            .merge(1, 10, Some(1), &msg(10, "after"), 200)
            .await
            .unwrap();
        assert_eq!(
            what,
            Merge::Changed {
                previous_version: 1
            }
        );

        let now = s.get(1, 10).await.unwrap().expect("still there");
        assert_eq!(now.payload["text"], json!("after"));
        assert_eq!(now.version, 2);

        let row = s
            .row(
                "SELECT payload FROM versions WHERE chat_id = 1 AND id = 10 AND version = 1",
                (),
            )
            .await
            .unwrap()
            .expect("the old payload is kept");
        let old: Map<String, Value> = serde_json::from_str(&row.get::<String>(0).unwrap()).unwrap();
        assert_eq!(old["text"], json!("before"));
    }

    #[tokio::test]
    async fn marking_touches_only_unseen_ids_above_the_start() {
        let s = store("marking").await;
        for id in 1..=10 {
            s.merge(1, id, Some(1), &msg(id, "x"), 100).await.unwrap();
        }
        let seen: HashSet<i64> = [6, 7, 9, 10].into_iter().collect();
        assert_eq!(s.mark_unseen_deleted(1, 5, &seen, 300).await.unwrap(), 1);

        assert!(s.get(1, 8).await.unwrap().unwrap().deleted_seen.is_some());
        for id in 1..=5 {
            assert!(
                s.get(1, id).await.unwrap().unwrap().deleted_seen.is_none(),
                "id {id} is below the start and must not be touched"
            );
        }
        assert_eq!(s.message_count(1).await.unwrap(), (10, 9));
    }

    #[tokio::test]
    async fn a_marked_message_is_unmarked_when_telegram_returns_it_again() {
        let s = store("unmark").await;
        for id in [8, 9] {
            s.merge(1, id, Some(1), &msg(id, "x"), 100).await.unwrap();
        }
        s.mark_unseen_deleted(1, 0, &HashSet::new(), 200)
            .await
            .unwrap();
        assert!(s.get(1, 8).await.unwrap().unwrap().deleted_seen.is_some());

        // Returned again, unchanged: unmarked, and no version written — a
        // deletion that turned out not to be one is not a change to the
        // message.
        assert_eq!(
            s.merge(1, 8, Some(1), &msg(8, "x"), 300).await.unwrap(),
            Merge::Unchanged
        );
        assert!(s.get(1, 8).await.unwrap().unwrap().deleted_seen.is_none());
        assert!(s
            .row("SELECT 1 FROM versions WHERE chat_id = 1 AND id = 8", ())
            .await
            .unwrap()
            .is_none());

        // And the one that really is gone stays gone.
        assert!(s.get(1, 9).await.unwrap().unwrap().deleted_seen.is_some());
    }

    #[tokio::test]
    async fn window_start_is_zero_for_a_small_archive_and_the_501st_newest_otherwise() {
        let s = store("window").await;
        for id in 1..=100 {
            s.merge(1, id, Some(1), &msg(id, "x"), 100).await.unwrap();
        }
        assert_eq!(s.window_start(1, 500).await.unwrap(), 0);
        assert_eq!(s.window_start(1, 10).await.unwrap(), 90);
        // 0 means "only what is new": start at the newest stored id.
        assert_eq!(s.window_start(1, 0).await.unwrap(), 100);
    }

    #[tokio::test]
    async fn two_chats_do_not_see_each_others_messages() {
        let s = store("two-chats").await;
        s.merge(1, 10, Some(1), &msg(10, "one"), 100).await.unwrap();
        s.merge(2, 10, Some(1), &msg(10, "two"), 100).await.unwrap();
        assert_eq!(s.message_count(1).await.unwrap(), (1, 1));
        assert_eq!(s.get(2, 10).await.unwrap().unwrap().payload["text"], "two");
        // Marking one chat leaves the other alone.
        s.mark_unseen_deleted(1, 0, &HashSet::new(), 200)
            .await
            .unwrap();
        assert!(s.get(2, 10).await.unwrap().unwrap().deleted_seen.is_none());
    }

    #[tokio::test]
    async fn an_unresolved_topic_never_overwrites_one_a_split_run_learned() {
        let s = store("topics").await;
        s.merge(1, 10, Some(4242), &msg(10, "x"), 100)
            .await
            .unwrap();
        // A later run with the topic split turned off reports General for
        // everything. It must not flatten the archive.
        s.merge(1, 10, None, &msg(10, "x"), 200).await.unwrap();
        assert_eq!(s.get(1, 10).await.unwrap().unwrap().topic_id, 4242);
        assert_eq!(s.messages_of(1, 4242).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_blob_is_stored_once_per_file_id_and_linked_from_every_message() {
        let s = store("blobs").await;
        assert!(!s.has_blob(77, "photos").await.unwrap());
        s.put_blob(77, "photos", "image/jpeg", None, b"first", 100)
            .await
            .unwrap();
        s.put_blob(77, "photos", "image/jpeg", None, b"second", 200)
            .await
            .unwrap();
        assert!(s.has_blob(77, "photos").await.unwrap());
        assert_eq!(s.blob(77, "photos").await.unwrap().unwrap(), b"first");

        // The same id in another kind is another file, not the same one.
        assert!(!s.has_blob(77, "video_files").await.unwrap());

        s.link_media(1, 10, "photo", 77, "photos", "photos/photo_1.jpg")
            .await
            .unwrap();
        s.link_media(1, 11, "photo", 77, "photos", "photos/photo_1.jpg")
            .await
            .unwrap();
        let row = s
            .row("SELECT COUNT(*) FROM media WHERE file_id = 77", ())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.get::<i64>(0).unwrap(), 2);
    }

    #[tokio::test]
    async fn a_blob_over_the_ceiling_is_refused_not_truncated() {
        let s = store("too-big").await;
        let huge = vec![0u8; (MAX_BLOB + 1) as usize];
        match s
            .put_blob(5, "video_files", "video/mp4", None, &huge, 100)
            .await
        {
            Err(Error::TooLarge { file_id, size }) => {
                assert_eq!(file_id, 5);
                assert_eq!(size, MAX_BLOB + 1);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
        assert!(!s.has_blob(5, "video_files").await.unwrap());
    }

    #[tokio::test]
    async fn a_schema_version_this_build_does_not_know_is_refused() {
        let path = tmp("schema");
        {
            let s = Store::open(&path).await.unwrap();
            s.conn
                .execute(
                    "UPDATE meta SET value = '99' WHERE key = 'schema_version'",
                    (),
                )
                .await
                .unwrap();
        }
        match Store::open(&path).await {
            Err(Error::Schema { found, wanted }) => {
                assert_eq!((found, wanted), (99, SCHEMA_VERSION));
            }
            other => panic!("expected a schema refusal, got {:?}", other.map(|_| ())),
        }
    }

    #[tokio::test]
    async fn a_run_records_what_it_did() {
        let s = store("runs").await;
        let id = s
            .start_run(1, "window", None, "{\"export_db\":true}", 100)
            .await
            .unwrap();
        s.finish_run(id, 12, 3, 1, 4, true, 200).await.unwrap();
        let row = s
            .row(
                "SELECT mode, new, changed, deleted, blobs_added, reached_end, finished
                   FROM runs WHERE id = ?1",
                params![id],
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "window");
        assert_eq!(row.get::<i64>(1).unwrap(), 12);
        assert_eq!(row.get::<i64>(5).unwrap(), 1);
        assert_eq!(row.get::<i64>(6).unwrap(), 200);
    }

    #[tokio::test]
    async fn committing_with_nothing_open_is_harmless() {
        let s = store("batch").await;
        s.commit_batch().await.unwrap();
        s.begin_batch().await.unwrap();
        s.begin_batch().await.unwrap();
        s.merge(1, 1, Some(1), &msg(1, "x"), 100).await.unwrap();
        s.commit_batch().await.unwrap();
        s.commit_batch().await.unwrap();
        assert_eq!(s.message_count(1).await.unwrap(), (1, 1));
    }

    fn with_text(id: i64, text: Value) -> Map<String, Value> {
        let mut m = msg(id, "");
        m.insert("text".into(), text);
        m
    }

    #[tokio::test]
    async fn search_finds_current_text_and_ignores_what_does_not_match() {
        let s = store("search-current").await;
        s.merge(1, 10, Some(1), &msg(10, "Sastanak u petak"), 100)
            .await
            .unwrap();
        s.merge(1, 11, Some(1), &msg(11, "nešto drugo"), 100)
            .await
            .unwrap();
        // Another chat in the same file is not this chat's hit.
        s.merge(2, 12, Some(1), &msg(12, "petak"), 100)
            .await
            .unwrap();
        let hits = s.search(1, "PETAK", 50).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, 10);
        assert_eq!(hits[0].date, "2025-12-18T16:00:00");
        assert!(!hits[0].deleted && !hits[0].in_earlier_version);
        assert_eq!(hits[0].excerpt, "Sastanak u petak");
        assert!(s.search(1, "   ", 50).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn search_finds_text_that_was_edited_away() {
        let s = store("search-version").await;
        s.merge(1, 10, Some(1), &msg(10, "lozinka je 1234"), 100)
            .await
            .unwrap();
        s.merge(1, 10, Some(1), &msg(10, "[obrisano]"), 200)
            .await
            .unwrap();
        let hits = s.search(1, "lozinka", 50).await.unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].in_earlier_version);
        assert_eq!(hits[0].excerpt, "lozinka je 1234");
        // What the message says now is a hit in the ordinary way.
        let now = s.search(1, "obrisano", 50).await.unwrap();
        assert!(!now[0].in_earlier_version);
    }

    #[tokio::test]
    async fn search_finds_a_deleted_message_and_says_so() {
        let s = store("search-deleted").await;
        s.merge(1, 10, Some(1), &msg(10, "vidimo se"), 100)
            .await
            .unwrap();
        s.mark_deleted(1, 10, 300).await.unwrap();
        let hits = s.search(1, "vidimo", 50).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].deleted);
    }

    #[tokio::test]
    async fn search_folds_cyrillic_case_which_sqlite_cannot() {
        let s = store("search-cyrillic").await;
        s.merge(1, 10, Some(1), &msg(10, "Ђорђе стиже у 5"), 100)
            .await
            .unwrap();
        for needle in ["ђорђе", "ЂОРЂЕ", "Ђорђе"] {
            let hits = s.search(1, needle, 50).await.unwrap();
            assert_eq!(hits.len(), 1, "{needle}");
        }
    }

    #[tokio::test]
    async fn search_reads_text_given_as_entities_as_one_string() {
        let s = store("search-entities").await;
        let text = json!([{"type": "bold", "text": "Важно"}, " sutra u 10", {"type": "link", "text": "https://x.rs"}]);
        s.merge(1, 10, Some(1), &with_text(10, text), 100)
            .await
            .unwrap();
        // Across the boundary between an entity and the plain run after it,
        // which a search of the raw JSON would never match.
        let hits = s.search(1, "важно sutra", 50).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].excerpt, "Важно sutra u 10https://x.rs");
        // And the JSON's own key names are not text.
        assert!(s.search(1, "bold", 50).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn search_stops_at_the_limit_oldest_first() {
        let s = store("search-limit").await;
        for id in 1..=5 {
            s.merge(1, id, Some(1), &msg(id, "isto"), 100)
                .await
                .unwrap();
        }
        let ids: Vec<i64> = s
            .search(1, "isto", 3)
            .await
            .unwrap()
            .iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn a_read_only_store_reads_and_refuses_to_write() {
        let s = store("read-only").await;
        s.merge(1, 10, Some(1), &msg(10, "v1"), 100).await.unwrap();
        s.merge(1, 10, Some(1), &msg(10, "v2"), 200).await.unwrap();
        s.put_blob(
            77,
            "files",
            "application/pdf",
            Some("plan.pdf"),
            b"%PDF",
            100,
        )
        .await
        .unwrap();
        s.link_media(1, 10, "file", 77, "files", "files/plan.pdf")
            .await
            .unwrap();
        s.link_media(1, 10, "thumbnail", 78, "files", "files/plan_thumb.jpg")
            .await
            .unwrap();
        let path = s.path().to_path_buf();
        drop(s);

        let r = Store::open_read(&path).await.unwrap();
        assert_eq!(r.all_messages(1).await.unwrap().len(), 1);
        let versions = r.versions_of(1, 10).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].payload["text"], json!("v1"));
        assert_eq!(versions[0].seen, 200);
        let media = r.media_of(1, 10).await.unwrap();
        assert_eq!(media.len(), 2);
        assert_eq!(media[0].file_name.as_deref(), Some("plan.pdf"));
        assert_eq!(media[0].size, Some(4));
        assert_eq!(media[1].size, None, "linked, but the bytes never arrived");
        assert!(r.merge(1, 11, Some(1), &msg(11, "x"), 300).await.is_err());
    }

    #[test]
    fn merge_volatile_replaces_text_and_reactions_and_keeps_everything_else() {
        let mut stored = msg(10, "before");
        stored.insert("file".into(), json!("files/a.mp3"));
        stored.insert("reactions".into(), json!([{ "emoji": "👍", "count": 1 }]));

        let mut fresh = msg(10, "after");
        fresh.insert("edited".into(), json!("2025-12-19T10:00:00"));
        // The fresh read has no reactions any more, and a different file — one
        // of those must survive and the other must not.
        fresh.insert("file".into(), json!("files/REPLACED.mp3"));

        let out = merge_volatile(&stored, &fresh);
        assert_eq!(out["text"], json!("after"));
        assert_eq!(out["edited"], json!("2025-12-19T10:00:00"));
        assert!(!out.contains_key("reactions"), "a removed reaction is gone");
        assert_eq!(
            out["file"],
            json!("files/a.mp3"),
            "media keys are never taken from a re-read"
        );
        // And the result is in Desktop's key order, so it still emits exactly.
        let keys: Vec<&String> = out.keys().collect();
        let ordered = tgx_format::order::ordered(&out);
        assert_eq!(keys, ordered.keys().collect::<Vec<&String>>());
    }

    #[test]
    fn a_file_the_earlier_run_declined_to_fetch_is_filled_in_later() {
        // Exported once with a 20 MB limit, so Desktop's placeholder went in.
        let mut stored = msg(10, "look at this");
        stored.insert(
            "file".into(),
            json!("(File exceeds maximum size. Change data exporting settings to download.)"),
        );
        // Re-synced with the limit raised: the run has the real path now.
        let mut fresh = msg(10, "look at this");
        fresh.insert("file".into(), json!("video_files/clip.mp4"));
        fresh.insert("file_size".into(), json!(48_000_000));

        let out = merge_volatile(&stored, &fresh);
        assert_eq!(
            out["file"],
            json!("video_files/clip.mp4"),
            "a placeholder is a record of a refusal, not of what the message carried"
        );
    }

    #[test]
    fn a_real_media_path_is_never_overwritten_by_a_re_read() {
        let mut stored = msg(10, "x");
        stored.insert("file".into(), json!("files/original.mp3"));
        let mut fresh = msg(10, "x");
        fresh.insert("file".into(), json!("files/REPLACED.mp3"));

        assert_eq!(
            merge_volatile(&stored, &fresh)["file"],
            json!("files/original.mp3"),
            "only placeholders are replaceable; history is not"
        );
    }
}
