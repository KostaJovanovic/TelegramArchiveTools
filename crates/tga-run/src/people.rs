//! People: everybody in a folder's chats, once each, kept in `people.json`
//! beside them and updated by every count.
//!
//! **Kept per chat, merged on the way out.** The file holds, for each chat by
//! its id, who is on its member list and who wrote how much. A count of some
//! chats replaces those chats' entries and leaves the rest, so the list grows
//! with every chat ever counted, and a chat counted twice — from Telegram and
//! from its export — is one entry, never counted double.
//!
//! **One person once, by peer key (`user123`), never by name.** Somebody in
//! five chats is one row, their messages added up; a namesake is another row.
//! The name shown is the one from the chat counted most recently.
//!
//! An export already read is not read again while its file is unchanged (size,
//! time and the Handles only setting), so Update list costs only what moved.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::job::{in_parallel, workers, Wake, Waking};
use crate::refresh::{Source, Target};

/// The saved list, in the folder it describes.
pub const FILE: &str = "people.json";

/// One account, across every chat in the list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Person {
    pub key: String,
    pub name: String,
    pub messages: usize,
    /// How many chats they are in: on the member list or wrote in.
    pub chats: usize,
    /// When the chat their name came from was counted. Whose name wins.
    pub newest: i64,
    /// The organisation a tag in their name names (its label), or empty.
    pub org: String,
}

/// What the People screen shows.
#[derive(Debug, Clone, Default)]
pub struct List {
    pub people: Vec<Person>,
    pub orgs: Vec<OrgCount>,
    /// Any chat has message counts. Without, people are measured in chats.
    pub messages: bool,
    /// Every chat in the saved list, by title.
    pub chats: Vec<ChatRow>,
    /// Said under the head: what was not read, and why.
    pub notes: Vec<String>,
}

/// One saved chat, as the Chats tab lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRow {
    /// The key in `people.json`: the chat's id, or `export:<path>` for an
    /// export that names none.
    pub id: String,
    pub title: String,
    pub people: usize,
    /// Messages counted, `None` when only the member list was read.
    pub messages: Option<usize>,
    pub updated: i64,
    /// Read from an export rather than from Telegram.
    pub from_export: bool,
    /// The export it was read from is no longer on disk.
    pub file_gone: bool,
}

impl ChatRow {
    /// The Telegram chat id, where the key is one.
    pub fn chat_id(&self) -> Option<i64> {
        self.id.parse().ok()
    }
}

/// Take one chat out of the saved list in `dir`, and list what is left.
///
/// An export still in the folder is read again by the next update; this is
/// for a chat that is gone, or one counted by mistake.
pub fn remove(dir: &Path, id: &str) -> Result<List> {
    let mut book = Book::load(dir)?;
    if book.chats.remove(id).is_some() {
        book.save(dir)?;
    }
    Ok(book.list(dir, Vec::new()))
}

/// One organisation and who in the list belongs to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgCount {
    pub label: String,
    pub name: String,
    pub people: usize,
    pub messages: usize,
}

/// The file: every chat counted, by chat id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Book {
    pub chats: BTreeMap<String, Chat>,
}

/// One chat's people.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chat {
    pub title: String,
    /// Unix seconds this entry was last written.
    pub updated: i64,
    /// Peer key → name, from the member list. Empty when none was read.
    pub members: BTreeMap<String, String>,
    /// Peer key → (name, messages), from every message. `None` when the
    /// messages were never counted.
    pub messages: Option<BTreeMap<String, (String, usize)>>,
    /// The export file it was last read from, so an unchanged one is skipped.
    pub export: Option<Seen>,
}

/// An export as it was when read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: u64,
    pub handles: bool,
}

impl Seen {
    fn of(target: &Target, handles: bool) -> Self {
        Seen {
            path: target.path().to_path_buf(),
            bytes: target.bytes,
            modified: target.changed.map_or(0, secs),
            handles,
        }
    }
}

fn secs(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn now() -> i64 {
    secs(SystemTime::now()) as i64
}

impl Book {
    /// The list in `dir`; empty when there is none yet. **A file that cannot
    /// be read is an error, not an empty list**, so it is never overwritten
    /// with one.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(FILE);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("{} could not be read", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| path.display().to_string()),
        }
    }

    /// Written beside, then renamed over, so a save cut short leaves the last
    /// whole list.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join(FILE);
        let part = dir.join(format!("{FILE}.part"));
        std::fs::write(&part, serde_json::to_vec(self)?)
            .and_then(|()| std::fs::rename(&part, &path))
            .with_context(|| path.display().to_string())
    }

    fn entry(&mut self, id: &str, title: &str) -> &mut Chat {
        let chat = self.chats.entry(id.to_string()).or_default();
        if !title.is_empty() {
            chat.title = title.to_string();
        }
        chat.updated = now();
        chat
    }

    /// A chat's whole member list, from Telegram.
    pub fn put_members(&mut self, id: i64, title: &str, members: BTreeMap<String, String>) {
        self.entry(&id.to_string(), title).members = members;
    }

    /// A chat's every message, counted by sender, from Telegram. Newer than
    /// any export of it, so the export is forgotten and read again next time.
    pub fn put_messages(
        &mut self,
        id: i64,
        title: &str,
        messages: BTreeMap<String, (String, usize)>,
    ) {
        let chat = self.entry(&id.to_string(), title);
        chat.messages = Some(messages);
        chat.export = None;
    }

    /// Everybody, each once, most messages (or chats) first.
    pub fn people(&self) -> (Vec<Person>, bool) {
        let mut all: HashMap<&str, Person> = HashMap::new();
        let messages = self.chats.values().any(|c| c.messages.is_some());
        for chat in self.chats.values() {
            let counted = chat.messages.iter().flatten();
            let named = chat.members.iter().map(|(k, n)| (k, (n, 0)));
            let mut here: BTreeMap<&String, (&String, usize)> = named.collect();
            for (key, (name, n)) in counted {
                here.insert(key, (name, *n));
            }
            for (key, (name, n)) in here {
                let person = all.entry(key).or_insert_with(|| Person {
                    key: key.clone(),
                    newest: i64::MIN,
                    ..Person::default()
                });
                person.messages += n;
                person.chats += 1;
                if !name.is_empty() && chat.updated >= person.newest {
                    person.newest = chat.updated;
                    person.name = name.clone();
                }
            }
        }
        let mut people: Vec<Person> = all.into_values().collect();
        people.sort_by(|a, b| {
            (b.messages, b.chats)
                .cmp(&(a.messages, a.chats))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        (people, messages)
    }

    /// The screen's list, with the organisations in `dir` matched in.
    pub fn list(&self, dir: &Path, mut notes: Vec<String>) -> List {
        let (mut people, messages) = self.people();
        let orgs = match load_orgs(dir) {
            Ok(orgs) => assign(&mut people, &orgs),
            Err(e) => {
                notes.push(format!("{e:#}"));
                Vec::new()
            }
        };
        let mut chats: Vec<ChatRow> = self
            .chats
            .iter()
            .map(|(id, chat)| {
                let counted = chat.messages.as_ref();
                let mut keys: std::collections::BTreeSet<&String> = chat.members.keys().collect();
                keys.extend(counted.into_iter().flat_map(|m| m.keys()));
                ChatRow {
                    id: id.clone(),
                    title: chat.title.clone(),
                    people: keys.len(),
                    messages: counted.map(|m| m.values().map(|(_, n)| n).sum()),
                    updated: chat.updated,
                    from_export: chat.export.is_some(),
                    file_gone: chat.export.as_ref().is_some_and(|e| !e.path.exists()),
                }
            })
            .collect();
        chats.sort_by_key(|c| c.title.to_lowercase());
        List {
            people,
            orgs,
            messages,
            chats,
            notes,
        }
    }
}

/// What the export counter says.
#[derive(Debug, Clone)]
pub enum Tally {
    /// This many of `of` exports have been looked at.
    Read {
        done: usize,
        of: usize,
    },
    Done(List),
}

/// Bring the list in `dir` up to date with `targets`, off the paint thread:
/// read every export that changed since it was last read, save, list.
///
/// `handles` is Settings › Analysis › Handles only, so a name here is the one
/// the reports show.
pub fn count(dir: PathBuf, targets: Vec<Target>, handles: bool, wake: Wake) -> Receiver<Tally> {
    let (tx, rx) = channel();
    let tx = Waking { tx, wake };
    std::thread::Builder::new()
        .name("tga-people".into())
        .spawn(move || {
            let of = targets.len();
            let done = AtomicUsize::new(0);
            let list = update(&dir, &targets, handles, &|| {
                let done = done.fetch_add(1, Ordering::Relaxed) + 1;
                tx.send(Tally::Read { done, of });
            });
            tx.send(Tally::Done(list));
        })
        .expect("the OS refused a worker thread");
    rx
}

/// [`count`]'s work. `each` is called after each export, read or skipped.
pub fn update(dir: &Path, targets: &[Target], handles: bool, each: &(dyn Fn() + Sync)) -> List {
    let mut book = match Book::load(dir) {
        Ok(book) => book,
        Err(e) => {
            return List {
                notes: vec![format!("{e:#}; nothing was changed in it")],
                ..List::default()
            }
        }
    };
    let unchanged: Vec<Seen> = book
        .chats
        .values()
        .filter_map(|c| c.export.clone())
        .collect();
    let read: Mutex<Vec<(String, Chat)>> = Mutex::default();
    let failed: Mutex<Vec<String>> = Mutex::default();
    let never = AtomicBool::new(false);
    in_parallel(targets, workers(targets.len()), &never, |_, target| {
        let seen = Seen::of(target, handles);
        if !unchanged.contains(&seen) {
            match read_export(target, seen) {
                Ok(chat) => read.lock().expect("read").push(chat),
                Err(e) => failed
                    .lock()
                    .expect("failed")
                    .push(format!("Not read: {}: {e:#}", target.name())),
            }
        }
        each();
    });
    let read = read.into_inner().expect("read");
    let mut notes = failed.into_inner().expect("failed");
    if !read.is_empty() {
        for (id, chat) in read {
            // The member list is Telegram's alone; an export does not replace it.
            let members = book
                .chats
                .remove(&id)
                .map(|c| c.members)
                .unwrap_or_default();
            book.chats.insert(id, Chat { members, ..chat });
        }
        if let Err(e) = book.save(dir) {
            notes.push(format!("The list could not be saved: {e:#}"));
        }
    }
    book.list(dir, notes)
}

/// One export's senders, keyed by its chat's id where it has one.
fn read_export(target: &Target, seen: Seen) -> Result<(String, Chat)> {
    let mut export = match &target.source {
        Source::Database(db) => tga_db::load(db, None, None)?,
        Source::Folder(dir) => tga_read::load(dir, None)?,
    };
    if seen.handles {
        export.handles_only();
    }
    let id = export.topics.iter().find_map(|t| t.chat_id).map_or_else(
        || format!("export:{}", seen.path.display()),
        |id| id.to_string(),
    );
    // The name on each sender's newest message: a rename shows as who they are now.
    let mut by: BTreeMap<String, (String, usize, i64)> = BTreeMap::new();
    for m in export.said().filter(|m| !m.sender.is_empty()) {
        let row = by
            .entry(m.sender.clone())
            .or_insert((String::new(), 0, i64::MIN));
        row.1 += 1;
        if m.unix >= row.2 && !m.name.is_empty() {
            row.0 = m.name.clone();
            row.2 = m.unix;
        }
    }
    let title = if export.name.is_empty() {
        target.name()
    } else {
        export.name.clone()
    };
    let chat = Chat {
        title,
        updated: now(),
        members: BTreeMap::new(),
        messages: Some(by.into_iter().map(|(k, (n, c, _))| (k, (n, c))).collect()),
        export: Some(seen),
    };
    Ok((id, chat))
}

// --- organisations --------------------------------------------------------

/// The hand-made list of organisations, beside `people.json`: ReVision's
/// `organisations.csv`, same columns, so its file works here as it is.
pub const ORGS: &str = "organisations.csv";

/// One organisation, ready to match.
#[derive(Debug, Clone)]
pub struct Org {
    pub name: String,
    /// The first short name, else the name; with the city when two share it.
    pub label: String,
    /// Every form that names it, lowercased: short names of 2+ characters,
    /// the name and other names of 3+.
    forms: Vec<String>,
}

/// The organisations in `dir`; none when there is no file.
pub fn load_orgs(dir: &Path) -> Result<Vec<Org>> {
    let path = dir.join(ORGS);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| path.display().to_string()),
    };
    Ok(parse_orgs(text.trim_start_matches('\u{feff}')))
}

fn parse_orgs(text: &str) -> Vec<Org> {
    let mut rows = csv_rows(text).into_iter();
    let Some(head) = rows.next() else {
        return Vec::new();
    };
    let col = |name: &str| head.iter().position(|h| h.trim() == name);
    let (id, name, city, short, other) = (
        col("id"),
        col("name"),
        col("city"),
        col("short"),
        col("other_names"),
    );
    let get = |row: &[String], at: Option<usize>| {
        at.and_then(|i| row.get(i))
            .map_or("", |s| s.trim())
            .to_string()
    };
    let split = |s: String| -> Vec<String> {
        s.split('|')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect()
    };
    let mut orgs: Vec<(Org, String)> = Vec::new();
    for row in rows {
        if get(&row, id).is_empty() {
            continue;
        }
        let name = get(&row, name);
        let shorts = split(get(&row, short));
        let others = split(get(&row, other));
        let label = shorts.first().cloned().unwrap_or_else(|| name.clone());
        let mut forms: Vec<String> = shorts
            .iter()
            .filter(|s| s.chars().count() >= 2)
            .map(|s| s.to_lowercase())
            .collect();
        forms.extend(
            std::iter::once(&name)
                .chain(&others)
                .filter(|s| s.chars().count() >= 3)
                .map(|s| s.to_lowercase()),
        );
        orgs.push((Org { name, label, forms }, get(&row, city)));
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (org, _) in &orgs {
        *seen.entry(org.label.clone()).or_default() += 1;
    }
    orgs.into_iter()
        .map(|(mut org, city)| {
            if seen[&org.label] > 1 && !city.is_empty() && !org.label.contains(&city) {
                org.label = format!("{}, {city}", org.label);
            }
            org
        })
        .collect()
}

/// RFC 4180 rows: quoted fields, doubled quotes, newlines inside quotes.
fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let (mut row, mut field, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// The organisations a name names, as indexes into `orgs`: each form as a
/// whole word in any case, and a match inside a longer one does not count
/// ("Pravni" inside "Pravni Novi Sad").
fn matches(name: &str, orgs: &[Org]) -> Vec<usize> {
    let hay = name.to_lowercase();
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    for (i, org) in orgs.iter().enumerate() {
        for form in &org.forms {
            for (at, _) in hay.match_indices(form.as_str()) {
                let end = at + form.len();
                let before = hay[..at].chars().next_back();
                let after = hay[end..].chars().next();
                if !before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(char::is_alphanumeric)
                {
                    spans.push((at, end, i));
                }
            }
        }
    }
    let mut out: Vec<usize> = spans
        .iter()
        .filter(|(a, b, _)| {
            !spans
                .iter()
                .any(|(a2, b2, _)| a2 <= a && b <= b2 && (b2 - a2) > (b - a))
        })
        .map(|(_, _, i)| *i)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Give each person the organisation their name names, and count each
/// organisation. A name that names several (a form two share) goes to the
/// one most people are plainly in, as ReVision ranks them.
pub fn assign(people: &mut [Person], orgs: &[Org]) -> Vec<OrgCount> {
    if orgs.is_empty() {
        return Vec::new();
    }
    let found: Vec<Vec<usize>> = people.iter().map(|p| matches(&p.name, orgs)).collect();
    let mut plain = vec![0usize; orgs.len()];
    for f in found.iter().filter(|f| f.len() == 1) {
        plain[f[0]] += 1;
    }
    let mut counts: Vec<OrgCount> = orgs
        .iter()
        .map(|o| OrgCount {
            label: o.label.clone(),
            name: o.name.clone(),
            people: 0,
            messages: 0,
        })
        .collect();
    for (person, f) in people.iter_mut().zip(found) {
        let Some(&best) = f.iter().max_by_key(|&&i| (plain[i], std::cmp::Reverse(i))) else {
            continue;
        };
        person.org = orgs[best].label.clone();
        counts[best].people += 1;
        counts[best].messages += person.messages;
    }
    counts.retain(|c| c.people > 0);
    counts.sort_by_key(|c| std::cmp::Reverse((c.messages, c.people)));
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn one_person_in_two_chats_is_one_row_and_a_recount_replaces_its_chat() {
        let mut book = Book::default();
        let members = |rows: &[(&str, &str)]| {
            rows.iter()
                .map(|(k, n)| (k.to_string(), n.to_string()))
                .collect()
        };
        let counted = |rows: &[(&str, &str, usize)]| {
            rows.iter()
                .map(|(k, n, c)| (k.to_string(), (n.to_string(), *c)))
                .collect()
        };
        book.put_members(1, "A", members(&[("user1", "Ana"), ("user2", "Ana")]));
        book.put_messages(2, "B", counted(&[("user1", "Ana", 5), ("user3", "Bo", 2)]));
        let (people, messages) = book.people();
        assert!(messages);
        assert_eq!(people.len(), 3, "a namesake is somebody else");
        assert_eq!(
            (people[0].key.as_str(), people[0].messages, people[0].chats),
            ("user1", 5, 2)
        );

        // Counting chat 2 again replaces it; it is never added twice.
        book.put_messages(2, "B", counted(&[("user1", "Ana", 7)]));
        let (people, _) = book.people();
        assert_eq!(people[0].messages, 7);
        assert_eq!(people.len(), 2);

        let dir = temp("tga-people-book");
        book.save(&dir).expect("save");
        assert_eq!(Book::load(&dir).expect("load").chats, book.chats);

        // An export whose file has gone is marked, and removing a chat takes
        // its people with it, from the file as well.
        book.chats.insert(
            "3".into(),
            Chat {
                title: "C".into(),
                messages: Some(counted(&[("user9", "Ida", 1)])),
                export: Some(Seen {
                    path: dir.join("no such export"),
                    bytes: 1,
                    modified: 1,
                    handles: false,
                }),
                ..Chat::default()
            },
        );
        book.save(&dir).expect("save");
        let list = book.list(&dir, Vec::new());
        let gone: Vec<&str> = list
            .chats
            .iter()
            .filter(|c| c.file_gone)
            .map(|c| c.title.as_str())
            .collect();
        assert_eq!(gone, ["C"]);
        assert_eq!(list.chats[0].chat_id(), Some(1));
        let left = remove(&dir, "3").expect("remove");
        assert_eq!(left.chats.len(), 2);
        assert!(left.people.iter().all(|p| p.key != "user9"));
        assert!(!Book::load(&dir).expect("load").chats.contains_key("3"));

        std::fs::write(dir.join(FILE), "not json").expect("write");
        assert!(
            Book::load(&dir).is_err(),
            "a broken file is not an empty list"
        );
    }

    #[test]
    fn an_unchanged_export_is_not_read_again() {
        // The committed synthetic export names nobody, so this never skips.
        let dir = temp("tga-people-exports");
        let synthetic = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tga-report/tests/synthetic");
        let target = crate::refresh::target_for(&synthetic).expect("synthetic export");
        let reads = AtomicUsize::new(0);
        let first = update(&dir, std::slice::from_ref(&target), false, &|| {});
        assert!(first.notes.is_empty(), "{:?}", first.notes);
        assert!(!first.people.is_empty() && first.messages);
        assert_eq!(first.chats.len(), 1);
        assert!(first.chats[0].from_export && !first.chats[0].file_gone);

        let before = std::fs::metadata(dir.join(FILE))
            .expect("saved")
            .modified()
            .ok();
        let again = update(&dir, &[target.clone(), target], false, &|| {
            reads.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        assert_eq!(
            again.people, first.people,
            "the same export twice is one chat"
        );
        let after = std::fs::metadata(dir.join(FILE))
            .expect("saved")
            .modified()
            .ok();
        assert_eq!(before, after, "nothing changed, nothing written");
    }

    #[test]
    fn a_tag_in_a_name_names_its_organisation() {
        let orgs = parse_orgs(
            "id,name,kind,parent,city,country,short,other_names\r\n\
             u,Univerzitet A,university,,Grad,X,UA,\r\n\
             a,Pravni fakultet,faculty,u,Grad,X,PF,Pravni\r\n\
             b,Pravni fakultet,faculty,u,Drugi,X,PF D,\"Pravni Drugi\"\r\n\
             c,\"Elektrotehnički fakultet\",faculty,u,Grad,X,ETF,Elektrotehnički\r\n",
        );
        assert_eq!(orgs.len(), 4);
        assert_eq!(
            (orgs[1].label.as_str(), orgs[2].label.as_str()),
            ("PF", "PF D")
        );
        let label = |name: &str| {
            matches(name, &orgs)
                .iter()
                .map(|&i| orgs[i].label.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(label("Ana ETF"), ["ETF"]);
        assert_eq!(label("ana etf"), ["ETF"], "any case");
        assert_eq!(
            label("Mika Pravni Drugi"),
            ["PF D"],
            "the longer match wins"
        );
        let twins = parse_orgs("id,name,city,short\na,One,Grad,PF\nb,Two,Drugi,PF\n");
        assert_eq!(twins[1].label, "PF, Drugi", "a shared label gets its city");
        assert!(
            label("Petar").is_empty() && label("RETFA").is_empty(),
            "whole words only"
        );

        let person = |key: &str, name: &str, messages| Person {
            key: key.into(),
            name: name.into(),
            messages,
            chats: 1,
            ..Person::default()
        };
        let mut people = vec![
            person("user1", "Ana ETF", 10),
            person("user2", "Ivo etf", 5),
            person("user3", "Petar", 1),
        ];
        let counts = assign(&mut people, &orgs);
        assert_eq!(people[0].org, "ETF");
        assert!(people[2].org.is_empty());
        assert_eq!(counts.len(), 1);
        assert_eq!((counts[0].people, counts[0].messages), (2, 15));
    }
}
