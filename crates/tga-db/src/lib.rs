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
//! **The rows come through the exporter's own reader**, `tgx_archive::Store`,
//! opened read-only. See `Cargo.toml` for why this stopped being rusqlite when
//! the two repositories merged. One behaviour moved with it: a payload or a
//! topic head that does not parse as JSON used to be skipped here, and now
//! fails the read with the file named. The exporter is the only writer and
//! serialises every payload with serde_json, so an unparsable one is a damaged
//! file — and a report quietly missing messages from a damaged file is the
//! worse of the two outcomes. The corpus test's "every message SQL counts" rule
//! already said so.
//!
//! What the database has that a folder never did — `deleted_seen` on a message
//! Telegram no longer returns, the `versions` of an edited one, and the `runs`
//! table — is read but not yet carried into the model. See `PLAN.md`.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use tga_read::{Export, Progress};
use tgx_archive::Store;

/// What the exporter named the file while it kept every chat in one. It now
/// writes `<title>/<title>.sqlite` per chat; see [`find`].
pub const FILE_NAME: &str = "telegram.sqlite";

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

/// Run one of the store's futures to completion on this thread.
///
/// **Not callable from inside an async task** — tokio refuses to start a
/// runtime within a runtime. Every caller of this crate is a plain function on
/// a worker thread or the command line; a caller living on an async runtime
/// reaches here through `spawn_blocking`.
fn block<T>(work: impl Future<Output = Result<T>>) -> Result<T> {
    runtime()?.block_on(work)
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .context("starting the database reader")
}

/// Open read-only.
///
/// **Read-only is the point, not a precaution.** The window and the exporter
/// can be pointed at one file at the same time, and opening a SQLite database
/// for writing takes a lock and can create `-wal` and `-shm` files beside it.
/// An analyser has no business doing either to somebody's archive.
/// `Store::open_read` is the exporter's own read-only open, and it refuses a
/// schema version this build does not know.
async fn open(path: &Path) -> Result<Store> {
    Store::open_read(path)
        .await
        .with_context(|| format!("{} is not a database this build can read", path.display()))
}

/// One database, open for as long as its caller keeps asking it things.
///
/// **The free functions below each start a runtime and open the file**, which
/// is right for one question and wrong for ten thousand. A digest or a photo
/// pass asks for one blob per attachment, and paid a runtime, a libsql open and
/// a schema check for every one of them; one `analyse` opened the same file
/// five times. This holds one of each for its lifetime, and the free functions
/// are this, opened for a single call.
///
/// The same rule as [`block`]: not for use from inside an async task.
pub struct Reader {
    runtime: tokio::runtime::Runtime,
    store: Store,
    path: PathBuf,
}

impl Reader {
    /// Open read-only; see [`open`].
    pub fn open(path: &Path) -> Result<Self> {
        let runtime = runtime()?;
        let store = runtime.block_on(open(path))?;
        Ok(Reader {
            runtime,
            store,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// [`chats`].
    pub fn chats(&self) -> Result<Vec<Chat>> {
        self.runtime.block_on(chats_in(&self.store))
    }

    /// [`load`].
    pub fn load(&self, wanted: Option<&str>, progress: Option<Progress<'_>>) -> Result<Export> {
        self.runtime
            .block_on(load_from(&self.store, &self.path, wanted, progress))
    }

    /// [`deleted`].
    pub fn deleted(&self, chat_id: i64) -> Result<usize> {
        let (all, alive) = self.runtime.block_on(self.store.message_count(chat_id))?;
        Ok((all - alive).max(0) as usize)
    }

    /// [`edits`].
    pub fn edits(&self, chat_id: i64) -> Result<(usize, usize)> {
        let (msgs, rows) = self.runtime.block_on(self.store.edit_counts(chat_id))?;
        Ok((msgs.max(0) as usize, rows.max(0) as usize))
    }

    /// [`blob`].
    pub fn blob(&self, file_id: i64, kind: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.runtime.block_on(self.store.blob(file_id, kind))?)
    }

    /// When the newest finished update of the chat ran; see
    /// `Store::last_update`. A catch-up is bookmarked by it.
    pub fn last_update(&self, chat_id: i64) -> Result<Option<i64>> {
        Ok(self.runtime.block_on(self.store.last_update(chat_id))?)
    }

    /// How many updates after `after` and up to `through` added anything, and
    /// the ids of what they added.
    pub fn added_by_updates(
        &self,
        chat_id: i64,
        after: Option<i64>,
        through: i64,
    ) -> Result<(usize, Vec<i64>)> {
        Ok(self
            .runtime
            .block_on(self.store.added_by_updates(chat_id, after, through))?)
    }

    /// How many messages were sent at or after `since`, in unix seconds.
    pub fn count_since(&self, chat_id: i64, since: i64) -> Result<usize> {
        Ok(self
            .runtime
            .block_on(self.store.count_since(chat_id, since))?)
    }

    /// When the newest message was sent, in unix seconds.
    pub fn newest(&self, chat_id: i64) -> Result<Option<i64>> {
        Ok(self.runtime.block_on(self.store.newest(chat_id))?)
    }

    /// [`media_index`].
    pub fn media_index(&self, chat_id: i64) -> Result<HashMap<(i64, String), (i64, String)>> {
        Ok(self
            .runtime
            .block_on(self.store.media_rows_of(chat_id))?
            .into_iter()
            .map(|r| ((r.message_id, r.role), (r.file_id, r.kind)))
            .collect())
    }
}

/// Every chat in the file, most messages first.
pub fn chats(path: &Path) -> Result<Vec<Chat>> {
    Reader::open(path)?.chats()
}

/// Just the names, without counting anything.
///
/// **Separate from [`chats`] because the window calls this on every keystroke**
/// in the path field. Counting messages per chat is an index scan over the
/// whole table; on a 450,000-message archive that is long enough to feel in a
/// text field, and the field only needs to say what the file is.
pub fn chat_titles(path: &Path) -> Result<Vec<(i64, String)>> {
    block(async { Ok(open(path).await?.chat_titles().await?) })
}

async fn chats_in(store: &Store) -> Result<Vec<Chat>> {
    Ok(store
        .chat_summaries()
        .await?
        .into_iter()
        .map(|c| Chat {
            id: c.id,
            title: c.title,
            kind: c.kind,
            messages: c.messages.max(0) as usize,
        })
        .collect())
}

/// Read one chat out of the database into an [`Export`].
///
/// `wanted` picks the chat by id or by title when the file holds more than one.
/// The exporter is moving to one database per chat, so the ordinary case is a
/// file with exactly one and no argument; a file with several is read as its
/// largest, and the caller is expected to say which others it passed over.
pub fn load(path: &Path, wanted: Option<&str>, progress: Option<Progress<'_>>) -> Result<Export> {
    Reader::open(path)?.load(wanted, progress)
}

async fn load_from(
    store: &Store,
    path: &Path,
    wanted: Option<&str>,
    mut progress: Option<Progress<'_>>,
) -> Result<Export> {
    let all = chats_in(store).await?;
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

    // In the order the topics were created. **Only topics with a `topics`
    // row**, as it always was here: a message is read through the topic it is
    // filed under, so one filed under a topic no run ever listed is not read.
    // `Store::message_topics` can name such topics; reading them would change
    // the figures, which is not a thing a port of the reader should do.
    let heads = store.topics_of(chat.id).await?;
    let total = heads.len();
    for (index, stored) in heads.iter().enumerate() {
        // Back into the map a `result.json` would have carried: the head holds
        // everything except the three keys the `chats` and `topics` rows own.
        let mut header = stored.head.clone();
        header.insert("name".into(), Value::String(stored.title.clone()));
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
        let start = export.msgs.len();
        for stored in store.messages_of(chat.id, stored.id).await? {
            let raw = Value::Object(stored.payload);
            if let Some(msg) = tga_read::one(&raw, index, topic.root) {
                export.msgs.push(msg);
                topic.messages += 1;
            }
        }
        // The chat's title, not the topic's: the sender a 1-on-1 chat implies
        // is the person it is named after.
        tga_read::imply_senders(&mut export.msgs[start..], &topic, &chat.title);

        let label = topic.name.clone();
        export.topics.push(topic);
        if let Some(report) = progress.as_deref_mut() {
            report(index + 1, total, &label);
        }
    }

    export.roster_complete = head_flag(&heads, "members_complete");
    export.roster = roster(store, chat.id).await;
    // **`capped` is always `None` here, and that is the honest answer.** The
    // exporter writes `complete` and `capped` into `participants.json`, but
    // only `members_complete` reaches the topic head, so a database has no
    // record of whether the member fetch stopped at a limit. `roster_capped`
    // is an `Option` for exactly this: absent means the export never said,
    // which is a different fact from "it said no".
    export.roster_capped = None;

    export
        .msgs
        .sort_by(|a, b| a.unix.cmp(&b.unix).then_with(|| a.id.cmp(&b.id)));
    Ok(export)
}

/// The roster, out of the `participants` table, ordered by peer key.
///
/// A roster that cannot be read is an empty one rather than a failed report,
/// as it always was here: the figures stand without it, and the report says
/// when there is no member list.
async fn roster(store: &Store, chat_id: i64) -> Vec<tga_read::Member> {
    store
        .participants_of(chat_id)
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(tga_read::member)
        .collect()
}

/// One boolean out of any of a chat's topic heads.
///
/// Any of them: the exporter writes the same roster block into every topic's
/// head of a chat, so the first that carries the key answers for all.
///
/// Read by [`tga_read::flag`], the rule `participants.json` is read by. This
/// had its own, under which `0` was yes, so one export said its member list was
/// complete from a database and incomplete from a folder.
fn head_flag(heads: &[tgx_archive::StoredTopic], key: &str) -> Option<bool> {
    heads.iter().find_map(|t| tga_read::flag(t.head.get(key)))
}

/// What the file says about messages Telegram no longer returns.
///
/// Not in the [`Export`] yet — the model has nowhere to put it and the report
/// has no section for it. Exposed so a caller can say the number out loud
/// rather than let it pass unmentioned, because "17 of these were deleted and
/// kept" is the one thing this format knows that a folder never could.
pub fn deleted(path: &Path, chat_id: i64) -> Result<usize> {
    Reader::open(path)?.deleted(chat_id)
}

/// How many stored messages have an earlier version kept, and how many
/// versions there are in total.
pub fn edits(path: &Path, chat_id: i64) -> Result<(usize, usize)> {
    Reader::open(path)?.edits(chat_id)
}

/// The bytes of one stored file, if the database has them.
///
/// Keyed `(file_id, kind)` because that is the shipped schema's primary key on
/// `blobs`; `media` is what maps a message to the pair. Once per file is
/// [`Reader::blob`]'s job; this opens the database for the one.
pub fn blob(path: &Path, file_id: i64, kind: &str) -> Result<Option<Vec<u8>>> {
    Reader::open(path)?.blob(file_id, kind)
}

/// `(message_id, role) -> (file_id, kind)` for one chat.
///
/// The link table, read whole. It is one row per message that carries media —
/// 263 rows for a 7,077-message archive — so a map costs less than a query per
/// message.
///
/// What the digest joins on to read a document out of `blobs` with
/// `tga_docs::read_bytes` (`tga_run::cli::Stored`). The key is `(file_id,
/// kind)` — the shipped schema's, which is *not* the `(family, file_id)` the
/// exporter's own `DATABASE.md` specifies.
pub fn media_index(path: &Path, chat_id: i64) -> Result<HashMap<(i64, String), (i64, String)>> {
    Reader::open(path)?.media_index(chat_id)
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
    ///
    /// **The database is written by the exporter's own `Store`**, not by SQL
    /// in this file. It used to be built here with a hand-copied schema, which
    /// proved the reader agreed with that copy and nothing else.
    struct Both {
        folder: PathBuf,
        db: PathBuf,
    }

    const CHAT: i64 = 3586682625;

    fn header() -> Value {
        json!({
            "name": "ćaskanje",
            "type": "private_supergroup",
            "id": CHAT,
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

    /// The fixture's async half, run on the same kind of runtime [`block`]
    /// builds.
    fn run<T>(work: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(work)
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
        run(async {
            let store = Store::open(&db).await.expect("create database");
            store
                .upsert_chat(CHAT, "UA KOLAB", "private_supergroup", 0)
                .await
                .expect("chat");
            store
                .upsert_topic(CHAT, 12, "ćaskanje", head.as_object().expect("head"), 0)
                .await
                .expect("topic");
            for m in messages() {
                let id = m["id"].as_i64().expect("id");
                store
                    .merge(CHAT, id, Some(12), m.as_object().expect("message"), 0)
                    .await
                    .expect("message");
            }
            for p in members() {
                store
                    .upsert_participant(CHAT, p["id"].as_str().expect("peer"), &p, 0)
                    .await
                    .expect("participant");
            }
        });

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
    fn one_reader_answers_what_the_one_shot_functions_answer() {
        let both = build("reader");
        let reader = Reader::open(&both.db).expect("open");
        let once = load(&both.db, None, None).expect("load");
        let held = reader.load(None, None).expect("load");
        assert_eq!(
            held.msgs.iter().map(|m| m.id).collect::<Vec<_>>(),
            once.msgs.iter().map(|m| m.id).collect::<Vec<_>>()
        );
        assert_eq!(held.name, once.name);
        assert_eq!(reader.chats().expect("chats").len(), 1);
        assert_eq!(
            reader.media_index(CHAT).expect("index"),
            media_index(&both.db, CHAT).expect("index")
        );
        assert_eq!(reader.deleted(CHAT).expect("deleted"), 0);
        assert_eq!(
            reader.edits(CHAT).expect("edits"),
            edits(&both.db, CHAT).expect("edits")
        );
        assert_eq!(reader.blob(1, "document").expect("blob"), None);
        assert_eq!(reader.path(), both.db.as_path());
    }

    #[test]
    fn a_zero_complete_flag_reads_as_no_from_a_database_as_from_a_folder() {
        // `members_complete: 0` was yes here and no from `participants.json`.
        let base = std::env::temp_dir().join("tga-db-test-flag");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("temp dir");
        let db = base.join(FILE_NAME);
        run(async {
            let store = Store::open(&db).await.expect("create database");
            store
                .upsert_chat(CHAT, "UA KOLAB", "private_supergroup", 0)
                .await
                .expect("chat");
            let head = json!({"topic_id": 12, "members_complete": 0});
            store
                .upsert_topic(CHAT, 12, "t", head.as_object().expect("head"), 0)
                .await
                .expect("topic");
        });
        assert_eq!(
            load(&db, None, None).expect("database").roster_complete,
            Some(false)
        );
        let _ = std::fs::remove_dir_all(&base);
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
        run(async {
            let store = Store::open(&both.db).await.expect("open");
            store.mark_deleted(CHAT, 13, 99).await.expect("mark");
        });

        // Keeping what Telegram dropped is the whole point of the format.
        // Filtering it out here would make the archive agree with Telegram.
        let export = load(&both.db, None, None).expect("database");
        assert_eq!(export.msgs.len(), 4);
        assert!(export.msgs.iter().any(|m| m.id == 13));
        assert_eq!(deleted(&both.db, CHAT).expect("count"), 1);
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
        // No `Store` method writes a future schema version, which is the
        // point of the refusal; so this one write goes straight to libsql.
        run(async {
            let db = libsql::Builder::new_local(&both.db)
                .build()
                .await
                .expect("open");
            db.connect()
                .expect("connect")
                .execute(
                    "UPDATE meta SET value = '2' WHERE key = 'schema_version'",
                    (),
                )
                .await
                .expect("bump");
        });
        let err = load(&both.db, None, None).expect_err("refused");
        assert!(format!("{err:#}").contains("schema version 2"), "{err:#}");
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

    /// Read-only, because the exporter may be writing this very file and an
    /// analyser has no business changing somebody's archive.
    ///
    /// **This used to assert that no `-wal` or `-shm` appeared, and that was
    /// never true of a real export.** The old fixture was built by hand in
    /// SQLite's rollback mode; the exporter writes WAL, and SQLite cannot let
    /// even a read-only connection into a WAL database without the shared-memory
    /// index beside it, so it creates `-shm` and an empty `-wal` on the way in.
    /// Every database in the corpus folder already carried the pair from the
    /// rusqlite reader before this one existed. Now that the fixture is written
    /// by the exporter's own `Store` it is WAL too, and the old assertion failed
    /// on its first honest run.
    ///
    /// What a read must not do is the thing the rule was for: change a byte of
    /// the database, or put anything into its log.
    #[test]
    fn reading_a_database_changes_nothing_in_it() {
        let both = build("readonly");
        let before = std::fs::read(&both.db).expect("database bytes");
        let _ = load(&both.db, None, None).expect("database");
        let _ = chats(&both.db).expect("chats");
        assert!(
            std::fs::read(&both.db).expect("database bytes") == before,
            "the database file changed"
        );
        let wal = both.db.with_extension("sqlite-wal");
        let logged = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert_eq!(logged, 0, "a read wrote {logged} bytes into the WAL");
    }
}
