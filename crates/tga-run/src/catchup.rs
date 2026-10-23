//! A catch-up: what happened in the Analysis view's chats since you last looked,
//! a short summary each, one chat after another.
//!
//! **"Since you last looked" is counted in database updates, not on the
//! clock.** Every sync of a database writes a row in its `runs` table, and
//! every message it adds carries that run's stamp as `first_seen`. A chat's
//! bookmark is the stamp of the newest update its last catch-up covered, and
//! the next catch-up reads what the updates since then added — as whole days,
//! so a message is read beside what was said around it. A time on the clock
//! would lose what a late sync brings in: catch up at 10:00 on a chat last
//! synced at 08:00, sync at 12:00, and everything said between 08:00 and 10:00
//! is older than the bookmark and never read.
//!
//! The bookmarks sit beside each database, in
//! `analysis\summaries\caught-up.json`, because reading a database writes
//! nothing into it.
//!
//! **Two passes, because the card holds one model.** Every chat's days are
//! read first, and their photos with the OCR; then Gemma writes each chat's
//! summary in turn. Reading photos per chat would load and unload both models
//! once per chat.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use chrono::{NaiveDate, TimeZone};
use serde_json::{json, Map, Value};
use tga_sum::{Cache, Client, Event, Keeper, Lang, Length, Request, Want};

use crate::job::{Wake, Waking};
use crate::summary::{self, Chat, Server};

/// A chat no catch-up has covered yet starts this many days back, today
/// included.
pub const FIRST_DAYS: i64 = 3;

/// Which days a catch-up reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Days {
    /// What each chat's updates added since its last catch-up.
    #[default]
    SinceLast,
    /// Each chat's last 24 hours: the day before its newest message, not
    /// before now (`summary::Chat::last_hours`).
    Hours24,
    /// The last N calendar days, today included.
    Last(u32),
    /// Two days, both included.
    Between(NaiveDate, NaiveDate),
}

impl Days {
    /// What Settings remembers: a range of dates is not remembered.
    pub fn code(self) -> String {
        match self {
            Days::Last(n) => n.to_string(),
            Days::Hours24 => "24h".into(),
            Days::SinceLast | Days::Between(..) => "since".into(),
        }
    }

    pub fn parse(s: &str) -> Days {
        match s.trim() {
            // "1" was Today, which 24 hours replaced on the page.
            "24h" | "1" => Days::Hours24,
            s => match s.parse::<u32>() {
                Ok(n) if n > 0 => Days::Last(n),
                _ => Days::SinceLast,
            },
        }
    }

    /// The calendar days asked for, or `None` for [`Days::SinceLast`] and
    /// [`Days::Hours24`], whose days are each chat's own.
    pub fn window(self, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
        match self {
            Days::SinceLast | Days::Hours24 => None,
            Days::Last(n) => Some((
                today - chrono::Duration::days(i64::from(n.max(1)) - 1),
                today,
            )),
            Days::Between(a, b) => Some((a.min(b), a.max(b))),
        }
    }
}

/// A chat a catch-up can cover: one of the Analysis view's databases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The database file.
    pub path: PathBuf,
    /// Which chat in it; `None` reads its largest, as the report does.
    pub chat_id: Option<i64>,
    pub title: String,
    /// The folder its summaries and bookmark live in.
    pub home: PathBuf,
}

/// What is new in a chat for the days asked, counted before anything is read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct News {
    pub messages: usize,
    /// Updates that added something since the last catch-up.
    pub updates: usize,
    /// No catch-up has covered the chat yet, so "since the last" is the last
    /// [`FIRST_DAYS`] days.
    pub first: bool,
    /// When the chat was last caught up, as unix seconds; counted only for
    /// [`Days::SinceLast`] and [`Days::Hours24`].
    pub caught_up: Option<i64>,
    /// Something is new since the chat's last catch-up: what "every chat
    /// with news" picks. For days on the calendar, simply `messages > 0`;
    /// for [`Days::Hours24`] it is not, since every chat has a last day.
    pub fresh: bool,
}

// -- bookmarks -----------------------------------------------------------------

fn bookmarks_at(home: &Path) -> PathBuf {
    home.join("analysis")
        .join("summaries")
        .join("caught-up.json")
}

fn bookmarks(home: &Path) -> Map<String, Value> {
    std::fs::read_to_string(bookmarks_at(home))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// The stamp of the newest update a catch-up of `chat_id` has covered.
pub fn bookmark(home: &Path, chat_id: i64) -> Option<i64> {
    let v = bookmarks(home).get(&chat_id.to_string())?.clone();
    // A bare number is a bookmark kept before the time beside it was.
    v.as_i64().or_else(|| v.get("through")?.as_i64())
}

/// When a catch-up last moved `chat_id`'s bookmark, as unix seconds. A
/// bookmark kept before that was recorded answers with its file's time.
pub fn caught_up_at(home: &Path, chat_id: i64) -> Option<i64> {
    let v = bookmarks(home).get(&chat_id.to_string())?.clone();
    if let Some(at) = v.get("at").and_then(Value::as_i64) {
        return Some(at);
    }
    let modified = std::fs::metadata(bookmarks_at(home))
        .ok()?
        .modified()
        .ok()?;
    let secs = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    i64::try_from(secs).ok()
}

/// How long ago `then` was at `now`, both unix seconds, in words.
pub fn ago(then: i64, now: i64) -> String {
    let minutes = (now - then).max(0) / 60;
    let (n, unit) = match minutes {
        0 => return "just now".into(),
        m if m < 60 => (m, "minute"),
        m if m < 60 * 24 => (m / 60, "hour"),
        m => (m / (60 * 24), "day"),
    };
    match n {
        1 if unit == "hour" => "an hour ago".into(),
        1 => format!("a {unit} ago"),
        n => format!("{n} {unit}s ago"),
    }
}

fn set_bookmark(home: &Path, chat_id: i64, through: i64) -> Result<(), String> {
    let mut all = bookmarks(home);
    let at = chrono::Local::now().timestamp();
    all.insert(chat_id.to_string(), json!({ "through": through, "at": at }));
    let path = bookmarks_at(home);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(all)).unwrap_or_default();
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

// -- what is new ---------------------------------------------------------------

/// The first second of `day` on this machine's clock, which is the clock a
/// message's wall-clock date was written on.
fn day_start(day: NaiveDate) -> i64 {
    let midnight = day.and_hms_opt(0, 0, 0).unwrap_or_default();
    chrono::Local
        .from_local_datetime(&midnight)
        .earliest()
        .map(|t| t.timestamp())
        .unwrap_or_else(|| midnight.and_utc().timestamp())
}

pub fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn chat_id_of(group: &Group, reader: &tga_db::Reader) -> Result<i64, String> {
    if let Some(id) = group.chat_id {
        return Ok(id);
    }
    reader
        .chats()
        .map_err(|e| format!("{e:#}"))?
        .first()
        .map(|c| c.id)
        .ok_or_else(|| "The database holds no chat.".into())
}

/// Whether `days` reads a span of each chat's own — counted back from its
/// last message — rather than days on the calendar.
///
/// "24 h" and "N days" are either, by the Summaries tick box
/// (`Options::from_last`); "since the last catch-up" is always the chat's own,
/// and two dates are always the calendar's.
fn chats_own(days: Days, from_last: bool) -> bool {
    match days {
        Days::SinceLast => true,
        Days::Hours24 | Days::Last(_) => from_last,
        Days::Between(..) => false,
    }
}

/// How much is new in `group` for `days`, from the database's indexes alone.
///
/// `from_last`: "24 h" and "N days" count back from the chat's newest
/// message rather than from now.
pub fn news(group: &Group, days: Days, from_last: bool, today: NaiveDate) -> Result<News, String> {
    let err = |e: anyhow::Error| format!("{e:#}");
    let reader = tga_db::Reader::open(&group.path).map_err(err)?;
    let chat = chat_id_of(group, &reader)?;
    let between = |a: NaiveDate, b: NaiveDate| -> Result<usize, String> {
        let from = reader.count_since(chat, day_start(a)).map_err(err)?;
        let after = reader
            .count_since(chat, day_start(b + chrono::Duration::days(1)))
            .map_err(err)?;
        Ok(from.saturating_sub(after))
    };
    if !chats_own(days, from_last) {
        // Days on the calendar: which chats is simply the ones with some.
        let messages = match days {
            Days::Hours24 => reader
                .count_since(
                    chat,
                    chrono::Local::now().timestamp() - summary::DAY_SECS + 1,
                )
                .map_err(err)?,
            _ => match days.window(today) {
                Some((a, b)) => between(a, b)?,
                None => 0,
            },
        };
        return Ok(News {
            messages,
            fresh: messages > 0,
            ..News::default()
        });
    }
    let through = reader.last_update(chat).map_err(err)?;
    let caught_up = caught_up_at(&group.home, chat);
    let mut news = match (bookmark(&group.home, chat), through) {
        (Some(b), Some(t)) => {
            let (updates, ids) = reader.added_by_updates(chat, Some(b), t).map_err(err)?;
            News {
                messages: ids.len(),
                updates,
                first: false,
                caught_up,
                fresh: false,
            }
        }
        (Some(_), None) => News {
            caught_up,
            ..News::default()
        },
        (None, _) => News {
            messages: between(today - chrono::Duration::days(FIRST_DAYS - 1), today)?,
            updates: 0,
            first: true,
            caught_up: None,
            fresh: false,
        },
    };
    news.fresh = news.messages > 0;
    // Which chats: the ones with news, as above. What is read: each one's
    // own last hours or days, counted here so the list says how much.
    let newest = reader.newest(chat).map_err(err)?;
    match days {
        Days::Hours24 => {
            news.messages = match newest {
                Some(newest) => reader
                    .count_since(chat, newest - summary::DAY_SECS + 1)
                    .map_err(err)?,
                None => 0,
            };
        }
        Days::Last(_) => {
            news.messages = match newest
                .and_then(summary::local_day)
                .and_then(|d| days.window(d))
            {
                Some((a, b)) => between(a, b)?,
                None => 0,
            };
        }
        _ => {}
    }
    // Never caught up: everything in its own span is new to whoever reads it,
    // however long ago the chat went quiet.
    if news.first && days != Days::SinceLast {
        news.fresh = news.messages > 0;
    }
    Ok(news)
}

/// [`news`] for every group on a worker, each answer as it comes.
pub fn survey(
    groups: Vec<Group>,
    days: Days,
    from_last: bool,
    wake: Wake,
) -> Receiver<(PathBuf, Result<News, String>)> {
    let (tx, rx) = channel();
    let tx = Waking { tx, wake };
    std::thread::Builder::new()
        .name("tga-catchup-survey".into())
        .spawn(move || {
            let today = today();
            for g in groups {
                let found = news(&g, days, from_last, today);
                tx.send((g.path, found));
            }
        })
        .expect("the OS refused a worker thread");
    rx
}

// -- the run -------------------------------------------------------------------

/// What a catch-up writes.
#[derive(Debug, Clone)]
pub struct Options {
    pub days: Days,
    /// "24 h" and "N days" count back from each chat's last message, not
    /// from now.
    pub from_last: bool,
    /// A forum's topics one by one, rather than the chat as one.
    pub by_topic: bool,
    pub lang: Lang,
}

/// What a catch-up tells the window.
#[derive(Debug, Clone)]
pub enum Progress {
    /// The run as a whole: the model starting, photos being read. Never a
    /// message's text.
    Step(String),
    /// One chat, by its place in the list the run was given.
    Group(usize, Step),
    Done(Result<Finished, String>),
}

/// Where one chat has got to.
#[derive(Debug, Clone)]
pub enum Step {
    Reading,
    /// Its days are read, and wait for the model.
    Read {
        messages: usize,
        from: NaiveDate,
        to: NaiveDate,
    },
    Writing,
    Event(Event),
    /// Written and saved — or why not. "Nothing new" is an `Err` too.
    Done(Result<Card, String>),
}

/// One chat's summary, saved.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub messages: usize,
    /// The summary, without the file's header.
    pub text: String,
    /// The saved file.
    pub path: PathBuf,
    /// Only what was sent after this, unix seconds: a card of the chat's
    /// last 24 hours.
    pub after: Option<i64>,
    /// Parts the model gave no answer for, of how many.
    pub failed: usize,
    pub parts: usize,
    pub cut: bool,
    pub unread_photos: usize,
}

/// A catch-up finished.
#[derive(Debug, Clone)]
pub struct Finished {
    /// The record of it, for Earlier catch-ups; `None` when nothing was
    /// written.
    pub record: Option<PathBuf>,
    pub written: usize,
}

/// What every card of a catch-up says when nothing new was said.
pub const NOTHING_NEW: &str = "Nothing new";

pub struct Job {
    pub events: Receiver<Progress>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// Stop after what is being written now is cut short. Chats already
    /// written stay saved, with their bookmarks moved.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Catch up on `groups`, in their order, on a worker. `records` is the folder
/// the record of the run goes in.
pub fn spawn(
    groups: Vec<Group>,
    options: Options,
    server: Server,
    keeper: Arc<Keeper>,
    records: PathBuf,
    wake: Wake,
) -> Job {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let tx = Waking { tx, wake };
    std::thread::Builder::new()
        .name("tga-catchup".into())
        .spawn(move || {
            let done = run(&groups, &options, &server, &keeper, &records, &flag, &tx);
            keeper.touch();
            tx.send(Progress::Done(done));
        })
        .expect("the OS refused a worker thread");
    Job { events: rx, cancel }
}

/// One chat's days, read and waiting for the model.
struct Prepared {
    chat_id: i64,
    title: String,
    topics: Vec<String>,
    forum: bool,
    home: PathBuf,
    rows: Vec<tga_notes::Row>,
    from: NaiveDate,
    to: NaiveDate,
    /// The chat's last 24 hours: what was sent after this.
    after: Option<i64>,
    unread_photos: usize,
}

const CANCELLED: &str = "cancelled";

fn run(
    groups: &[Group],
    options: &Options,
    server: &Server,
    keeper: &Keeper,
    records: &Path,
    cancel: &AtomicBool,
    tx: &Waking<Progress>,
) -> Result<Finished, String> {
    let today = today();
    let say = |s: String| tx.send(Progress::Step(s));
    // The bookmark each chat moves to once its summary is saved.
    let mut through: Vec<Option<(i64, i64)>> = vec![None; groups.len()];
    let mut ready: Vec<(usize, Prepared)> = Vec::new();
    for (i, g) in groups.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        tx.send(Progress::Group(i, Step::Reading));
        match prepare(g, options, server, keeper, cancel, today, &say) {
            Ok((found, mark)) => {
                through[i] = mark;
                match found {
                    Some(p) => {
                        tx.send(Progress::Group(
                            i,
                            Step::Read {
                                messages: p.rows.len(),
                                from: p.from,
                                to: p.to,
                            },
                        ));
                        ready.push((i, p));
                    }
                    None => {
                        // Nothing to say is said, and the bookmark moves:
                        // the updates were read.
                        if let Some((chat, t)) = mark {
                            let _ = set_bookmark(&g.home, chat, t);
                        }
                        tx.send(Progress::Group(i, Step::Done(Err(NOTHING_NEW.into()))));
                    }
                }
            }
            Err(e) if e == CANCELLED => return Err(e),
            Err(e) => tx.send(Progress::Group(i, Step::Done(Err(e)))),
        }
    }
    if ready.is_empty() {
        return Ok(Finished {
            record: None,
            written: 0,
        });
    }

    let client = Client::new(server.port);
    keeper
        .ensure(
            &client,
            server.launch.as_ref(),
            Want::Gemma,
            cancel,
            &mut |s| say(s),
        )
        .map_err(|e| match e {
            tga_sum::Error::NoServer(port) if server.launch.is_none() => format!(
                "Nothing is serving on 127.0.0.1:{port}, and there is no llama-server and \
                 model to start: the models folder lacks one or the other. Settings › \
                 Summaries › How to add a model says what goes there."
            ),
            e => e.to_string(),
        })?;

    let written_at = chrono::Local::now();
    let mut kept: Vec<Value> = Vec::new();
    let mut stopped = false;
    for (i, p) in &ready {
        if cancel.load(Ordering::Relaxed) {
            stopped = true;
            break;
        }
        tx.send(Progress::Group(*i, Step::Writing));
        let request = Request {
            chat: &p.title,
            rows: &p.rows,
            topics: &p.topics,
            forum: p.forum,
            by_topic: options.by_topic && p.forum,
            length: Length::Brief,
            lang: options.lang,
            from: p.from,
            to: p.to,
        };
        let cache = Cache::at(&p.home.join("analysis").join("summaries").join("cache"));
        let summary = match tga_sum::summarise(&client, &request, &cache, cancel, &mut |e| {
            keeper.touch();
            tx.send(Progress::Group(*i, Step::Event(e)));
        }) {
            Ok(s) => s,
            Err(tga_sum::Error::Cancelled) => {
                stopped = true;
                tx.send(Progress::Group(*i, Step::Done(Err("Stopped".into()))));
                break;
            }
            Err(e) => {
                tx.send(Progress::Group(*i, Step::Done(Err(e.to_string()))));
                continue;
            }
        };
        let saved = summary::Options {
            from: p.from,
            to: p.to,
            length: Length::Brief,
            lang: options.lang,
            by_topic: request.by_topic,
            topic: None,
            after: p.after,
        };
        let path = saved.path(&p.home, &p.topics);
        let document = summary::document(
            &p.title,
            &p.topics,
            &saved,
            &summary,
            &written_at.format("%Y-%m-%d %H:%M").to_string(),
        );
        let wrote = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, &document));
        if let Err(e) = wrote {
            tx.send(Progress::Group(
                *i,
                Step::Done(Err(format!("{}: {e}", path.display()))),
            ));
            continue;
        }
        // **Only a summary that was saved moves the bookmark**: a chat that
        // failed is read again next time.
        if let Some((chat, t)) = through[*i] {
            if let Err(e) = set_bookmark(&p.home, chat, t) {
                say(format!("The bookmark was not saved: {e}"));
            }
        }
        let card = Card {
            from: p.from,
            to: p.to,
            messages: summary.messages,
            text: summary.body(),
            path,
            after: p.after,
            failed: summary.failed,
            parts: summary.parts,
            cut: summary.cut,
            unread_photos: p.unread_photos,
        };
        kept.push(record_card(&groups[*i], p.chat_id, &card));
        tx.send(Progress::Group(*i, Step::Done(Ok(card))));
    }

    let written = kept.len();
    let record = if kept.is_empty() {
        None
    } else {
        Some(write_record(records, options, &written_at, kept)?)
    };
    if stopped && record.is_none() {
        return Err(CANCELLED.into());
    }
    Ok(Finished { record, written })
}

/// Read one chat's days for the model. With them, the bookmark the chat
/// moves to — `(chat id, update stamp)` — when this is a catch-up since the
/// last; `None` for days that are not.
#[allow(clippy::type_complexity)]
fn prepare(
    group: &Group,
    options: &Options,
    server: &Server,
    keeper: &Keeper,
    cancel: &AtomicBool,
    today: NaiveDate,
    say: &dyn Fn(String),
) -> Result<(Option<Prepared>, Option<(i64, i64)>), String> {
    let err = |e: anyhow::Error| format!("{e:#}");
    let reader = tga_db::Reader::open(&group.path).map_err(err)?;
    let chat_id = chat_id_of(group, &reader)?;
    let mut mark = None;
    let own = chats_own(options.days, options.from_last);
    if options.days == Days::Hours24 {
        // Reading a chat's own last day is catching up on it: the bookmark
        // moves as it does for "since the last". The last 24 hours of the
        // calendar are not, and leave it where it is.
        let mark = if own {
            reader
                .last_update(chat_id)
                .map_err(err)?
                .map(|t| (chat_id, t))
        } else {
            None
        };
        drop(reader);
        let chat = Chat::open(&group.path, Some(&chat_id.to_string())).map_err(err)?;
        let hours = if own {
            chat.last_hours()
        } else {
            chat.hours_before(chrono::Local::now().timestamp())
        };
        let Some((after, from, to)) = hours else {
            return Ok((None, mark));
        };
        let rows = chat.rows_where(|m| m.unix > after, None);
        return finish(
            group,
            chat_id,
            &chat,
            rows,
            (from, to, Some(after)),
            options.lang,
            server,
            keeper,
            cancel,
            say,
        )
        .map(|p| (p, mark));
    }
    // Either the ids the updates added, whose days are read, or the days.
    let mut added: Option<HashSet<i64>> = None;
    // A chat's own last N days end on its newest message's day, and reading
    // them is catching up on it, so the bookmark moves.
    let end = match options.days {
        Days::Last(_) if own => {
            mark = reader
                .last_update(chat_id)
                .map_err(err)?
                .map(|t| (chat_id, t));
            match reader
                .newest(chat_id)
                .map_err(err)?
                .and_then(summary::local_day)
            {
                Some(d) => d,
                None => return Ok((None, mark)),
            }
        }
        _ => today,
    };
    let window = match options.days.window(end) {
        Some(w) => Some(w),
        None => {
            let t = reader.last_update(chat_id).map_err(err)?;
            mark = t.map(|t| (chat_id, t));
            match (bookmark(&group.home, chat_id), t) {
                (Some(b), Some(t)) => {
                    let (_, ids) = reader.added_by_updates(chat_id, Some(b), t).map_err(err)?;
                    if ids.is_empty() {
                        return Ok((None, mark));
                    }
                    added = Some(ids.into_iter().collect());
                    None
                }
                (Some(_), None) => return Ok((None, mark)),
                (None, _) => Some((today - chrono::Duration::days(FIRST_DAYS - 1), today)),
            }
        }
    };
    drop(reader);

    let chat = Chat::open(&group.path, Some(&chat_id.to_string())).map_err(err)?;
    let msgs = &chat.export.msgs;
    let days: BTreeSet<NaiveDate> = match (&added, window) {
        (Some(ids), _) => msgs
            .iter()
            .filter(|m| ids.contains(&m.id))
            .map(|m| m.when.date())
            .collect(),
        (None, Some((a, b))) => msgs
            .iter()
            .map(|m| m.when.date())
            .filter(|d| (a..=b).contains(d))
            .collect(),
        (None, None) => BTreeSet::new(),
    };
    let (Some(&from), Some(&to)) = (days.first(), days.last()) else {
        return Ok((None, mark));
    };
    let rows = chat.rows_where(|m| days.contains(&m.when.date()), None);
    finish(
        group,
        chat_id,
        &chat,
        rows,
        (from, to, None),
        options.lang,
        server,
        keeper,
        cancel,
        say,
    )
    .map(|p| (p, mark))
}

/// The rows picked, their linked pages and photos read: one chat ready for
/// the model, or `None` when nothing was picked. `lang` is the catch-up's,
/// which the pages are asked for in.
#[allow(clippy::too_many_arguments)]
fn finish(
    group: &Group,
    chat_id: i64,
    chat: &Chat,
    mut rows: Vec<tga_notes::Row>,
    (from, to, after): (NaiveDate, NaiveDate, Option<i64>),
    lang: Lang,
    server: &Server,
    keeper: &Keeper,
    cancel: &AtomicBool,
    say: &dyn Fn(String),
) -> Result<Option<Prepared>, String> {
    if rows.is_empty() {
        return Ok(None);
    }
    if server.links {
        summary::read_links(chat, &mut rows, lang, cancel, say)?;
    }
    let unread_photos = if server.photos {
        summary::read_photos(chat, &mut rows, server, keeper, cancel, say)?
    } else {
        0
    };
    Ok(Some(Prepared {
        chat_id,
        title: chat.title().to_string(),
        topics: chat.topics(),
        forum: chat.forum(),
        home: group.home.clone(),
        rows,
        from,
        to,
        after,
        unread_photos,
    }))
}

// -- the record ----------------------------------------------------------------

/// A catch-up written earlier, as Earlier catch-ups lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub path: PathBuf,
    /// When it was written, `YYYY-MM-DD HH:MM`.
    pub written: String,
    /// "since the last catch-up", "last 3 days", …
    pub days: String,
    pub cards: Vec<(Group, Card)>,
}

fn record_card(group: &Group, chat_id: i64, card: &Card) -> Value {
    json!({
        "title": group.title,
        "database": group.path.to_string_lossy(),
        "chat_id": chat_id,
        "home": group.home.to_string_lossy(),
        "from": card.from.to_string(),
        "to": card.to.to_string(),
        "messages": card.messages,
        "file": card.path.to_string_lossy(),
        "text": card.text,
        "after": card.after,
    })
}

/// What a catch-up read, in words. `from_last` as in [`Options`].
pub fn describe(days: Days, from_last: bool) -> String {
    match days {
        Days::SinceLast => "since the last catch-up".into(),
        Days::Hours24 if from_last => "in each chat's last 24 hours".into(),
        Days::Hours24 => "in the last 24 hours".into(),
        Days::Last(1) if from_last => "on each chat's last day".into(),
        Days::Last(n) if from_last => format!("in each chat's last {n} days"),
        Days::Last(1) => "today".into(),
        Days::Last(n) => format!("the last {n} days"),
        Days::Between(a, b) if a == b => a.to_string(),
        Days::Between(a, b) => format!("{a} – {b}"),
    }
}

fn write_record(
    records: &Path,
    options: &Options,
    at: &chrono::DateTime<chrono::Local>,
    cards: Vec<Value>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(records).map_err(|e| format!("{}: {e}", records.display()))?;
    let path = records.join(format!("{}.json", at.format("%Y-%m-%d_%H%M%S")));
    let body = json!({
        "written": at.format("%Y-%m-%d %H:%M").to_string(),
        "days": describe(options.days, options.from_last),
        "cards": cards,
    });
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&body).unwrap_or_default(),
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Every catch-up recorded in `records`, newest first. One that does not
/// read is left out.
pub fn records(records: &Path) -> Vec<Record> {
    let Ok(dir) = std::fs::read_dir(records) else {
        return Vec::new();
    };
    let mut out: Vec<Record> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| read_record(&p))
        .collect();
    out.sort_by(|a, b| b.path.cmp(&a.path));
    out
}

pub fn read_record(path: &Path) -> Option<Record> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let s = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let day = |v: &Value, k: &str| NaiveDate::parse_from_str(&s(v, k), "%Y-%m-%d").ok();
    let cards = v
        .get("cards")?
        .as_array()?
        .iter()
        .filter_map(|c| {
            Some((
                Group {
                    path: PathBuf::from(s(c, "database")),
                    chat_id: c.get("chat_id").and_then(Value::as_i64),
                    title: s(c, "title"),
                    home: PathBuf::from(s(c, "home")),
                },
                Card {
                    from: day(c, "from")?,
                    to: day(c, "to")?,
                    messages: c.get("messages").and_then(Value::as_u64).unwrap_or(0) as usize,
                    text: s(c, "text"),
                    path: PathBuf::from(s(c, "file")),
                    after: c.get("after").and_then(Value::as_i64),
                    failed: 0,
                    parts: 0,
                    cut: false,
                    unread_photos: 0,
                },
            ))
        })
        .collect();
    Some(Record {
        path: path.to_path_buf(),
        written: s(&v, "written"),
        days: s(&v, "days"),
        cards,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_days_asked_for_end_today_and_a_range_is_put_in_order() {
        let today = day("2026-10-16");
        assert_eq!(Days::SinceLast.window(today), None);
        assert_eq!(Days::Last(1).window(today), Some((today, today)));
        assert_eq!(
            Days::Last(3).window(today),
            Some((day("2026-10-14"), today))
        );
        assert_eq!(
            Days::Between(day("2026-10-10"), day("2026-10-08")).window(today),
            Some((day("2026-10-08"), day("2026-10-10")))
        );
        assert_eq!(Days::parse(&Days::Last(7).code()), Days::Last(7));
        assert_eq!(Days::parse(&Days::SinceLast.code()), Days::SinceLast);
        assert_eq!(Days::parse("nonsense"), Days::SinceLast);
        assert_eq!(Days::parse(&Days::Hours24.code()), Days::Hours24);
        // Today is gone from the page; a settings file that chose it opens on
        // the 24 hours that replaced it.
        assert_eq!(Days::parse("1"), Days::Hours24);
        assert_eq!(Days::Hours24.window(today), None);
    }

    /// A database synced twice: the first update stores 1 and 2 on the 14th
    /// and 3 on the 16th, the second adds 4 on the 16th.
    fn synced_twice(name: &str) -> Group {
        const CHAT: i64 = 4343;
        let base = std::env::temp_dir().join(format!("tga-catchup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("Dev");
        let db = home.join("Dev.sqlite");
        std::fs::create_dir_all(&home).unwrap();
        let message = |id: i64, on: &str| {
            let noon = day_start(day(on)) + 12 * 3600;
            serde_json::json!({
                "id": id, "type": "message", "date": format!("{on}T12:00:00"),
                "date_unixtime": noon.to_string(), "from": "Ana", "from_id": "user1",
                "text": format!("poruka {id}"),
            })
        };
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                let store = tgx_archive::Store::open(&db).await.unwrap();
                store
                    .upsert_chat(CHAT, "Dev", "private_group", 100)
                    .await
                    .unwrap();
                let head = serde_json::json!({});
                store
                    .upsert_topic(CHAT, 0, "Dev", head.as_object().unwrap(), 100)
                    .await
                    .unwrap();
                for (stamp, batch) in [
                    (
                        100,
                        vec![(1, "2026-10-14"), (2, "2026-10-14"), (3, "2026-10-16")],
                    ),
                    (200, vec![(4, "2026-10-16")]),
                ] {
                    let run = store
                        .start_run(CHAT, "window", None, "{}", stamp)
                        .await
                        .unwrap();
                    for (id, on) in &batch {
                        let m = message(*id, on);
                        store
                            .merge(CHAT, *id, Some(0), m.as_object().unwrap(), stamp)
                            .await
                            .unwrap();
                    }
                    store
                        .finish_run(run, batch.len(), 0, 0, 0, true, stamp)
                        .await
                        .unwrap();
                }
            });
        Group {
            path: db,
            chat_id: Some(CHAT),
            title: "Dev".into(),
            home,
        }
    }

    fn quiet_server() -> Server {
        Server {
            port: 1,
            launch: None,
            photos: false,
            ocr: None,
            links: false,
        }
    }

    #[test]
    fn since_the_last_catch_up_is_what_the_updates_since_it_added_read_as_whole_days() {
        let g = synced_twice("since");
        let today = day("2026-10-16");
        // Never caught up: the last three days, every message in them.
        assert_eq!(
            news(&g, Days::SinceLast, true, today).unwrap(),
            News {
                messages: 4,
                updates: 0,
                first: true,
                caught_up: None,
                fresh: true,
            }
        );
        // Caught up through the first update: the second added one message.
        set_bookmark(&g.home, 4343, 100).unwrap();
        let found = news(&g, Days::SinceLast, true, today).unwrap();
        assert!(found.caught_up.is_some());
        assert_eq!(
            found,
            News {
                messages: 1,
                updates: 1,
                first: false,
                caught_up: found.caught_up,
                fresh: true,
            }
        );
        // The last 24 hours pick the same chats and count the chat's last day:
        // 3 and 4, both at noon on the 16th.
        let hours = news(&g, Days::Hours24, true, today).unwrap();
        assert_eq!((hours.messages, hours.fresh), (2, true));
        let options = Options {
            days: Days::SinceLast,
            from_last: true,
            by_topic: false,
            lang: Lang::English,
        };
        let keeper = Keeper::new();
        let never = AtomicBool::new(false);
        let (found, mark) = prepare(
            &g,
            &options,
            &quiet_server(),
            &keeper,
            &never,
            today,
            &|_| {},
        )
        .unwrap();
        let found = found.unwrap();
        // The day the new message is on, whole: 3 was read before, and is
        // what 4 is said beside. The 14th had nothing new.
        assert_eq!(
            found.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert_eq!((found.from, found.to), (today, today));
        assert_eq!(mark, Some((4343, 200)));

        // Caught up through the second: nothing new, and the mark stays put.
        set_bookmark(&g.home, 4343, 200).unwrap();
        assert_eq!(news(&g, Days::SinceLast, true, today).unwrap().messages, 0);
        let (found, mark) = prepare(
            &g,
            &options,
            &quiet_server(),
            &keeper,
            &never,
            today,
            &|_| {},
        )
        .unwrap();
        assert!(found.is_none());
        assert_eq!(mark, Some((4343, 200)));

        // The last 24 hours read the chat's last day by the clock, and move
        // the bookmark as a catch-up does.
        let hours = Options {
            days: Days::Hours24,
            ..options.clone()
        };
        let (found, mark) =
            prepare(&g, &hours, &quiet_server(), &keeper, &never, today, &|_| {}).unwrap();
        let found = found.unwrap();
        assert_eq!(
            found.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert!(found.after.is_some());
        assert_eq!(mark, Some((4343, 200)));

        // Days on the calendar ignore the bookmark.
        assert_eq!(news(&g, Days::Last(1), false, today).unwrap().messages, 2);
        assert_eq!(
            news(
                &g,
                Days::Between(day("2026-10-14"), day("2026-10-15")),
                false,
                today
            )
            .unwrap()
            .messages,
            2
        );
        let _ = std::fs::remove_dir_all(g.home.parent().unwrap());
    }

    #[test]
    fn n_days_count_back_from_the_last_message_or_from_today_as_ticked() {
        let g = synced_twice("anchor");
        // Six weeks after the chat went quiet: counted from today, the last
        // day is empty; counted from its last message, it is the 16th.
        let later = day("2026-12-01");
        let calendar = news(&g, Days::Last(1), false, later).unwrap();
        assert_eq!((calendar.messages, calendar.fresh), (0, false));
        let own = news(&g, Days::Last(1), true, later).unwrap();
        assert_eq!(own.messages, 2);
        // Its own days are picked like "since the last": by what is new.
        assert!(own.fresh);
        assert_eq!(news(&g, Days::Last(3), true, later).unwrap().messages, 4);

        let options = Options {
            days: Days::Last(3),
            from_last: true,
            by_topic: false,
            lang: Lang::English,
        };
        let keeper = Keeper::new();
        let never = AtomicBool::new(false);
        let (found, mark) = prepare(
            &g,
            &options,
            &quiet_server(),
            &keeper,
            &never,
            later,
            &|_| {},
        )
        .unwrap();
        let found = found.unwrap();
        assert_eq!(
            found.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            (found.from, found.to),
            (day("2026-10-14"), day("2026-10-16"))
        );
        // Reading a chat's own last days is catching up on it.
        assert_eq!(mark, Some((4343, 200)));

        // Counted from today, the same three days hold nothing.
        let calendar = Options {
            from_last: false,
            ..options
        };
        let (found, mark) = prepare(
            &g,
            &calendar,
            &quiet_server(),
            &keeper,
            &never,
            later,
            &|_| {},
        )
        .unwrap();
        assert!(found.is_none());
        assert_eq!(mark, None);
        assert_eq!(describe(Days::Last(3), true), "in each chat's last 3 days");
        assert_eq!(describe(Days::Last(3), false), "the last 3 days");
        let _ = std::fs::remove_dir_all(g.home.parent().unwrap());
    }

    #[test]
    fn a_bookmark_is_kept_per_chat_beside_the_database() {
        let home = std::env::temp_dir().join(format!("tga-catchup-mark-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(bookmark(&home, 7), None);
        set_bookmark(&home, 7, 100).unwrap();
        set_bookmark(&home, 8, 50).unwrap();
        set_bookmark(&home, 7, 200).unwrap();
        assert_eq!(bookmark(&home, 7), Some(200));
        assert_eq!(bookmark(&home, 8), Some(50));
        assert!(bookmarks_at(&home).is_file());
        let now = chrono::Local::now().timestamp();
        assert!(caught_up_at(&home, 7).is_some_and(|at| (now - at).abs() < 60));
        assert_eq!(caught_up_at(&home, 9), None);
        // A bookmark from before the time was kept: the number alone, dated
        // by its file.
        std::fs::write(bookmarks_at(&home), r#"{"7": 300}"#).unwrap();
        assert_eq!(bookmark(&home, 7), Some(300));
        assert!(caught_up_at(&home, 7).is_some_and(|at| (now - at).abs() < 60));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn how_long_ago_is_said_in_its_largest_whole_unit() {
        let t = 1_000_000;
        assert_eq!(ago(t, t + 30), "just now");
        assert_eq!(ago(t, t + 60), "a minute ago");
        assert_eq!(ago(t, t + 59 * 60), "59 minutes ago");
        assert_eq!(ago(t, t + 90 * 60), "an hour ago");
        assert_eq!(ago(t, t + 23 * 3600), "23 hours ago");
        assert_eq!(ago(t, t + 24 * 3600), "a day ago");
        assert_eq!(ago(t, t + 3 * 86400 + 5), "3 days ago");
        assert_eq!(ago(t + 10, t), "just now");
    }

    #[test]
    fn a_record_reads_back_as_the_cards_it_was_written_from() {
        let dir = std::env::temp_dir().join(format!("tga-catchup-rec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let group = Group {
            path: PathBuf::from("X/X.sqlite"),
            chat_id: Some(7),
            title: "Dev".into(),
            home: PathBuf::from("X"),
        };
        let card = Card {
            from: day("2026-10-14"),
            to: day("2026-10-16"),
            messages: 12,
            text: "**Gist.**\n- A thing [#3]".into(),
            path: PathBuf::from("X/analysis/summaries/a.md"),
            after: Some(1_760_000_000),
            failed: 0,
            parts: 0,
            cut: false,
            unread_photos: 0,
        };
        let at = chrono::Local::now();
        let path = write_record(
            &dir,
            &Options {
                days: Days::Last(3),
                from_last: false,
                by_topic: false,
                lang: Lang::English,
            },
            &at,
            vec![record_card(&group, 7, &card)],
        )
        .unwrap();
        let all = records(&dir);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].path, path);
        assert_eq!(all[0].days, "the last 3 days");
        assert_eq!(all[0].cards, vec![(group, card)]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
