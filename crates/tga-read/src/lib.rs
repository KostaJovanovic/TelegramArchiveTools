//! Load a finished export off disk into a model the metrics can walk.
//!
//! A port of `analyser/read.py` from the Python original, the old
//! `Projekti\telegram` project. The only input is a folder.
//! `result.json` files are found wherever they sit — that app writes one per
//! topic directly under the export root, while Telegram Desktop writes
//! `chats/chat_<id>/result.json` — so both layouts load without a flag.
//!
//! **Two decisions are made once, here, and every metric inherits them.**
//!
//! *Time is the export's local wall clock.* Desktop writes `date` as a naive
//! local timestamp and `date_unixtime` as UTC epoch seconds. Anything with a
//! calendar or a clock face on it — hour of day, weekday, messages per day —
//! reads [`Msg::when`], because "who posts at 3am" is a question about the
//! clock on the wall, not about UTC. Anything measuring a *duration* reads
//! [`Msg::unix`], which stays monotonic across a DST change where `when` does
//! not.
//!
//! *A sender is a typed peer key, never a name.* `from_id` is stable across
//! renames and `from` is only what that person was called at the time. Both
//! are kept; deciding what to do with them belongs to identity, not here.
//!
//! Nothing here reaches the network.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use serde_json::Value;

pub mod text;
pub use text::{domain_of, emoji_runs};

/// What Desktop writes into `file` or `photo` for an attachment it has no
/// bytes for.
///
/// **The prefix of all of them, not of one.** It was `"(File exceeds"`, which
/// is only the over-the-size-limit apology. Desktop and the exporter also write
/// `"(File not included. Change data exporting settings to download.)"` when
/// media is off and `"(File unavailable, please try again later)"` when a fetch
/// failed, and those read as saved files with the apology for a path.
/// `tgx-archive`'s `"(File "` rule is the same test; copied rather than
/// imported, because no `tga-*` crate may reach a `tgx-*` one beyond the
/// archive's reader.
const SKIPPED_PREFIX: &str = "(File ";

// ---------------------------------------------------------------------------
// the model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Reaction {
    pub emoji: String,
    pub count: i64,
    /// Peer keys of the reactors Telegram actually named — never the whole
    /// list. It caps at three per *message* unless the export asked for the
    /// full one, and people who react anonymously are in it at no setting.
    pub named: Vec<String>,
}

/// Desktop's `poll` object: the question, and each answer with its tally.
///
/// **Tallies, never voters.** Desktop records how many chose each answer and
/// whether the exporting account was one of them, and nothing about who the
/// others were — so nothing downstream can say who voted for what.
#[derive(Debug, Clone, Default)]
pub struct Poll {
    pub question: String,
    pub closed: bool,
    pub total_voters: i64,
    pub answers: Vec<PollAnswer>,
}

#[derive(Debug, Clone, Default)]
pub struct PollAnswer {
    pub text: String,
    pub voters: i64,
}

#[derive(Debug, Clone)]
pub struct Msg {
    pub id: i64,
    pub topic: usize,
    pub when: NaiveDateTime,
    pub unix: i64,
    pub service: bool,
    pub action: String,
    pub sender: String,
    pub name: String,
    pub text: String,
    pub chars: usize,
    pub words: usize,
    pub reply_to: Option<i64>,
    pub forward_from: String,
    pub forward_id: String,
    pub media: String,
    /// The attachment's path, relative to the topic's own folder, and empty
    /// when nothing was saved.
    ///
    /// Not the same string as [`file_name`](Self::file_name): the exporter
    /// deduplicates, so `Zapisnik-KRGM-16-07-2026.docx` sent twice is
    /// `files/Zapisnik-KRGM-16-07-2026 (2).docx` on disk the second time.
    pub file: String,
    /// The name the sender's own machine gave the file.
    ///
    /// **Carried, where it used to be dropped.** The digest threw filenames
    /// away on the grounds that they tell a model nothing -- true of the
    /// exporter's own `photo_2649@07-01-2026_20-19-54.jpg`, and false of every
    /// document a person named. In the KRGM export the filename is where the
    /// date lives: 80 of 695 document attachments carry a full one, and the
    /// minutes named `ZAPISNIK 30.12.2024..docx` was posted 268 days after the
    /// meeting it records.
    pub file_name: String,
    pub file_size: i64,
    pub media_saved: bool,
    pub sticker_emoji: String,
    pub emoji: Vec<String>,
    pub domains: Vec<String>,
    /// Every address the message linked, as written: a link in the text, a
    /// `text_link`'s target, the link preview's page. For a summary that
    /// reads linked pages (`tga-links`); nothing else reads it.
    pub links: Vec<String>,
    pub mentions: Vec<String>,
    pub hashtags: Vec<String>,
    pub reactions: Vec<Reaction>,
    pub edited: bool,
    pub members: Vec<String>,
    pub grouped: Option<i64>,
    /// The poll this message carries, if it is one.
    pub poll: Option<Poll>,
    /// `duration_seconds`: how long a voice note, a round video, a video or
    /// a piece of audio runs, and 0 where Desktop wrote none.
    pub duration: i64,
    /// The name a service message gives something: the group's new title
    /// (`edit_group_title`), the old group's (`migrate_from_group`), a new
    /// topic's (`topic_created`) or a renamed one's (`topic_edit`). Empty
    /// otherwise.
    pub title: String,
    /// The message a `pin_message` pinned, when the export says which.
    pub pinned: Option<i64>,
}

impl Msg {
    pub fn reaction_total(&self) -> i64 {
        self.reactions.iter().map(|r| r.count).sum()
    }
}

#[derive(Debug, Clone)]
pub struct Member {
    pub key: String,
    pub name: String,
    pub role: String,
    pub joined: Option<NaiveDateTime>,
    /// The `@handle`, without the `@`. Empty when the account has none, which
    /// is ordinary — two of UA KOLAB's 43 members have never set one.
    ///
    /// Worth carrying because it is the only *stable* thing about a person: a
    /// display name changes whenever they feel like it and the numeric key
    /// means nothing to a reader, so the handle is what lets somebody looking
    /// at the report recognise who they are looking at.
    pub username: String,
    /// Whether Telegram says this account is a bot.
    ///
    /// A bot on the member list is not a person who never spoke, and counting
    /// it among them makes the silence look more sociable than it was.
    pub bot: bool,
}

#[derive(Debug, Clone)]
pub struct Topic {
    pub index: usize,
    pub name: String,
    pub folder: PathBuf,
    pub chat_id: Option<i64>,
    pub chat_type: String,
    pub created: Option<NaiveDateTime>,
    pub messages: usize,
    /// The topic's root message id. Every top-level message in a forum topic
    /// is sent as a reply to it — see [`one`].
    pub root: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct Export {
    pub root: PathBuf,
    pub name: String,
    pub topics: Vec<Topic>,
    pub msgs: Vec<Msg>,
    pub roster: Vec<Member>,
    /// `None` when the export wrote no roster at all, which is a different
    /// thing from a roster it knows is short.
    pub roster_complete: Option<bool>,
    /// The exporter's own `capped` flag: it stopped collecting members before
    /// it ran out of them.
    ///
    /// Kept apart from `roster_complete` rather than folded into it, because
    /// the two say different things about *why* a list is short — one is "I
    /// know I missed some", the other is "I stopped at a limit" — and the
    /// report can only pass that on if it still has both.
    pub roster_capped: Option<bool>,
    pub members_count: Option<i64>,
}

impl Export {
    /// Message id -> index into `msgs`. Ids are chat-global, so this spans
    /// topics.
    pub fn by_id(&self) -> std::collections::HashMap<i64, usize> {
        self.msgs
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect()
    }

    /// Real messages only — a service entry is not somebody talking.
    pub fn said(&self) -> impl Iterator<Item = &Msg> {
        self.msgs.iter().filter(|m| !m.service)
    }

    /// Show everybody who has an `@handle` by it, instead of by name.
    ///
    /// **A display name in an export may not be the one the person chose.**
    /// For anyone the exporting account has saved, Telegram replaces their
    /// `first_name`/`last_name` with the address-book entry — "Mama", "Marko
    /// plumber" — and sends no second field with their own. The handle is the
    /// only identifier guaranteed to be theirs. The exporter's `own_names`
    /// makes the same substitution at export time; this is the same rule for
    /// an export written without it.
    ///
    /// **Somebody with no handle keeps the name in the export**, which is their
    /// Telegram name unless they are one of your contacts — there is nothing
    /// else to show, and a peer key is worse than either. Their renames
    /// survive; a person with a handle has one name from here on, so theirs
    /// do not.
    ///
    /// Done here, on the model, rather than in the report: names reach the
    /// figures through a dozen paths — senders, the roster, renames, forwards,
    /// the names a join announcement lists — and rewriting them once at the
    /// source is the only way none is missed. The handle comes from the member
    /// list alone; a message never carries one, so an export with no roster
    /// comes through unchanged.
    pub fn handles_only(&mut self) {
        let handle_of: std::collections::HashMap<String, String> = self
            .roster
            .iter()
            .filter(|m| !m.username.is_empty())
            .map(|m| (m.key.clone(), format!("@{}", m.username)))
            .collect();

        // A join announcement and a forward from a hidden account name people
        // by string alone, with no key. A name maps to a handle only when it
        // was only ever seen against one key, and that key has one.
        //
        // **Every key counts, handle or not.** Two accounts can show the same
        // display name, and an address book makes that likelier, not less.
        // Indexing only the handle-holders, first seen winning, handed a
        // namesake's join and forward to whoever happened to come first --
        // including to somebody else's handle when the namesake had none. A
        // name that could be either person stays the name the export wrote.
        let mut owners: std::collections::HashMap<&str, HashSet<&str>> =
            std::collections::HashMap::new();
        let names = self
            .roster
            .iter()
            .map(|m| (&m.key, &m.name))
            .chain(self.msgs.iter().map(|m| (&m.sender, &m.name)));
        for (key, name) in names {
            if !key.is_empty() && !name.is_empty() {
                owners.entry(name).or_default().insert(key);
            }
        }
        let by_name: std::collections::HashMap<String, String> = owners
            .into_iter()
            .filter_map(|(name, keys)| {
                let mut keys = keys.into_iter();
                match (keys.next(), keys.next()) {
                    (Some(only), None) => handle_of
                        .get(only)
                        .map(|handle| (name.to_string(), handle.clone())),
                    _ => None,
                }
            })
            .collect();
        let swap =
            |name: &mut String, map: &std::collections::HashMap<String, String>, key: &str| {
                if let Some(handle) = map.get(key) {
                    *name = handle.clone();
                }
            };

        for member in &mut self.roster {
            swap(&mut member.name, &handle_of, &member.key);
        }
        for msg in &mut self.msgs {
            swap(&mut msg.name, &handle_of, &msg.sender);
            for name in &mut msg.members {
                let was = name.clone();
                swap(name, &by_name, &was);
            }
            if msg.forward_id.is_empty() {
                let was = msg.forward_from.clone();
                swap(&mut msg.forward_from, &by_name, &was);
            } else {
                // A channel's key has no handle here, so its title stays.
                swap(&mut msg.forward_from, &handle_of, &msg.forward_id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// coercion
//
// Every one of these mirrors a Python idiom rather than a Rust one, because
// the shapes they meet are whatever Desktop happened to write. Where the
// Python reads `str(x or "")`, the truthiness matters: 0, false and "" all
// collapse to the empty string, and a port that only checks for null gets a
// literal "0" where the original has nothing.
// ---------------------------------------------------------------------------

/// `str(value or "")`.
fn as_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::Bool(false)) => String::new(),
        Some(Value::Bool(true)) => "True".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => {
            if n.as_f64() == Some(0.0) {
                String::new() // falsy
            } else {
                n.to_string()
            }
        }
        Some(Value::Array(a)) if a.is_empty() => String::new(),
        Some(other) => other.to_string(),
    }
}

/// `int(value)`, and 0 where Python would have raised.
fn as_int(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f.trunc() as i64))
            .unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn as_i64(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(n)) => n.as_i64(),
        _ => None,
    }
}

/// `datetime.fromisoformat`, tolerant, `None` on anything unreadable.
fn as_dt(value: Option<&Value>) -> Option<NaiveDateTime> {
    let raw = match value {
        Some(Value::String(s)) if !s.is_empty() => s.as_str(),
        _ => return None,
    };
    for fmt in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(dt);
        }
    }
    None
}

/// Desktop's `text`: a bare string, or a list of strings and entity objects.
fn plain(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => {
            let mut out = String::new();
            for part in parts {
                match part {
                    Value::String(s) => out.push_str(s),
                    Value::Object(_) => out.push_str(&as_text(part.get("text"))),
                    _ => {}
                }
            }
            out
        }
        None | Some(Value::Null) => String::new(),
        Some(other) => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// finding the files
// ---------------------------------------------------------------------------

/// Every `result.json` at or under `root`, shallowest first.
///
/// Bounded, because an export folder holds tens of thousands of media files
/// and an unbounded walk pays for all of them to find at most a handful of
/// JSON files three levels down.
///
/// Order decides [`Topic::index`], which every downstream figure is keyed on,
/// so it is not incidental: shallowest first, and within a depth, by the
/// lowercased path. That is what Python's `sorted()` over `WindowsPath` does,
/// since `PureWindowsPath` compares case-insensitively.
pub fn find_results(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut found: Vec<(usize, String, PathBuf)> = Vec::new();

    let direct = root.join("result.json");
    if direct.is_file() {
        found.push((0, String::new(), direct));
    }

    let mut frontier: Vec<PathBuf> = vec![root.to_path_buf()];
    for depth in 1..=max_depth {
        let mut next = Vec::new();
        for dir in &frontier {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                // glob's `*` skips dotfiles, and so does this.
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
                {
                    continue;
                }
                let candidate = path.join("result.json");
                if candidate.is_file() {
                    let sort_key = candidate.to_string_lossy().to_lowercase();
                    found.push((depth, sort_key, candidate));
                }
                next.push(path);
            }
        }
        frontier = next;
    }

    found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut seen: HashSet<PathBuf> = HashSet::new();
    found
        .into_iter()
        .map(|(_, _, p)| p)
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// one message
// ---------------------------------------------------------------------------

fn entities(raw: &Value, out: &mut Msg) {
    if let Some(Value::Array(list)) = raw.get("text_entities") {
        for ent in list {
            if !ent.is_object() {
                continue;
            }
            let kind = as_text(ent.get("type"));
            let text = as_text(ent.get("text"));
            match kind.as_str() {
                "link" => {
                    out.domains.push(domain_of(&text));
                    out.links.push(text);
                }
                "text_link" => {
                    let href = as_text(ent.get("href"));
                    out.domains.push(domain_of(&href));
                    out.links.push(href);
                }
                "mention" => out
                    .mentions
                    .push(text.trim_start_matches('@').to_lowercase()),
                "mention_name" => {
                    let uid = ent.get("user_id");
                    let named = as_text(uid);
                    out.mentions.push(if named.is_empty() {
                        text.to_lowercase()
                    } else {
                        format!("user{named}")
                    });
                }
                "hashtag" => out
                    .hashtags
                    .push(text.trim_start_matches('#').to_lowercase()),
                _ => {}
            }
        }
    }
    if let Some(preview) = raw.get("link_preview") {
        if preview.is_object() {
            let url = as_text(preview.get("url"));
            if !url.is_empty() {
                let host = domain_of(&url);
                if !host.is_empty() && !out.domains.contains(&host) {
                    out.domains.push(host);
                }
                if !out.links.contains(&url) {
                    out.links.push(url);
                }
            }
        }
    }
}

/// The shape of the attachment, normalised.
///
/// `media_type` is Telegram's own answer and wins when it is there. A photo
/// carries no `media_type` at all, so the `photo` key and then the MIME major
/// type stand in for it.
fn media_kind(raw: &Value) -> String {
    let declared = as_text(raw.get("media_type"));
    if !declared.is_empty() {
        return declared;
    }
    if !as_text(raw.get("photo")).is_empty() {
        return "photo".to_string();
    }
    let mime = as_text(raw.get("mime_type"));
    if mime.starts_with("image/") {
        return "photo".to_string();
    }
    if mime.starts_with("video/") {
        return "video_file".to_string();
    }
    if mime.starts_with("audio/") {
        return "audio_file".to_string();
    }
    if !as_text(raw.get("file")).is_empty() || !as_text(raw.get("file_name")).is_empty() {
        return "document".to_string();
    }
    String::new()
}

/// One message object into a [`Msg`], or `None` if it carries no date.
///
/// `root` is the topic's own id, and it is the reason this takes an extra
/// argument. **A forum topic is a thread**, so Telegram sends every top-level
/// message in one as a reply to the message that opened it. Read literally,
/// that makes 2,702 of the 3,518 "replies" in the UA KOLAB export answers to a
/// single message, gives one topic root 2,463 replies, and drags the median
/// response time out to days. Those are not replies and they are dropped here,
/// once, so no metric downstream has to know about it.
///
/// The General topic is exempt because it is not a real thread — its messages
/// carry no `reply_to_message_id` at all, which is the same reason
/// `messages.getReplies` returns nothing for it.
pub fn one(raw: &Value, topic: usize, root: Option<i64>) -> Option<Msg> {
    let when = as_dt(raw.get("date"))?;
    let service = as_text(raw.get("type")) == "service";
    let text = plain(raw.get("text"));

    let parent = match as_i64(raw.get("reply_to_message_id")) {
        Some(p) if Some(p) != root => Some(p),
        _ => None,
    };

    let unix = {
        let declared = as_int(raw.get("date_unixtime"));
        if declared != 0 {
            declared
        } else {
            // Python falls back to `when.timestamp()`, which reads a naive
            // datetime as *local* time. Desktop always writes date_unixtime,
            // so this path effectively never runs on a real export.
            use chrono::TimeZone;
            chrono::Local
                .from_local_datetime(&when)
                .earliest()
                .map(|dt| dt.timestamp())
                .unwrap_or(0)
        }
    };

    let mut out = Msg {
        id: as_int(raw.get("id")),
        topic,
        when,
        unix,
        service,
        action: as_text(raw.get("action")),
        sender: as_text(raw.get(if service { "actor_id" } else { "from_id" })),
        name: as_text(raw.get(if service { "actor" } else { "from" })),
        // `len()` on a Python str counts code points, not bytes.
        chars: text.chars().count(),
        // `str.split()` with no argument splits on whitespace runs and drops
        // the empties.
        words: text.split_whitespace().count(),
        emoji: emoji_runs(&text),
        text,
        reply_to: parent,
        forward_from: as_text(raw.get("forwarded_from")),
        forward_id: as_text(raw.get("forwarded_from_id")),
        media: media_kind(raw),
        file: String::new(),
        file_name: as_text(raw.get("file_name")),
        file_size: 0,
        media_saved: false,
        sticker_emoji: as_text(raw.get("sticker_emoji")),
        domains: Vec::new(),
        links: Vec::new(),
        mentions: Vec::new(),
        hashtags: Vec::new(),
        reactions: Vec::new(),
        edited: matches!(raw.get("edited"), Some(v) if !v.is_null() && v != &Value::Bool(false)),
        members: Vec::new(),
        grouped: as_i64(raw.get("grouped_id")),
        poll: raw.get("poll").and_then(poll),
        duration: as_int(raw.get("duration_seconds")),
        // Service messages only: an audio file carries a `title` of its own,
        // beside its `performer`. `topic_edit` names the new title
        // `new_title`; the other three call it `title`.
        title: match service {
            false => String::new(),
            true => match as_text(raw.get("title")) {
                title if title.is_empty() => as_text(raw.get("new_title")),
                title => title,
            },
        },
        pinned: service.then(|| as_i64(raw.get("message_id"))).flatten(),
    };

    entities(raw, &mut out);

    // `file_size or photo_file_size` — a zero is falsy, so it falls through.
    out.file_size = match as_int(raw.get("file_size")) {
        0 => as_int(raw.get("photo_file_size")),
        size => size,
    };
    let path = {
        let file = as_text(raw.get("file"));
        if file.is_empty() {
            as_text(raw.get("photo"))
        } else {
            file
        }
    };
    out.media_saved = !path.is_empty() && !path.starts_with(SKIPPED_PREFIX);
    // Only when it is really on disk. `path` otherwise holds the exporter's
    // apology -- "(File exceeds maximum size...)", "(File not included...)" --
    // and joining that onto the topic folder makes a path that looks real and
    // is not.
    if out.media_saved {
        out.file = path;
    }

    if let Some(Value::Array(list)) = raw.get("reactions") {
        for entry in list {
            if !entry.is_object() {
                continue;
            }
            let emoji = {
                let e = as_text(entry.get("emoji"));
                if !e.is_empty() {
                    e
                } else {
                    let d = as_text(entry.get("document_id"));
                    if d.is_empty() {
                        "?".to_string()
                    } else {
                        d
                    }
                }
            };
            let named = match entry.get("recent") {
                Some(Value::Array(recent)) => recent
                    .iter()
                    .filter(|r| r.is_object())
                    .map(|r| as_text(r.get("from_id")))
                    .filter(|s| !s.is_empty())
                    .collect(),
                _ => Vec::new(),
            };
            out.reactions.push(Reaction {
                emoji,
                count: as_int(entry.get("count")),
                named,
            });
        }
    }

    if let Some(Value::Array(members)) = raw.get("members") {
        out.members = members.iter().map(|m| as_text(Some(m))).collect();
    }

    Some(out)
}

/// A `poll` object, or `None` if it is not one.
///
/// The question and answers go through [`plain`] rather than being read as
/// strings, so a later Desktop that writes them with entities, the way it
/// writes `text`, still reads.
fn poll(raw: &Value) -> Option<Poll> {
    if !raw.is_object() {
        return None;
    }
    let answers = match raw.get("answers") {
        Some(Value::Array(list)) => list
            .iter()
            .filter(|a| a.is_object())
            .map(|a| PollAnswer {
                text: plain(a.get("text")),
                voters: as_int(a.get("voters")),
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(Poll {
        question: plain(raw.get("question")),
        closed: matches!(raw.get("closed"), Some(Value::Bool(true))),
        total_voters: as_int(raw.get("total_voters")),
        answers,
    })
}

// ---------------------------------------------------------------------------
// the roster
// ---------------------------------------------------------------------------

/// One entry of `participants.json`'s `members` array, or `None` if it has no
/// id to key it by.
///
/// **Public because a database export has the same rows in a table** and must
/// not grow a second copy of this: the `@` strip and the empty-role default are
/// exactly the kind of detail two readers drift apart on.
pub fn member(entry: &Value) -> Option<Member> {
    if !entry.is_object() {
        return None;
    }
    let key = as_text(entry.get("id"));
    if key.is_empty() {
        return None;
    }
    Some(Member {
        key,
        name: as_text(entry.get("name")),
        role: match as_text(entry.get("role")) {
            r if r.is_empty() => "member".to_string(),
            r => r,
        },
        joined: as_dt(entry.get("joined")),
        // Written with no `@` by the exporter; strip one anyway, so a
        // hand-edited file that added it does not end up rendering `@@name`.
        username: as_text(entry.get("username"))
            .trim_start_matches('@')
            .to_string(),
        bot: entry.get("bot") == Some(&Value::Bool(true)),
    })
}

/// The [`Topic`] a `result.json` header describes, before a message is counted.
///
/// **Public for the same reason as [`member`].** A database keeps the header as
/// a JSON map in `topics.head`, so the database reader rebuilds that map and
/// hands it here rather than reimplementing which key means what — including
/// `topic_id`, which is [`Topic::root`] and carries the thread rule with it.
pub fn header_topic(body: &Value, index: usize, folder: PathBuf) -> Topic {
    Topic {
        index,
        name: match as_text(body.get("name")) {
            n if n.is_empty() => folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            n => n,
        },
        folder,
        chat_id: as_i64(body.get("id")),
        chat_type: as_text(body.get("type")),
        created: as_dt(body.get("topic_created")),
        messages: 0,
        root: as_i64(body.get("topic_id")),
    }
}

/// Give a message with no sender the one its chat implies.
///
/// **In a 1-on-1 chat or a channel the sender can be the chat itself.**
/// Telegram leaves `from_id` off a message the other person sent you, and off
/// a channel's post, because the chat already says who it is. Desktop writes
/// the chat as `from`/`from_id` there; the exporter did not until 2026-10-15,
/// so a 66,219-message private chat read as one person talking to nobody, and
/// a database never rewrites a stored message's sender on a later sync. So
/// it is filled in here, on every reader's way in, and only where it is empty.
///
/// A group gets nothing: a message there with no sender has no one to imply.
///
/// **Public for the same reason as [`header_topic`]:** the database reader
/// applies it too, with the chat's own title as the name.
pub fn imply_senders(msgs: &mut [Msg], topic: &Topic, chat_name: &str) {
    let prefix = match topic.chat_type.as_str() {
        "personal_chat" | "bot_chat" => "user",
        "public_channel" | "private_channel" => "channel",
        _ => return,
    };
    let Some(id) = topic.chat_id else {
        return;
    };
    let key = format!("{prefix}{id}");
    for msg in msgs.iter_mut().filter(|m| m.sender.is_empty()) {
        msg.sender = key.clone();
        if msg.name.is_empty() {
            msg.name = chat_name.to_string();
        }
    }
}

fn roster(root: &Path) -> (Vec<Member>, Option<bool>, Option<bool>) {
    let path = root.join("participants.json");
    if !path.is_file() {
        return (Vec::new(), None, None);
    }
    let body: Value = match std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(body) => body,
        None => return (Vec::new(), None, None),
    };

    let mut people = Vec::new();
    if let Some(Value::Array(list)) = body.get("members") {
        people.extend(list.iter().filter_map(member));
    }

    (people, flag(body.get("complete")), flag(body.get("capped")))
}

/// A roster flag — `complete`, `capped` — as the export wrote it.
///
/// Absent is not the same as `false`. A roster the export never wrote and a
/// roster it knows is short look identical downstream unless this distinction
/// survives, so absent and `null` are `None`. `false` and `0` are both no.
///
/// **Public because a database carries the same flag** in its topic heads, and
/// a second reading of it is how `members_complete: 0` came to mean yes from a
/// database and no from a folder.
pub fn flag(value: Option<&Value>) -> Option<bool> {
    match value {
        None | Some(Value::Null) => None,
        Some(v) => Some(v != &Value::Bool(false) && v != &serde_json::json!(0)),
    }
}

// ---------------------------------------------------------------------------
// load
// ---------------------------------------------------------------------------

/// One `result.json`'s messages into `into`, converted as they are parsed, and
/// everything else in it handed back as the header.
///
/// **Streamed, because the file is the size of the chat.** Parsing it into a
/// `Value` first held the text and the whole tree of every message at once —
/// several times the file, for a topic of 450,000 messages — before the first
/// [`Msg`] existed. This holds the bytes and one message's tree at a time.
///
/// Every message is converted with no topic root, because the header that
/// names the root need not come before `messages` in the file; [`load`]
/// applies the thread rule once the header is known.
fn stream(
    bytes: &[u8],
    topic: usize,
    into: &mut Vec<Msg>,
) -> serde_json::Result<serde_json::Map<String, Value>> {
    use serde::de::DeserializeSeed;

    let mut de = serde_json::Deserializer::from_slice(bytes);
    let header = Body { topic, into }.deserialize(&mut de)?;
    // What `from_str` checks too: nothing but whitespace after the value.
    de.end()?;
    Ok(header)
}

/// The top level of a `result.json`.
struct Body<'a> {
    topic: usize,
    into: &'a mut Vec<Msg>,
}

impl<'de> serde::de::DeserializeSeed<'de> for Body<'_> {
    type Value = serde_json::Map<String, Value>;

    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        de.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for Body<'_> {
    type Value = serde_json::Map<String, Value>;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a result.json")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let start = self.into.len();
        let mut header = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if key == "messages" {
                // A repeated key: the last one wins, as it did in a `Value`.
                self.into.truncate(start);
                map.next_value_seed(Messages {
                    topic: self.topic,
                    into: &mut *self.into,
                })?;
            } else {
                header.insert(key, map.next_value()?);
            }
        }
        Ok(header)
    }

    // Anything that is not an object has no header and no messages, which is
    // what `Value::get` answered on one.
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
        Ok(serde_json::Map::new())
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(serde_json::Map::new())
    }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(serde_json::Map::new())
    }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(serde_json::Map::new())
    }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(serde_json::Map::new())
    }

    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(serde_json::Map::new())
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Map::new())
    }
}

/// The `messages` array, one element at a time.
struct Messages<'a> {
    topic: usize,
    into: &'a mut Vec<Msg>,
}

impl<'de> serde::de::DeserializeSeed<'de> for Messages<'_> {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for Messages<'_> {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a list of messages")
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while let Some(entry) = seq.next_element::<Value>()? {
            if !entry.is_object() {
                continue;
            }
            if let Some(msg) = one(&entry, self.topic, None) {
                self.into.push(msg);
            }
        }
        Ok(())
    }

    // `messages` that is not a list holds none.
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while map
            .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
            .is_some()
        {}
        Ok(())
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}

/// Called as `(done, total, label)` after each file is read.
///
/// Borrowed rather than owned so the caller can keep whatever it is writing
/// into — a CLI's stdout, or a window's progress bar — and `Option` rather
/// than a no-op default so a caller that wants nothing pays nothing.
pub type Progress<'a> = &'a mut dyn FnMut(usize, usize, &str);

/// Read every `result.json` under `root` into one [`Export`], refusing a
/// folder that holds more than one chat.
///
/// `progress` gives a window something to move while a large export is parsed:
/// the KRGM corpus is 333,582 messages, and every one of them is read before
/// the first figure can be computed.
pub fn load(root: &Path, mut progress: Option<Progress<'_>>) -> Result<Export> {
    let files = find_results(root, 3);
    anyhow::ensure!(!files.is_empty(), "No result.json under {}", root.display());

    let mut export = Export {
        root: root.to_path_buf(),
        name: root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Default::default()
    };
    (export.roster, export.roster_complete, export.roster_capped) = roster(root);

    let total = files.len();
    for (index, path) in files.iter().enumerate() {
        let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let start = export.msgs.len();
        let body = Value::Object(
            stream(&raw, index, &mut export.msgs)
                .with_context(|| format!("parsing {}", path.display()))?,
        );
        drop(raw);

        let folder = path.parent().unwrap_or(root).to_path_buf();
        let mut topic = header_topic(&body, index, folder);

        // **One chat per call.** A folder of several chats' `result.json`s —
        // Desktop's `chats/chat_<id>/` layout, taken for more than one chat —
        // would otherwise load as one group whose topics are chats, and
        // [`Export::by_id`] is keyed on the bare message id, which is only
        // unique inside a chat: chat A's reply to message 17 would land on
        // chat B's 17. A database with several chats is read one at a time
        // for the same reason.
        if let Some(id) = topic.chat_id {
            if let Some(other) = export
                .topics
                .iter()
                .find(|t| t.chat_id.is_some_and(|seen| seen != id))
            {
                anyhow::bail!(
                    "{} holds more than one chat: {} ({}) and {} ({}). \
                     Point the analyser at one chat's folder.",
                    root.display(),
                    other.chat_id.unwrap_or_default(),
                    other.folder.display(),
                    id,
                    topic.folder.display()
                );
            }
        }

        // The thread rule, applied after the fact: the messages were converted
        // as they streamed past, and `topic_id` need not come before them in
        // the file. `one` with no root keeps every reply, so dropping the ones
        // aimed at the root here is the same answer `one` with the root gives.
        for msg in &mut export.msgs[start..] {
            if topic.root.is_some() && msg.reply_to == topic.root {
                msg.reply_to = None;
            }
        }
        topic.messages = export.msgs.len() - start;
        imply_senders(
            &mut export.msgs[start..],
            &topic,
            &as_text(body.get("name")),
        );

        if export.members_count.is_none() {
            export.members_count = as_i64(body.get("members_count"));
        }

        let label = topic.name.clone();
        export.topics.push(topic);
        if let Some(report) = progress.as_deref_mut() {
            report(index + 1, total, &label);
        }
    }

    export
        .msgs
        .sort_by(|a, b| a.unix.cmp(&b.unix).then_with(|| a.id.cmp(&b.id)));
    Ok(export)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(extra: Value) -> Value {
        let mut base = json!({
            "id": 10, "type": "message", "date": "2026-08-18T22:59:03",
            "date_unixtime": "1786000000", "from": "Someone", "from_id": "user1",
            "text": ""
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        base
    }

    #[test]
    fn a_reply_to_the_topic_root_is_not_a_reply() {
        // The whole reason `one` takes a root: every top-level message in a
        // forum topic is sent as a reply to the message that opened it.
        let raw = msg(json!({"reply_to_message_id": 2}));
        assert_eq!(one(&raw, 0, Some(2)).unwrap().reply_to, None);
        assert_eq!(one(&raw, 0, Some(99)).unwrap().reply_to, Some(2));
        assert_eq!(one(&raw, 0, None).unwrap().reply_to, Some(2));
    }

    #[test]
    fn handles_only_swaps_a_saved_name_for_the_handle_and_leaves_the_rest() {
        let member = |key: &str, name: &str, username: &str| {
            member(&json!({"id": key, "name": name, "username": username})).unwrap()
        };
        let mut export = Export {
            roster: vec![
                member("user1", "Mama", "ana"),
                member("user2", "Bob Jones", ""),
            ],
            msgs: vec![
                // An older name of user1's, which must not survive as an alias.
                one(&msg(json!({"from": "Ana K", "from_id": "user1"})), 0, None).unwrap(),
                one(
                    &msg(json!({"from": "Bob Jones", "from_id": "user2"})),
                    0,
                    None,
                )
                .unwrap(),
                one(
                    &msg(json!({"forwarded_from": "Mama", "forwarded_from_id": "user1"})),
                    0,
                    None,
                )
                .unwrap(),
                one(
                    &msg(json!({"forwarded_from": "News", "forwarded_from_id": "channel9"})),
                    0,
                    None,
                )
                .unwrap(),
                one(
                    &msg(json!({"type": "service", "action": "invite_members",
                                "actor": "Mama", "actor_id": "user1",
                                "members": ["Ana K", "Bob Jones"]})),
                    0,
                    None,
                )
                .unwrap(),
            ],
            ..Default::default()
        };
        export.handles_only();

        assert_eq!(export.roster[0].name, "@ana");
        assert_eq!(export.roster[1].name, "Bob Jones", "no handle: name kept");
        assert_eq!(export.msgs[0].name, "@ana");
        assert_eq!(export.msgs[1].name, "Bob Jones");
        assert_eq!(export.msgs[2].forward_from, "@ana");
        assert_eq!(export.msgs[3].forward_from, "News", "a channel is public");
        assert_eq!(export.msgs[4].name, "@ana");
        assert_eq!(export.msgs[4].members, vec!["@ana", "Bob Jones"]);
    }

    #[test]
    fn handles_only_leaves_a_name_two_people_share_as_it_was() {
        let member = |key: &str, name: &str, username: &str| {
            member(&json!({"id": key, "name": name, "username": username})).unwrap()
        };
        let mut export = Export {
            roster: vec![
                // Two accounts called Ana, only one with a handle.
                member("user1", "Ana", "ana"),
                member("user2", "Ana", ""),
                // Two with handles, both called Marko.
                member("user3", "Marko", "marko1"),
                member("user4", "Marko", "marko2"),
                // And one name nobody else has.
                member("user5", "Vesna", "vesna"),
            ],
            msgs: vec![
                one(
                    &msg(json!({"type": "service", "action": "invite_members",
                                "actor": "Vesna", "actor_id": "user5",
                                "members": ["Ana", "Marko", "Vesna"]})),
                    0,
                    None,
                )
                .unwrap(),
                // A forward from a hidden account: a name and no key.
                one(&msg(json!({"forwarded_from": "Ana"})), 0, None).unwrap(),
            ],
            ..Default::default()
        };
        export.handles_only();

        // Either Ana could have been invited; neither is credited.
        assert_eq!(export.msgs[0].members, vec!["Ana", "Marko", "@vesna"]);
        assert_eq!(export.msgs[1].forward_from, "Ana");
        // A keyed name still swaps, namesake or not: the key says who it is.
        assert_eq!(export.roster[0].name, "@ana");
        assert_eq!(export.roster[1].name, "Ana");
        assert_eq!(export.roster[3].name, "@marko2");
    }

    /// A fresh folder under the temp dir, emptied first.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tga-read-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write(dir: &Path, body: &Value) {
        std::fs::create_dir_all(dir).expect("folder");
        std::fs::write(
            dir.join("result.json"),
            serde_json::to_string_pretty(body).expect("json"),
        )
        .expect("write result.json");
    }

    #[test]
    fn the_thread_rule_holds_wherever_the_header_sits_in_the_file() {
        // Streamed, so the messages are converted before a `topic_id` written
        // after them is known. The root must be dropped either way.
        let messages = json!([
            msg(json!({"id": 13, "reply_to_message_id": 12})),
            msg(json!({"id": 14, "reply_to_message_id": 13})),
            "not a message",
        ]);
        for (tag, text) in [
            (
                "head-first",
                format!(r#"{{"name": "t", "id": 7, "topic_id": 12, "messages": {messages}}}"#),
            ),
            (
                "head-last",
                format!(r#"{{"messages": {messages}, "name": "t", "id": 7, "topic_id": 12}}"#),
            ),
        ] {
            let dir = scratch(tag);
            std::fs::write(dir.join("result.json"), text).expect("write");
            let export = load(&dir, None).expect("load");
            let replies: Vec<Option<i64>> = export.msgs.iter().map(|m| m.reply_to).collect();
            assert_eq!(replies, [None, Some(13)], "{tag}");
            assert_eq!(export.topics[0].root, Some(12), "{tag}");
            assert_eq!(export.topics[0].messages, 2, "{tag}");
            assert_eq!(export.topics[0].chat_id, Some(7), "{tag}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_file_that_is_not_an_object_holds_nothing_and_a_broken_one_fails() {
        let dir = scratch("shapes");
        std::fs::write(dir.join("result.json"), "[1, 2]").expect("write");
        let export = load(&dir, None).expect("an array is an empty topic");
        assert!(export.msgs.is_empty());
        assert_eq!(export.topics.len(), 1);

        std::fs::write(dir.join("result.json"), r#"{"messages": [}"#).expect("write");
        let err = load(&dir, None).expect_err("malformed");
        assert!(format!("{err:#}").contains("parsing"), "{err:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_holding_two_chats_is_refused_and_both_are_named() {
        // Message ids are unique only inside a chat, so two chats loaded as one
        // would answer chat A's replies with chat B's messages.
        let dir = scratch("two-chats");
        let body = |id: i64| {
            json!({"name": "c", "type": "private_group", "id": id,
                                    "messages": [msg(json!({}))]})
        };
        write(&dir.join("chats").join("chat_1"), &body(111));
        write(&dir.join("chats").join("chat_2"), &body(222));
        let err = load(&dir, None).expect_err("two chats");
        let text = err.to_string();
        assert!(text.contains("111") && text.contains("222"), "{text}");

        // One chat's topics, which share the chat's id, still load as one.
        let dir = scratch("one-chat");
        write(&dir.join("a"), &body(111));
        write(&dir.join("b"), &body(111));
        assert_eq!(load(&dir, None).expect("one chat").topics.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("tga-read-test-two-chats"));
    }

    #[test]
    fn a_message_with_no_date_is_dropped() {
        let mut raw = msg(json!({}));
        raw["date"] = Value::Null;
        assert!(one(&raw, 0, None).is_none());
    }

    #[test]
    fn chars_counts_code_points_not_bytes() {
        // "ćaskanje" is 8 characters and 9 bytes. Getting this wrong inflates
        // every length figure for exactly the exports that are not English.
        let raw = msg(json!({"text": "ćaskanje"}));
        let out = one(&raw, 0, None).unwrap();
        assert_eq!(out.chars, 8);
        assert_eq!(out.words, 1);
    }

    #[test]
    fn a_private_chat_or_a_channel_names_the_sender_it_left_out() {
        // A database made before the converter wrote the other side of a
        // 1-on-1 chat: their messages carry no `from_id` at all.
        let topic = |kind: &str| header_topic(&json!({"type": kind, "id": 345}), 0, PathBuf::new());
        let read = |kind: &str| {
            let mut theirs = msg(json!({}));
            for key in ["from", "from_id"] {
                theirs.as_object_mut().unwrap().remove(key);
            }
            let mut msgs = vec![
                one(&msg(json!({"from": "Me", "from_id": "user12"})), 0, None).unwrap(),
                one(&theirs, 0, None).unwrap(),
            ];
            imply_senders(&mut msgs, &topic(kind), "Omar");
            msgs
        };
        let private = read("personal_chat");
        assert_eq!(private[0].sender, "user12", "a sender it has is kept");
        assert_eq!(
            (private[1].sender.as_str(), private[1].name.as_str()),
            ("user345", "Omar")
        );
        assert_eq!(read("private_channel")[1].sender, "channel345");
        assert_eq!(
            read("private_supergroup")[1].sender,
            "",
            "a group implies no one"
        );
    }

    #[test]
    fn a_skipped_file_is_not_a_saved_one() {
        let saved = one(&msg(json!({"file": "photos/x.jpg"})), 0, None).unwrap();
        assert!(saved.media_saved);
        let skipped = one(
            &msg(json!({"file": "(File exceeds maximum size. Change data exporting settings to download.)"})),
            0,
            None,
        )
        .unwrap();
        assert!(!skipped.media_saved);
    }

    #[test]
    fn every_apology_desktop_writes_is_unsaved_media_and_never_a_path() {
        // Media off, and a fetch that failed. Both used to count as saved and
        // land in `file`, where the digest joined them onto the topic folder.
        for apology in [
            "(File not included. Change data exporting settings to download.)",
            "(File unavailable, please try again later)",
        ] {
            for key in ["file", "photo"] {
                let out = one(&msg(json!({ key: apology })), 0, None).unwrap();
                assert!(!out.media_saved, "{key}: {apology}");
                assert_eq!(out.file, "", "{key}: {apology}");
            }
        }
        // A real file whose name merely starts with a parenthesis is not one.
        let odd = one(&msg(json!({"file": "files/(File) notes.txt"})), 0, None).unwrap();
        assert!(odd.media_saved);
    }

    #[test]
    fn a_service_entry_takes_its_sender_from_the_actor_fields() {
        let raw = msg(json!({
            "type": "service", "action": "invite_members",
            "actor": "Someone Else", "actor_id": "user2",
            "from": "wrong", "from_id": "user1"
        }));
        let out = one(&raw, 0, None).unwrap();
        assert!(out.service);
        assert_eq!(out.sender, "user2");
        assert_eq!(out.name, "Someone Else");
    }

    #[test]
    fn a_poll_keeps_its_question_and_every_tally() {
        // The shape the exporter writes and the reference's seven carry.
        let raw = msg(json!({"poll": {
            "question": "Kada?", "closed": true, "total_voters": 9,
            "answers": [
                {"text": "petak", "voters": 6, "chosen": true},
                {"text": "subota", "voters": 3, "chosen": false},
                "not an answer"
            ]
        }}));
        let poll = one(&raw, 0, None).unwrap().poll.expect("a poll");
        assert_eq!(poll.question, "Kada?");
        assert!(poll.closed);
        assert_eq!(poll.total_voters, 9);
        assert_eq!(poll.answers.len(), 2, "a malformed answer is dropped");
        assert_eq!(poll.answers[0].text, "petak");
        assert_eq!(poll.answers[1].voters, 3);
        assert!(one(&msg(json!({})), 0, None).unwrap().poll.is_none());
    }

    #[test]
    fn a_title_and_a_pin_are_read_off_service_messages_only() {
        let renamed = msg(json!({"type": "service", "action": "topic_edit",
                                 "new_title": "foto video", "new_icon_emoji_id": 0}));
        assert_eq!(one(&renamed, 0, None).unwrap().title, "foto video");
        let pin = msg(json!({"type": "service", "action": "pin_message", "message_id": 41}));
        assert_eq!(one(&pin, 0, None).unwrap().pinned, Some(41));
        // An audio file carries a `title` of its own, which is a song's.
        let song = one(
            &msg(json!({"title": "a song", "duration_seconds": 214})),
            0,
            None,
        )
        .unwrap();
        assert_eq!(song.title, "");
        assert_eq!(song.duration, 214);
        assert_eq!(song.pinned, None);
    }

    #[test]
    fn a_falsy_number_reads_as_empty_the_way_python_does() {
        // `str(x or "")` — 0 is falsy, so a zeroed id is no id, not "0".
        assert_eq!(as_text(Some(&json!(0))), "");
        assert_eq!(as_text(Some(&json!(12))), "12");
        assert_eq!(as_text(None), "");
    }

    #[test]
    fn an_absent_complete_flag_differs_from_a_false_one() {
        // A roster the export never wrote and one it knows is short must not
        // look the same downstream.
        assert_eq!(roster(Path::new("no-such-folder")).1, None);
        assert_eq!(flag(None), None);
        assert_eq!(flag(Some(&Value::Null)), None);
        assert_eq!(flag(Some(&json!(false))), Some(false));
        assert_eq!(flag(Some(&json!(0))), Some(false));
        assert_eq!(flag(Some(&json!(true))), Some(true));
        assert_eq!(flag(Some(&json!(1))), Some(true));
    }
}
