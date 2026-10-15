//! **Is anything in an old export missing from the database?**
//!
//! An export folder is a photograph: the messages a chat held on the day it was
//! taken. A database is what this app has read since. A message in the first
//! and not the second was deleted on Telegram before the database ever saw it,
//! and the old export is the only copy left — which is exactly what someone
//! holding a drive of old exports wants to know before throwing any away.
//!
//! Offline. Every `result.json` under the folders given is read for its chat id
//! and its message ids; every database in the database folder for the same.
//! **Ids and dates only, never text**: the report is printed to a terminal and
//! may be written to a file, and both are the sort of thing that gets pasted.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// One message as a comparison needs it.
#[derive(Debug, Clone, Deserialize)]
pub struct Seen {
    pub id: i64,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub date: String,
}

/// One chat as a `result.json` records it. Every other key is skipped by serde
/// without being kept, which is what lets a 500 MB file pass through.
///
/// Generic over what a message is read as: [`Seen`] for a comparison,
/// [`Kept`] for an import, which wants a few whole messages out of thousands.
#[derive(Debug, Deserialize)]
struct ChatJson<M> {
    id: Option<i64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default = "Vec::new")]
    messages: Vec<M>,
}

#[derive(Debug, Deserialize)]
struct ListJson<M> {
    #[serde(default = "Vec::new")]
    list: Vec<ChatJson<M>>,
}

/// A `result.json` is one of two shapes: a single chat (Desktop's "export chat
/// history", and every folder this app writes), or a whole account, where the
/// chats are under `chats.list` and `left_chats.list`.
///
/// The single chat's fields are spelled out rather than `#[serde(flatten)]`ed
/// from [`ChatJson`]: flatten buffers the whole object before handing it on,
/// and the whole object is the 500 MB of messages.
#[derive(Debug, Deserialize)]
struct FileJson<M> {
    id: Option<i64>,
    #[serde(default)]
    name: Option<String>,
    /// Written by this app into a forum topic's folder; Desktop never splits.
    #[serde(default)]
    topic_id: Option<i64>,
    #[serde(default = "Vec::new")]
    messages: Vec<M>,
    #[serde(default = "Option::default")]
    chats: Option<ListJson<M>>,
    #[serde(default = "Option::default")]
    left_chats: Option<ListJson<M>>,
}

/// One chat out of one file: its id, name, topic and messages.
type Parsed<M> = (i64, Option<String>, Option<i64>, Vec<M>);

/// Read one `result.json` of either shape into its chats.
fn parse<M: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<Parsed<M>>, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let file: FileJson<M> = serde_json::from_reader(std::io::BufReader::with_capacity(1 << 20, f))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(id) = file.id {
        out.push((id, file.name, file.topic_id, file.messages));
    }
    let lists = file.chats.into_iter().chain(file.left_chats);
    for chat in lists.flat_map(|l| l.list) {
        if let Some(id) = chat.id {
            out.push((id, chat.name, None, chat.messages));
        }
    }
    Ok(out)
}

thread_local! {
    /// The ids [`Kept`] keeps, for the length of one [`payloads`] call. A
    /// `Deserialize` impl takes no arguments, and the alternative to passing
    /// the set this way is holding every message of a 500 MB file at once.
    static WANTED: std::cell::RefCell<std::collections::HashSet<i64>> = Default::default();
}

/// A whole message if its id is wanted, nothing otherwise. Each message is
/// parsed whole and then kept or dropped, so memory is one message plus the
/// ones asked for.
struct Kept(Option<serde_json::Map<String, serde_json::Value>>);

impl<'de> Deserialize<'de> for Kept {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let m = serde_json::Map::deserialize(d)?;
        let id = m.get("id").and_then(serde_json::Value::as_i64);
        let wanted = id.is_some_and(|id| WANTED.with(|w| w.borrow().contains(&id)));
        Ok(Kept(wanted.then_some(m)))
    }
}

/// A message as an old export wrote it, and the topic its folder was for.
pub struct Payload {
    pub topic_id: Option<i64>,
    pub message: serde_json::Map<String, serde_json::Value>,
}

/// The whole messages for `wanted` (chat id → ids), read back out of the
/// files that hold them. The first file to have a message wins, as in
/// [`read_exports`].
pub fn payloads(
    files: &[PathBuf],
    wanted: &HashMap<i64, std::collections::HashSet<i64>>,
) -> (HashMap<(i64, i64), Payload>, Vec<Skipped>) {
    let mut out = HashMap::new();
    let mut skipped = Vec::new();
    WANTED.with(|w| *w.borrow_mut() = wanted.values().flatten().copied().collect());
    for path in files {
        match parse::<Kept>(path) {
            Ok(chats) => {
                for (chat_id, _, topic_id, messages) in chats {
                    let Some(ids) = wanted.get(&chat_id) else {
                        continue;
                    };
                    for m in messages.into_iter().filter_map(|k| k.0) {
                        let Some(id) = m.get("id").and_then(serde_json::Value::as_i64) else {
                            continue;
                        };
                        if ids.contains(&id) {
                            out.entry((chat_id, id)).or_insert(Payload {
                                topic_id,
                                message: m,
                            });
                        }
                    }
                }
            }
            Err(e) => skipped.push((path.clone(), e)),
        }
    }
    WANTED.with(|w| w.borrow_mut().clear());
    (out, skipped)
}

/// Everything the exports say about one chat, merged across every file that
/// holds it — topic folders, and older and newer exports of the same chat.
#[derive(Debug, Default)]
pub struct Exported {
    pub title: String,
    pub files: BTreeSet<PathBuf>,
    pub messages: BTreeMap<i64, Seen>,
}

/// A file that could not be used, and why.
pub type Skipped = (PathBuf, String);

/// Find every `result.json` under `roots`, no more than `depth` folders down.
///
/// Bounded because the media folders under an export hold thousands of files
/// and none of them is ever a `result.json`; four levels covers a root of
/// exports, one export, its topic folders and one spare.
pub fn find_results(roots: &[PathBuf], depth: usize) -> Vec<PathBuf> {
    find_named(roots, depth, "result.json")
}

/// Every folder under `roots` that is an **HTML-only** export: a
/// `messages.html` with no `result.json` beside it. Desktop writes one when
/// its JSON box is left unticked, and a phone backup often has nothing else.
pub fn find_html(roots: &[PathBuf], depth: usize) -> Vec<PathBuf> {
    find_named(roots, depth, "messages.html")
        .into_iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .filter(|d| !d.join("result.json").exists())
        .collect()
}

fn find_named(roots: &[PathBuf], depth: usize, name: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, name: &str, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                if depth > 0 {
                    walk(&path, depth - 1, name, out);
                }
            } else if path.file_name().is_some_and(|n| n == name) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for root in roots {
        if root.is_file() {
            if root.file_name().is_some_and(|n| n == name) {
                out.push(root.clone());
            }
        } else {
            walk(root, depth, name, &mut out);
        }
    }
    out.sort();
    out
}

/// Read an HTML-only export folder: its chat's name and every message id,
/// with the date each was sent.
///
/// The pages carry **no chat id**, so the result is matched to a database by
/// name — see [`match_titles`]. Read by plain scanning for the three marks
/// Desktop's writer (and `tgx-html`, which reproduces it) always puts down:
///
/// - `id="message123"` on a message; `id="message-4"` on a day divider, whose
///   text is the day a service message under it was sent — a service message
///   has no time of its own in the markup;
/// - `class="pull_right date details" title="18.12.2025 16:17:52 UTC+01:00"`,
///   the full date of an ordinary message. *Not* the `date details` span in a
///   forwarded block, which is the original's date.
pub fn read_html(folder: &Path) -> Result<Exported, String> {
    let mut pages: Vec<(u32, PathBuf)> = std::fs::read_dir(folder)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix("messages")?.strip_suffix(".html")?;
            let n = if n.is_empty() { 1 } else { n.parse().ok()? };
            Some((n, e.path()))
        })
        .collect();
    pages.sort();
    let mut out = Exported::default();
    for (_, page) in &pages {
        let text = std::fs::read_to_string(page).map_err(|e| e.to_string())?;
        if out.title.is_empty() {
            out.title = header_title(&text).unwrap_or_default();
        }
        scan_page(&text, &mut out.messages);
    }
    if out.messages.is_empty() {
        return Err("no messages in its pages — not a Telegram HTML export".into());
    }
    out.files.insert(folder.join("messages.html"));
    Ok(out)
}

fn header_title(page: &str) -> Option<String> {
    let at = page.find(r#"class="text bold""#)?;
    let rest = &page[at..];
    let body = &rest[rest.find('>')? + 1..];
    let body = &body[..body.find("</")?];
    let title = unescape(body.trim());
    // A topic page's header is a back link: "‹ Chat".
    Some(title.trim_start_matches('‹').trim().to_string())
}

fn scan_page(page: &str, into: &mut BTreeMap<i64, Seen>) {
    const MARK: &str = "id=\"message";
    const DATE: &str = "class=\"pull_right date details\" title=\"";
    let starts: Vec<usize> = page.match_indices(MARK).map(|(i, _)| i).collect();
    let mut day = String::new();
    for (n, &at) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(page.len());
        let block = &page[at..end];
        let tag_start = page[..at].rfind('<').unwrap_or(at);
        let tag = &page[tag_start..at];
        let id_text = &block[MARK.len()..];
        let id_text = &id_text[..id_text.find('"').unwrap_or(0)];
        if id_text.starts_with('-') {
            if let Some(d) = between(block, "class=\"body details\">", "</div>") {
                day = day_of(d.trim()).unwrap_or_default();
            }
            continue;
        }
        let Ok(id) = id_text.parse::<i64>() else {
            continue;
        };
        let service = tag.contains("service");
        let date = if service {
            if day.is_empty() {
                String::new()
            } else {
                format!("{day}T00:00:00")
            }
        } else {
            between(block, DATE, "\"")
                .and_then(desktop_date)
                .unwrap_or_default()
        };
        into.entry(id).or_insert(Seen {
            id,
            kind: if service { "service" } else { "message" }.into(),
            date,
        });
    }
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let from = s.find(open)? + open.len();
    let to = s[from..].find(close)? + from;
    Some(&s[from..to])
}

/// `18.12.2025 16:17:52 UTC+01:00` → `2025-12-18T16:17:52`, the form
/// `result.json` writes, so the two kinds of export report alike.
fn desktop_date(title: &str) -> Option<String> {
    let (date, rest) = title.split_once(' ')?;
    let time = rest.split(' ').next()?;
    let mut p = date.split('.');
    let (d, m, y) = (p.next()?, p.next()?, p.next()?);
    Some(format!("{y}-{m}-{d}T{time}"))
}

/// `18 December 2025` → `2025-12-18`.
fn day_of(text: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let mut p = text.split_whitespace();
    let d: u32 = p.next()?.parse().ok()?;
    let month = p.next()?;
    let m = MONTHS.iter().position(|x| *x == month)? + 1;
    let y: u32 = p.next()?.parse().ok()?;
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Give each HTML export the id of the database whose chat has the same name,
/// case and surrounding space aside. What matches nobody is returned as is.
pub async fn match_titles(
    html: Vec<Exported>,
    db_dir: &Path,
) -> std::io::Result<(BTreeMap<i64, Exported>, Vec<Exported>)> {
    let archived = crate::engine::archived_chats(db_dir).await?;
    let key = |t: &str| t.trim().to_lowercase();
    let ids: HashMap<String, i64> = archived
        .chats
        .iter()
        .map(|(id, title)| (key(title), *id))
        .collect();
    let mut matched: BTreeMap<i64, Exported> = BTreeMap::new();
    let mut unmatched = Vec::new();
    for e in html {
        match ids.get(&key(&e.title)) {
            Some(id) => {
                let into = matched.entry(*id).or_default();
                if into.title.is_empty() {
                    into.title = e.title.clone();
                }
                into.files.extend(e.files);
                for (k, v) in e.messages {
                    into.messages.entry(k).or_insert(v);
                }
            }
            None => unmatched.push(e),
        }
    }
    Ok((matched, unmatched))
}

/// Read every file and merge by chat id.
pub fn read_exports(files: &[PathBuf]) -> (BTreeMap<i64, Exported>, Vec<Skipped>) {
    let mut chats: BTreeMap<i64, Exported> = BTreeMap::new();
    let mut skipped = Vec::new();
    for path in files {
        let parsed = match parse::<Seen>(path) {
            Ok(p) => p,
            Err(e) => {
                skipped.push((path.clone(), e));
                continue;
            }
        };
        let found = parsed.len();
        for (id, name, _, messages) in parsed {
            let entry = chats.entry(id).or_default();
            if entry.title.is_empty() {
                entry.title = name.unwrap_or_default();
            }
            entry.files.insert(path.clone());
            for m in messages {
                entry.messages.entry(m.id).or_insert(m);
            }
        }
        if found == 0 {
            skipped.push((
                path.clone(),
                "no chat id in it — not a Desktop-format export".into(),
            ));
        }
    }
    (chats, skipped)
}

/// One chat's answer.
#[derive(Debug)]
pub struct Verdict {
    pub chat_id: i64,
    pub title: String,
    pub export_files: Vec<PathBuf>,
    pub exported: usize,
    pub database: Option<PathBuf>,
    /// When the database last finished a sync. An import trusts "missing" to
    /// mean "deleted on Telegram" only once one has.
    pub synced: Option<i64>,
    pub stored: usize,
    pub stored_deleted: usize,
    /// In an export, not in the database, and no newer than the newest
    /// message the database holds: the messages only an old export still has.
    pub missing: Vec<Seen>,
    /// In an export and past the newest message the database holds: sent
    /// after its last sync, so never seen rather than deleted. Kept apart
    /// because an import marks what is `missing` deleted, and a Desktop
    /// export taken after the last sync would otherwise stamp Tuesday's live
    /// messages deleted on a database last synced on Monday. A database
    /// holding nothing for the chat has no newest message, so everything the
    /// export holds lands here.
    pub newer: Vec<Seen>,
    /// In the database and inside the export's id range, but not in any
    /// export — sent and deleted between two exports, or skipped by one.
    pub only_in_database: usize,
}

/// Compare every exported chat against the database folder.
pub async fn compare(
    exports: BTreeMap<i64, Exported>,
    db_dir: &Path,
) -> std::io::Result<(Vec<Verdict>, Vec<Skipped>)> {
    let mut archived = crate::engine::archived_chats(db_dir).await?;
    let mut verdicts = Vec::new();
    let titles: HashMap<i64, String> = archived.chats.iter().cloned().collect();
    for (chat_id, exported) in exports {
        // The database's title first: a forum split into topic folders names
        // each `result.json` after its topic, so the export alone would call
        // half the chats "General". Failing that, the folder above a topic
        // folder, which is the chat's.
        let title = titles.get(&chat_id).cloned().unwrap_or_else(|| {
            let first = exported.files.iter().next();
            let topic_split = first
                .and_then(|f| f.parent())
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .is_some_and(|n| {
                    n.split_once(" - ")
                        .is_some_and(|(id, _)| id.parse::<u64>().is_ok())
                });
            match first.and_then(|f| f.parent()).and_then(|d| d.parent()) {
                Some(chat_dir) if topic_split => chat_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                _ => exported.title.clone(),
            }
        });
        let mut v = Verdict {
            chat_id,
            title,
            export_files: exported.files.iter().cloned().collect(),
            exported: exported.messages.len(),
            database: archived.files.get(&chat_id).cloned(),
            synced: archived.synced.get(&chat_id).copied(),
            stored: 0,
            stored_deleted: 0,
            missing: Vec::new(),
            newer: Vec::new(),
            only_in_database: 0,
        };
        // A database that will not open is reported with the other unreadable
        // files, not quietly treated as absent — "no database" would send the
        // reader looking for a file that is right there.
        let stored: HashMap<i64, bool> = match v.database.clone() {
            Some(path) => match open_ids(&path, chat_id).await {
                Ok(ids) => ids.into_iter().collect(),
                Err(e) => {
                    v.database = None;
                    archived.unreadable.push((path, e.to_string()));
                    HashMap::new()
                }
            },
            None => HashMap::new(),
        };
        v.stored = stored.len();
        v.stored_deleted = stored.values().filter(|d| **d).count();
        if v.database.is_some() {
            // Bounded by id, not by date. A message id only grows within a
            // chat, so one under the newest stored id was sent before a
            // message the database has read, and the walk that read it went
            // past; an export's dates are a local wall clock and `synced` is
            // not, so the two cannot be compared to the hour.
            let newest = stored.keys().max().copied();
            (v.newer, v.missing) = exported
                .messages
                .values()
                .filter(|m| !stored.contains_key(&m.id))
                .cloned()
                .partition(|m| newest.is_none_or(|n| m.id > n));
            if let (Some(lo), Some(hi)) = (
                exported.messages.keys().next(),
                exported.messages.keys().next_back(),
            ) {
                v.only_in_database = stored
                    .keys()
                    .filter(|id| (*lo..=*hi).contains(*id) && !exported.messages.contains_key(id))
                    .count();
            }
        }
        verdicts.push(v);
    }
    Ok((verdicts, archived.unreadable))
}

async fn open_ids(path: &Path, chat_id: i64) -> Result<Vec<(i64, bool)>, tgx_archive::Error> {
    tgx_archive::Store::open(path)
        .await?
        .message_ids(chat_id)
        .await
}

/// The first and last date of a set of messages, as Desktop writes them.
pub fn date_span(messages: &[Seen]) -> Option<(&str, &str)> {
    let mut dates = messages
        .iter()
        .map(|m| m.date.as_str())
        .filter(|d| !d.is_empty());
    let first = dates.next()?;
    let (lo, hi) = dates.fold((first, first), |(lo, hi), d| (lo.min(d), hi.max(d)));
    Some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tgx-compare-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_single_chat_and_a_whole_account_export_are_both_read() {
        let dir = tmp("shapes");
        std::fs::create_dir_all(dir.join("one")).unwrap();
        std::fs::write(
            dir.join("one").join("result.json"),
            r#"{"name":"Dev","type":"private_group","id":7,"messages":[
                {"id":1,"type":"message","date":"2025-01-01T00:00:00","text":"x"},
                {"id":2,"type":"service","date":"2025-01-02T00:00:00","action":"pin_message"}]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("all")).unwrap();
        std::fs::write(
            dir.join("all").join("result.json"),
            r#"{"about":"x","chats":{"about":"y","list":[
                {"name":"Dev","type":"private_group","id":7,"messages":[{"id":3,"type":"message","date":"d"}]},
                {"name":"Mum","type":"personal_chat","id":8,"messages":[]}]},
               "left_chats":{"list":[{"name":"Old","type":"public_supergroup","id":9,"messages":[]}]}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("notes.json"), "{}").unwrap();
        let files = find_results(std::slice::from_ref(&dir), 4);
        assert_eq!(files.len(), 2);
        let (chats, skipped) = read_exports(&files);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(chats.keys().copied().collect::<Vec<_>>(), vec![7, 8, 9]);
        // The two exports of chat 7 are merged, not one chosen.
        assert_eq!(chats[&7].messages.len(), 3);
        assert_eq!(chats[&7].files.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_html_export_gives_up_its_ids_and_dates_across_pages() {
        let dir = tmp("html");
        let export = dir.join("Dev & Co");
        // Our own writer, which the html leg proves writes Desktop's markup.
        let mut w = tgx_html::HtmlWriter::new(&export, "Dev & Co", 2);
        let msgs = [
            json!({ "id": 5, "type": "message", "date": "2025-12-18T16:17:52",
                    "from": "A", "from_id": "user1", "text": "x" }),
            json!({ "id": 6, "type": "service", "date": "2025-12-19T09:00:00",
                    "actor": "A", "actor_id": "user1", "action": "pin_message",
                    "message_id": 5, "text": "" }),
            json!({ "id": 9, "type": "message", "date": "2025-12-19T10:11:12",
                    "from": "B", "from_id": "user2", "text": "y" }),
        ];
        for m in msgs {
            w.add(m.as_object().unwrap()).unwrap();
        }
        w.close().unwrap();
        assert!(
            export.join("messages2.html").is_file(),
            "meant to span pages"
        );

        assert_eq!(
            find_html(std::slice::from_ref(&dir), 2),
            vec![export.clone()]
        );
        let e = read_html(&export).unwrap();
        assert_eq!(e.title, "Dev & Co");
        let got: Vec<(i64, &str, &str)> = e
            .messages
            .values()
            .map(|m| (m.id, m.kind.as_str(), m.date.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (5, "message", "2025-12-18T16:17:52"),
                // A service message's time is not in the markup; its day is.
                (6, "service", "2025-12-19T00:00:00"),
                (9, "message", "2025-12-19T10:11:12"),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_message_only_the_export_has_is_reported_missing() {
        let dir = tmp("missing");
        let db = dir.join("db");
        let store = tgx_archive::Store::open(&db.join("Dev").join("Dev.sqlite"))
            .await
            .unwrap();
        store
            .upsert_chat(7, "Dev", "private_group", 1)
            .await
            .unwrap();
        for id in [2, 3, 5] {
            let msg = serde_json::json!({"id": id, "type": "message", "date": "d", "text": ""});
            store
                .merge(7, id, None, msg.as_object().unwrap(), 1)
                .await
                .unwrap();
        }
        drop(store);
        let export = dir.join("export");
        std::fs::create_dir_all(&export).unwrap();
        std::fs::write(
            export.join("result.json"),
            r#"{"name":"Dev","type":"private_group","id":7,"messages":[
                {"id":1,"type":"message","date":"2024-05-01T00:00:00"},
                {"id":2,"type":"message","date":"2024-05-02T00:00:00"},
                {"id":4,"type":"service","date":"2024-05-03T00:00:00"}]}"#,
        )
        .unwrap();
        // And a chat with no database at all.
        std::fs::create_dir_all(export.join("other")).unwrap();
        std::fs::write(
            export.join("other").join("result.json"),
            r#"{"name":"Mum","type":"personal_chat","id":8,"messages":[{"id":1,"type":"message","date":"d"}]}"#,
        )
        .unwrap();

        let (chats, _) = read_exports(&find_results(&[export], 4));
        let (verdicts, _) = compare(chats, &db).await.unwrap();
        let dev = verdicts.iter().find(|v| v.chat_id == 7).unwrap();
        let missing: Vec<i64> = dev.missing.iter().map(|m| m.id).collect();
        assert_eq!(missing, vec![1, 4]);
        assert!(dev.newer.is_empty());
        assert_eq!(dev.stored, 3);
        // 3 is inside the export's range and not in it; 5 is newer than it.
        assert_eq!(dev.only_in_database, 1);
        let mum = verdicts.iter().find(|v| v.chat_id == 8).unwrap();
        assert!(mum.database.is_none());
        assert!(
            mum.missing.is_empty(),
            "no database is not the same as all missing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
