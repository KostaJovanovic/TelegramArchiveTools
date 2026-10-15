//! `TelegramTools summarise`: what happened in a chat between two dates, in a
//! local model's words.
//!
//! This half reads the chat — a database or a folder export, through the same
//! reader as the report — picks the range's messages by **wall clock**
//! (`Msg::when`: a day is the day on the calendar), maps them to digest rows,
//! and hands them to `tga-sum`, which does the cutting, the prompting and the
//! talking to the model. The summary is written beside the export, in
//! `analysis\summaries\`, and printed as it is written.
//!
//! **stdout is the summary; stderr is the narration.** The narration names
//! counts and timings and never a message, the rule `tga-sum` holds for its
//! own events.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use tga_sum::ocr::{Photo, Readings};
use tga_sum::{
    Cache, Client, Event, Grounding, Keeper, Lang, Launch, Length, Request, Summary, Turn, Want,
};

use crate::cli::{digest_row, thousands, Stored};

pub const USAGE: &str = "\
TelegramTools summarise -- a local model's summary of a date range

    TelegramTools summarise <database or export folder> --from <day> [options]

    `summarize` and `summary` are the same command. Needs a llama-server
    already running on this machine with the model loaded (the window's
    Summaries view starts one); nothing leaves 127.0.0.1.

Options
    --from YYYY-MM-DD   the first day, inclusive
    --to YYYY-MM-DD     the last day, inclusive (default: the --from day)
    --short             one paragraph (the default)
    --detailed          paragraphs by theme, citing message ids as [#id]
    --sections          headed bullet points: key events, decisions, plans,
                        disagreements, open questions; cited
    --by-topic          a forum: one summary per topic, busiest first, with an
                        overview above them when --detailed or --sections
    --topic NAME        a forum: that one topic only
    --lang sr|en|chat   Serbian (Latin, ekavian), English, or whatever the chat
                        is mostly written in. Default: sr
    --chat ID|TITLE     which chat, in a database holding more than one
    --port N            the model server's port on 127.0.0.1 (default 8080)
    --out PATH          where to write it (default: beside the export, in
                        analysis\\summaries\\)
    --no-cache          ask the model again for every part
    --any-model         use whatever model the server has loaded, not only
                        a Gemma; the prompts and sampling are tuned for Gemma
";

/// A chat read for summarising: the export, and what turns its messages into
/// rows.
pub struct Chat {
    pub export: tga_read::Export,
    people: tga_metrics::People,
    stored: Option<Stored>,
    /// The folder the export's own outputs go in: the database's folder, or
    /// the export folder.
    pub home: PathBuf,
}

impl Chat {
    /// Read a database (the file, or a folder holding one) or a folder export.
    pub fn open(path: &Path, chat: Option<&str>) -> Result<Chat> {
        let database = tga_db::find(path);
        if database.is_none() && !path.is_dir() {
            bail!("Not a database or an export folder: {}", path.display());
        }
        let export = match &database {
            Some(db) => tga_db::load(db, chat, None)?,
            None => tga_read::load(path, None)?,
        };
        if export.msgs.is_empty() {
            bail!("That export has no messages in it.");
        }
        let people = tga_metrics::People::new(&export);
        let stored = database.as_deref().and_then(|db| Stored::open(db, &export));
        let home = match &database {
            Some(db) => crate::refresh::beside(db),
            None => path.to_path_buf(),
        };
        Ok(Chat {
            export,
            people,
            stored,
            home,
        })
    }

    pub fn title(&self) -> &str {
        &self.export.name
    }

    /// A forum, split into topics. Anything else has one.
    pub fn forum(&self) -> bool {
        self.export.topics.len() > 1
    }

    pub fn topics(&self) -> Vec<String> {
        self.export.topics.iter().map(|t| t.name.clone()).collect()
    }

    /// A topic by its name, or failing that by its number from 1.
    pub fn topic(&self, wanted: &str) -> Option<usize> {
        let t = &self.export.topics;
        t.iter()
            .position(|t| t.name.eq_ignore_ascii_case(wanted))
            .or_else(|| {
                t.iter()
                    .position(|t| t.name.to_lowercase().contains(&wanted.to_lowercase()))
            })
            .or_else(|| {
                wanted
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=t.len()).contains(n))
                    .map(|n| n - 1)
            })
    }

    /// The first and last day anything was said.
    pub fn span(&self) -> Option<(NaiveDate, NaiveDate)> {
        let first = self.export.msgs.iter().map(|m| m.when.date()).min()?;
        let last = self.export.msgs.iter().map(|m| m.when.date()).max()?;
        Some((first, last))
    }

    /// The messages between two days, both inclusive, oldest first — of one
    /// topic, or of all. Attachments are read here, so only the range's are.
    pub fn rows(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        topic: Option<usize>,
    ) -> Vec<tga_notes::Row> {
        let mut picked: Vec<&tga_read::Msg> = self
            .export
            .msgs
            .iter()
            .filter(|m| (from..=to).contains(&m.when.date()))
            .filter(|m| topic.is_none_or(|t| m.topic == t))
            .collect();
        picked.sort_by_key(|m| (m.when, m.id));
        picked
            .into_iter()
            .map(|m| digest_row(&self.export, m, &self.people, self.stored.as_ref()))
            .collect()
    }

    /// The photos among `rows`: the row each is on, a name for the picture
    /// that holds across runs, and where its bytes are. A folder export's
    /// photo is a file on disk; a database's is a stored blob.
    fn pictures(&self, rows: &[tga_notes::Row]) -> Vec<(usize, String, Picture)> {
        let at: std::collections::HashMap<i64, usize> =
            rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        let mut out = Vec::new();
        for m in &self.export.msgs {
            let Some(&row) = at.get(&m.id) else { continue };
            if m.media != "photo" {
                continue;
            }
            let on_disk = (!m.file.is_empty())
                .then(|| self.export.topics.get(m.topic))
                .flatten()
                .map(|t| t.folder.join(&m.file))
                .filter(|p| p.is_file());
            if let Some(path) = on_disk {
                let size = path.metadata().map(|d| d.len()).unwrap_or(0);
                out.push((row, format!("file:{}:{size}", m.file), Picture::Disk(path)));
            } else if let Some((key, file_id, kind)) =
                self.stored.as_ref().and_then(|s| s.picture(m.id))
            {
                out.push((row, key, Picture::Stored(file_id, kind)));
            }
        }
        out.sort_by_key(|(row, ..)| *row);
        out
    }

    fn bytes(&self, picture: &Picture) -> Option<Vec<u8>> {
        match picture {
            Picture::Disk(path) => std::fs::read(path).ok(),
            Picture::Stored(file_id, kind) => self.stored.as_ref()?.blob(*file_id, kind),
        }
    }
}

/// Where a photo's bytes are.
enum Picture {
    Disk(PathBuf),
    Stored(i64, String),
}

/// Photos read for one OCR call's worth of progress, and saved between: a
/// stopped run keeps what it read.
const PHOTO_BATCH: usize = 40;

/// The OCR model's file name, which the readings are kept under.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Fill `rows`' photo text from what was read before; the photos never read
/// are returned, with the key their reading will be kept under.
fn kept_readings(
    readings: &Readings,
    model: &str,
    rows: &mut [tga_notes::Row],
    pictures: Vec<(usize, String, Picture)>,
) -> Vec<(usize, String, Picture)> {
    let mut todo = Vec::new();
    for (row, name, picture) in pictures {
        let key = Readings::key(model, &name);
        match readings.get(&key) {
            Some(text) => rows[row].photo_text = text.to_string(),
            None => todo.push((row, key, picture)),
        }
    }
    todo
}

/// The readings file beside an export.
fn readings_at(home: &Path) -> Readings {
    Readings::at(
        &home
            .join("analysis")
            .join("summaries")
            .join("cache")
            .join("photos.json"),
    )
}

/// What to write.
#[derive(Debug, Clone)]
pub struct Options {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub length: Length,
    pub lang: Lang,
    pub by_topic: bool,
    pub topic: Option<usize>,
}

impl Options {
    /// The file a summary of these options is saved as, under `home`.
    pub fn path(&self, home: &Path, chat: &Chat) -> PathBuf {
        let mut name = if self.from == self.to {
            self.from.to_string()
        } else {
            format!("{}_{}", self.from, self.to)
        };
        if let Some(t) = self.topic {
            name.push('_');
            name.push_str(&slug(&chat.topics()[t]));
        } else if self.by_topic {
            name.push_str("_by-topic");
        }
        name.push_str(&format!("_{}_{}.md", self.length.code(), self.lang.code()));
        home.join("analysis").join("summaries").join(name)
    }
}

/// A topic's name made safe for a filename.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(40).collect();
    if out.is_empty() {
        "topic".into()
    } else {
        out
    }
}

/// The saved file: a header saying what this is, then the summary.
pub fn document(chat: &Chat, options: &Options, summary: &Summary, written: &str) -> String {
    let span = if options.from == options.to {
        options.from.to_string()
    } else {
        format!("{} – {}", options.from, options.to)
    };
    let scope = match options.topic {
        Some(t) => format!("topic “{}”", chat.topics()[t]),
        None if options.by_topic => "by topic".into(),
        None => "whole group".into(),
    };
    let lang = match options.lang {
        Lang::Serbian => "Serbian",
        Lang::English => "English",
        Lang::AsChat => "the chat's language",
    };
    let mut out = format!(
        "# {} · {span}\n\n{scope} · {} · {lang} · {} messages · {} · written {written}\n\n",
        chat.title(),
        options.length.code(),
        thousands(summary.messages),
        summary.model,
    );
    if summary.failed > 0 {
        out.push_str(&format!(
            "> {} of {} parts got no readable answer from the model and are missing from this summary.\n\n",
            summary.failed, summary.parts
        ));
    }
    out.push_str(&summary.body());
    out.push('\n');
    out
}

// -- the window's side ---------------------------------------------------------

/// What a range will cost, before the model is asked: the window shows it
/// while the dates are being chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Estimate {
    pub messages: usize,
    /// Roughly: the text's length over [`CHARS_PER_TOKEN`], plus each line's
    /// id, time and name. Documents are not in it — they are read only when
    /// the summary is written.
    pub tokens: usize,
    /// Parts the notes pass would cut it into; zero when it fits one call.
    pub parts: usize,
    /// Photos, whose text is read before the summary is written.
    pub photos: usize,
}

/// Gemma's tokenizer on Serbian in Latin script, by eye: three and a bit
/// characters a token. Only the estimate uses it; the cuts are counted.
const CHARS_PER_TOKEN: usize = 3;

/// What a line costs beside its text: `#18234 09:14 Name (re #18200): `.
const LINE_TOKENS: usize = 12;

impl Chat {
    pub fn estimate(&self, from: NaiveDate, to: NaiveDate, topic: Option<usize>) -> Estimate {
        let mut e = Estimate::default();
        for m in
            self.export.msgs.iter().filter(|m| {
                (from..=to).contains(&m.when.date()) && topic.is_none_or(|t| m.topic == t)
            })
        {
            e.messages += 1;
            e.photos += usize::from(m.media == "photo");
            e.tokens += m.text.chars().count().min(tga_sum::lines::TEXT_CHARS) / CHARS_PER_TOKEN
                + LINE_TOKENS;
        }
        if e.tokens > tga_sum::SINGLE_TOKENS {
            // A part is cut a little under its budget, at day boundaries.
            e.parts = e.tokens.div_ceil(tga_sum::PART_TOKENS * 9 / 10);
        }
        e
    }
}

/// A summary written earlier, on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: PathBuf,
    /// The file's name without `.md`: the range, the scope, the length and
    /// the language, as [`Options::path`] wrote them.
    pub name: String,
    pub modified: Option<std::time::SystemTime>,
}

/// Every summary saved beside an export, newest first.
pub fn saved(home: &Path) -> Vec<Saved> {
    let Ok(dir) = std::fs::read_dir(home.join("analysis").join("summaries")) else {
        return Vec::new();
    };
    let mut out: Vec<Saved> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .map(|path| Saved {
            name: path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            modified: path.metadata().and_then(|m| m.modified()).ok(),
            path,
        })
        .collect();
    out.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// Read a chat on a worker: a large database is seconds.
pub fn load(
    path: PathBuf,
    chat: Option<String>,
    wake: crate::job::Wake,
) -> Receiver<Result<Arc<Chat>, String>> {
    let (tx, rx) = channel();
    let tx = crate::job::Waking { tx, wake };
    std::thread::Builder::new()
        .name("tga-summary-read".into())
        .spawn(move || {
            let chat = Chat::open(&path, chat.as_deref())
                .map(Arc::new)
                .map_err(|e| format!("{e:#}"));
            tx.send(chat);
        })
        .expect("the OS refused a worker thread");
    rx
}

/// Where the model is, and how to start it if nothing is there.
#[derive(Debug, Clone)]
pub struct Server {
    pub port: u16,
    /// `None`: use only a server that is already running.
    pub launch: Option<Launch>,
    /// Whether to read the text in the range's photos first.
    pub photos: bool,
    /// The OCR to read them with, on the same port: `None` when its files
    /// were not found beside the model.
    pub ocr: Option<Launch>,
}

/// What a summary run tells the window.
#[derive(Debug, Clone)]
pub enum Progress {
    /// What is being waited for: the server starting, the parts being read.
    /// Never a message's text.
    Step(String),
    Event(Event),
    Done(Result<Finished, String>),
}

/// A summary written and saved.
#[derive(Debug, Clone)]
pub struct Finished {
    pub path: PathBuf,
    /// The saved file's whole text, header included.
    pub document: String,
    pub summary: Summary,
    /// Photos in the range whose text could not be read, and so is not in
    /// the summary: no OCR, or another program's model holding the card.
    pub unread_photos: usize,
}

/// A summary being written: its events, and the switch that stops it.
pub struct Job {
    pub events: Receiver<Progress>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// Stop now, mid-answer if need be: a summary half-written is not kept,
    /// and the notes of every finished part already are.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Write a summary on a worker. The server is started on it too, if it has to
/// be, so the window never waits for a model to load.
pub fn spawn(
    chat: Arc<Chat>,
    options: Options,
    server: Server,
    keeper: Arc<Keeper>,
    wake: crate::job::Wake,
) -> Job {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let tx = crate::job::Waking { tx, wake };
    std::thread::Builder::new()
        .name("tga-summary".into())
        .spawn(move || {
            let done = write(&chat, &options, &server, &keeper, &flag, &tx);
            keeper.touch();
            tx.send(Progress::Done(done));
        })
        .expect("the OS refused a worker thread");
    Job { events: rx, cancel }
}

fn write(
    chat: &Chat,
    options: &Options,
    server: &Server,
    keeper: &Keeper,
    cancel: &AtomicBool,
    tx: &crate::job::Waking<Progress>,
) -> Result<Finished, String> {
    let client = Client::new(server.port);
    let mut rows = chat.rows(options.from, options.to, options.topic);
    if rows.is_empty() {
        return Err(format!(
            "Nothing was said between {} and {}.",
            options.from, options.to
        ));
    }
    // The photos before Gemma: the card holds one model at a time, and the
    // OCR is the small one to load and unload.
    let unread_photos = if server.photos {
        read_photos(chat, &mut rows, server, keeper, cancel, tx)?
    } else {
        0
    };
    keeper
        .ensure(
            &client,
            server.launch.as_ref(),
            Want::Gemma,
            cancel,
            &mut |s| tx.send(Progress::Step(s)),
        )
        .map_err(|e| match e {
            tga_sum::Error::NoServer(port) if server.launch.is_none() => format!(
                "Nothing is serving on 127.0.0.1:{port}, and there is no llama-server and \
                 model to start: the models folder lacks one or the other. Settings › \
                 Summaries › How to add a model says what goes there."
            ),
            e => e.to_string(),
        })?;
    let topics = chat.topics();
    let request = Request {
        chat: chat.title(),
        rows: &rows,
        topics: &topics,
        forum: chat.forum(),
        by_topic: options.by_topic && options.topic.is_none(),
        length: options.length,
        lang: options.lang,
        from: options.from,
        to: options.to,
    };
    let cache = Cache::at(&chat.home.join("analysis").join("summaries").join("cache"));
    let summary = tga_sum::summarise(&client, &request, &cache, cancel, &mut |e| {
        keeper.touch();
        tx.send(Progress::Event(e));
    })
    .map_err(|e| e.to_string())?;

    let path = options.path(&chat.home, chat);
    let written = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let document = document(chat, options, &summary, &written);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, &document).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Finished {
        path,
        document,
        summary,
        unread_photos,
    })
}

/// What a question tells the window.
#[derive(Debug, Clone)]
pub enum Reply {
    /// What is being waited for. Never a message's text.
    Step(String),
    Event(Event),
    Done(Result<Answered, String>),
}

/// A question answered.
#[derive(Debug, Clone)]
pub struct Answered {
    pub answer: String,
    /// What the answer rested on, kept for the next question about the same
    /// days.
    pub grounding: Arc<Grounding>,
}

/// A question being answered: its events, and the switch that stops it.
pub struct Asking {
    pub events: Receiver<Reply>,
    cancel: Arc<AtomicBool>,
}

impl Asking {
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Answer `question` about the days `options` names, on a worker. The days
/// are read first unless `grounding` already holds them; photos read before
/// are in it, and none are read for a question.
#[allow(clippy::too_many_arguments)]
pub fn ask(
    chat: Arc<Chat>,
    options: Options,
    server: Server,
    keeper: Arc<Keeper>,
    grounding: Option<Arc<Grounding>>,
    history: Vec<Turn>,
    question: String,
    wake: crate::job::Wake,
) -> Asking {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let tx = crate::job::Waking { tx, wake };
    std::thread::Builder::new()
        .name("tga-summary-ask".into())
        .spawn(move || {
            let done = answer(
                &chat, &options, &server, &keeper, grounding, &history, &question, &flag, &tx,
            );
            keeper.touch();
            tx.send(Reply::Done(done));
        })
        .expect("the OS refused a worker thread");
    Asking { events: rx, cancel }
}

#[allow(clippy::too_many_arguments)]
fn answer(
    chat: &Chat,
    options: &Options,
    server: &Server,
    keeper: &Keeper,
    grounding: Option<Arc<Grounding>>,
    history: &[Turn],
    question: &str,
    cancel: &AtomicBool,
    tx: &crate::job::Waking<Reply>,
) -> Result<Answered, String> {
    let client = Client::new(server.port);
    keeper
        .ensure(
            &client,
            server.launch.as_ref(),
            Want::Gemma,
            cancel,
            &mut |s| tx.send(Reply::Step(s)),
        )
        .map_err(|e| e.to_string())?;
    let topics = chat.topics();
    let mut rows = chat.rows(options.from, options.to, options.topic);
    if let (None, Some(ocr)) = (&grounding, &server.ocr) {
        let pictures = chat.pictures(&rows);
        kept_readings(
            &readings_at(&chat.home),
            &file_name(&ocr.model),
            &mut rows,
            pictures,
        );
    }
    let request = Request {
        chat: chat.title(),
        rows: &rows,
        topics: &topics,
        forum: chat.forum(),
        by_topic: false,
        length: options.length,
        lang: options.lang,
        from: options.from,
        to: options.to,
    };
    let grounding = match grounding {
        Some(g) => g,
        None => {
            if rows.is_empty() {
                return Err(format!(
                    "Nothing was said between {} and {}.",
                    options.from, options.to
                ));
            }
            tx.send(Reply::Step("Reading the days".into()));
            let cache = Cache::at(&chat.home.join("analysis").join("summaries").join("cache"));
            let g = tga_sum::ground(&client, &request, &cache, cancel, &mut |e| {
                keeper.touch();
                match e {
                    Event::Say(s) => tx.send(Reply::Step(s)),
                    Event::Part { done, of, .. } => tx.send(Reply::Step(format!(
                        "Reading the days: part {done} of {of}"
                    ))),
                    _ => {}
                }
            })
            .map_err(|e| e.to_string())?;
            Arc::new(g)
        }
    };
    tx.send(Reply::Step("Answering".into()));
    let answer = tga_sum::answer(
        &client,
        &request,
        &grounding,
        history,
        question,
        cancel,
        &mut |e| {
            keeper.touch();
            tx.send(Reply::Event(e));
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(Answered { answer, grounding })
}

/// Put the text of the range's photos into their rows: what was read before
/// at once, the rest by the OCR, started for it if need be. Photos are an
/// extra, so an OCR that cannot be had is said and the summary goes on
/// without them; only a stop ends the run. How many were left unread.
fn read_photos(
    chat: &Chat,
    rows: &mut [tga_notes::Row],
    server: &Server,
    keeper: &Keeper,
    cancel: &AtomicBool,
    tx: &crate::job::Waking<Progress>,
) -> Result<usize, String> {
    let say = |s: String| tx.send(Progress::Step(s));
    let pictures = chat.pictures(rows);
    if pictures.is_empty() {
        return Ok(0);
    }
    let count = pictures.len();
    let Some(ocr) = &server.ocr else {
        say(format!(
            "{count} photos not read: no PaddleOCR-VL beside the model"
        ));
        return Ok(count);
    };
    let mut readings = readings_at(&chat.home);
    let todo = kept_readings(&readings, &file_name(&ocr.model), rows, pictures);
    if todo.is_empty() {
        return Ok(0);
    }
    let client = Client::new(server.port);
    // Another program's server — a Gemma, usually — is never stopped
    // from here; the view offers that, as a press.
    if !keeper.running() && client.healthy() {
        if let Ok(props) = client.props() {
            if !tga_sum::ocr::is_ocr(&props.model) {
                say(format!(
                    "{} photos not read: another program's model server has {} loaded \
                     and holds the card. Stop it in the Summaries view to read them.",
                    todo.len(),
                    props.model
                ));
                return Ok(todo.len());
            }
        }
    }
    if let Err(e) = keeper.ensure(&client, Some(ocr), Want::Ocr, cancel, &mut |s| say(s)) {
        if matches!(e, tga_sum::Error::Cancelled) {
            return Err(e.to_string());
        }
        say(format!("{} photos not read: {e}", todo.len()));
        return Ok(todo.len());
    }
    let started = Instant::now();
    let (mut done, mut with_text, mut unread) = (0, 0, 0);
    for batch in todo.chunks(PHOTO_BATCH) {
        let photos: Vec<Photo> = batch
            .iter()
            .filter_map(|(_, key, picture)| {
                Some(Photo {
                    key: key.clone(),
                    bytes: chat.bytes(picture)?,
                })
            })
            .collect();
        let read = tga_sum::ocr::read_all(&client, photos, cancel).map_err(|e| e.to_string())?;
        for (row, key, _) in batch {
            match read.get(key) {
                Some(text) => {
                    rows[*row].photo_text = text.clone();
                    with_text += usize::from(!text.is_empty());
                }
                None => unread += 1,
            }
        }
        readings.extend(read);
        readings.save();
        keeper.touch();
        done += batch.len();
        say(format!("Reading photos: {done} of {}", todo.len()));
    }
    say(format!(
        "Read {done} photos in {:.0} s, {with_text} with text; {} read before",
        started.elapsed().as_secs_f64(),
        count - todo.len()
    ));
    Ok(unread)
}

/// Run as a command. `args` is what followed `summarise`.
/// The `summarise` command line. `models` is Settings › Summaries' models
/// folder, where the OCR's file name — which keys the photos read before —
/// comes from.
pub fn run(args: Vec<String>, models: &Path) -> Result<()> {
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print!("{USAGE}");
        return Ok(());
    }
    let path = PathBuf::from(&args[0]);
    if args[0].starts_with("--") {
        bail!("Name the database or export folder first.\n\n{USAGE}");
    }
    let mut from: Option<NaiveDate> = None;
    let mut to: Option<NaiveDate> = None;
    let mut length = Length::Short;
    let mut lang = Lang::Serbian;
    let mut by_topic = false;
    let mut topic: Option<String> = None;
    let mut chat: Option<String> = None;
    let mut port: u16 = 8080;
    let mut out: Option<PathBuf> = None;
    let mut cache = true;
    let mut any_model = false;

    let day = |v: Option<&String>, flag: &str| -> Result<NaiveDate> {
        let v = v.with_context(|| format!("{flag} needs a day, YYYY-MM-DD"))?;
        NaiveDate::parse_from_str(v, "%Y-%m-%d")
            .with_context(|| format!("{flag} {v}: not a day, YYYY-MM-DD"))
    };
    let mut rest = args[1..].iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--from" => from = Some(day(rest.next(), "--from")?),
            "--to" => to = Some(day(rest.next(), "--to")?),
            "--short" => length = Length::Short,
            "--detailed" => length = Length::Detailed,
            "--sections" => length = Length::Sections,
            "--by-topic" => by_topic = true,
            "--topic" => topic = Some(rest.next().context("--topic needs a name")?.clone()),
            "--lang" => {
                let v = rest.next().context("--lang needs sr, en or chat")?;
                lang = Lang::parse(v).with_context(|| format!("--lang {v}: sr, en or chat"))?;
            }
            "--chat" => {
                chat = Some(
                    rest.next()
                        .context("--chat needs a chat id or title")?
                        .clone(),
                )
            }
            "--port" => {
                let v = rest.next().context("--port needs a number")?;
                port = v
                    .parse()
                    .with_context(|| format!("--port {v}: not a port"))?;
            }
            "--out" => out = Some(PathBuf::from(rest.next().context("--out needs a path")?)),
            "--no-cache" => cache = false,
            "--any-model" => any_model = true,
            other => bail!("Unknown option: {other}\n\n{USAGE}"),
        }
    }
    let Some(from) = from else {
        bail!("--from is needed: the first day to summarise");
    };
    let to = to.unwrap_or(from);
    if to < from {
        bail!("--to {to} is before --from {from}");
    }
    if by_topic && topic.is_some() {
        bail!("--by-topic and --topic: one or the other");
    }

    // The server first: reading a large chat takes seconds, and finding out
    // after that there is nothing to talk to is a poor way to spend them.
    let client = Client::new(port);
    if !client.healthy() {
        bail!(
            "No model server is answering on 127.0.0.1:{port}. Start Gemma first \
             (the window's Summaries view does), or pass --port."
        );
    }
    let props = client.props()?;
    if !any_model && !props.model.to_lowercase().contains("gemma") {
        bail!(
            "The server on 127.0.0.1:{port} has {} loaded, not Gemma. The prompts and sampling \
             are tuned for Gemma; pass --any-model to use it anyway.",
            if props.model.is_empty() {
                "an unnamed model"
            } else {
                &props.model
            }
        );
    }

    let started = Instant::now();
    let chat = Chat::open(&path, chat.as_deref())?;
    let topic = match &topic {
        None => None,
        Some(_) if !chat.forum() => bail!("--topic: {} is not a forum", chat.title()),
        Some(name) => Some(chat.topic(name).with_context(|| {
            format!(
                "no topic {name:?}; the topics are: {}",
                chat.topics().join(", ")
            )
        })?),
    };
    if by_topic && !chat.forum() {
        bail!(
            "--by-topic: {} is not a forum, so it has no topics to split by",
            chat.title()
        );
    }
    let mut rows = chat.rows(from, to, topic);
    eprintln!(
        "  read {} messages in {:.1} s; {} in the range",
        thousands(chat.export.msgs.len()),
        started.elapsed().as_secs_f64(),
        thousands(rows.len())
    );
    // Photos the window has read before; the command line starts no OCR,
    // since the one server holds Gemma.
    let pictures = chat.pictures(&rows);
    let ocr = tga_sum::ocr::files_in(models);
    if let (false, Some((model, ..))) = (pictures.is_empty(), ocr) {
        let count = pictures.len();
        let left = kept_readings(
            &readings_at(&chat.home),
            &file_name(&model),
            &mut rows,
            pictures,
        );
        eprintln!(
            "  {count} photos, {} read before; the window's Summaries view reads the rest",
            count - left.len()
        );
    }
    if rows.is_empty() {
        let span = chat
            .span()
            .map(|(a, b)| format!(" The chat runs from {a} to {b}."))
            .unwrap_or_default();
        bail!("Nothing was said between {from} and {to}.{span}");
    }

    let options = Options {
        from,
        to,
        length,
        lang,
        by_topic,
        topic,
    };
    let topics = chat.topics();
    let request = Request {
        chat: chat.title(),
        rows: &rows,
        topics: &topics,
        forum: chat.forum(),
        by_topic,
        length,
        lang,
        from,
        to,
    };
    let cache = if cache {
        Cache::at(&chat.home.join("analysis").join("summaries").join("cache"))
    } else {
        Cache::none()
    };
    let never = AtomicBool::new(false);
    let mut stdout = std::io::stdout();
    eprintln!(
        "  model {} on 127.0.0.1:{port}, {} tokens of context",
        props.model, props.n_ctx
    );
    let summary = tga_sum::summarise(
        &client,
        &request,
        &cache,
        &never,
        &mut |event| match event {
            Event::Say(s) => eprintln!("  {s}"),
            Event::Part {
                done,
                of,
                cached: true,
                ..
            } => eprintln!("  part {done}/{of}  kept from an earlier run"),
            Event::Part {
                done,
                of,
                secs,
                timings,
                ..
            } => {
                let t = timings.unwrap_or_default();
                eprintln!(
                    "  part {done}/{of}  {secs:.1} s  {} tokens in at {:.0}/s, {} out at {:.1}/s",
                    t.prompt_tokens, t.prefill_tps, t.gen_tokens, t.gen_tps
                );
            }
            Event::Section(Some(name)) => {
                let _ = write!(stdout, "\n## {name}\n\n");
            }
            Event::Section(None) => {
                let _ = writeln!(stdout);
            }
            Event::Text(piece) => {
                let _ = write!(stdout, "{piece}");
                let _ = stdout.flush();
            }
            Event::Written(t) => {
                let _ = writeln!(stdout);
                eprintln!(
                    "  written: {} tokens in at {:.0}/s, {} out at {:.1}/s",
                    t.prompt_tokens, t.prefill_tps, t.gen_tokens, t.gen_tps
                );
            }
        },
    )?;

    let path = out.unwrap_or_else(|| options.path(&chat.home, &chat));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let written = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    std::fs::write(&path, document(&chat, &options, &summary, &written))
        .with_context(|| format!("writing {}", path.display()))?;
    if summary.failed > 0 {
        eprintln!(
            "  warning: {} of {} parts got no readable answer and are missing from the summary",
            summary.failed, summary.parts
        );
    }
    eprintln!(
        "  {:.0} s in all; saved {}",
        started.elapsed().as_secs_f64(),
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed synthetic export, which names nobody, in a folder of its
    /// own.
    fn synthetic(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tga-run-summary-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tga-report/tests/synthetic/result.json");
        std::fs::copy(fixture, dir.join("result.json")).unwrap();
        dir
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_range_is_calendar_days_both_inclusive_by_the_wall_clock() {
        let dir = synthetic("range");
        let chat = Chat::open(&dir, None).unwrap();
        let ids = |a, b| {
            chat.rows(day(a), day(b), None)
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("2025-01-02", "2025-01-02"), vec![1, 2]);
        // 23:40 local is the 4th, whatever day it is in UTC.
        assert_eq!(ids("2025-01-04", "2025-01-04"), vec![3]);
        assert_eq!(ids("2025-01-03", "2025-01-03"), Vec::<i64>::new());
        assert_eq!(ids("2025-01-01", "2025-01-31"), vec![1, 2, 3]);
        assert_eq!(chat.span(), Some((day("2025-01-02"), day("2025-01-04"))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_row_keeps_its_whole_text_its_reply_and_its_sender() {
        let dir = synthetic("row");
        let chat = Chat::open(&dir, None).unwrap();
        let rows = chat.rows(day("2025-01-02"), day("2025-01-02"), None);
        assert_eq!(rows[1].who, "Bob");
        assert_eq!(rows[1].re, Some(1));
        assert_eq!(rows[1].text, "ok https://example.com #kolab");
        assert!(!chat.forum());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_file_name_says_range_scope_length_and_language() {
        let dir = synthetic("name");
        let chat = Chat::open(&dir, None).unwrap();
        let mut o = Options {
            from: day("2025-01-02"),
            to: day("2025-01-02"),
            length: Length::Detailed,
            lang: Lang::English,
            by_topic: false,
            topic: None,
        };
        let home = Path::new("H");
        let name = |o: &Options| {
            o.path(home, &chat)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(name(&o), "2025-01-02_detailed_en.md");
        o.to = day("2025-01-04");
        o.by_topic = true;
        assert_eq!(name(&o), "2025-01-02_2025-01-04_by-topic_detailed_en.md");
        assert_eq!(
            o.path(home, &chat).parent().unwrap(),
            Path::new("H").join("analysis").join("summaries")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_photo_on_disk_is_found_and_an_earlier_reading_fills_its_line() {
        let dir = synthetic("photo");
        let chat = Chat::open(&dir, None).unwrap();
        let mut rows = chat.rows(day("2025-01-04"), day("2025-01-04"), None);
        // Named in the JSON, never written: nothing to read.
        assert!(chat.pictures(&rows).is_empty());

        std::fs::create_dir_all(dir.join("photos")).unwrap();
        std::fs::write(dir.join("photos").join("x.jpg"), [0xff, 0xd8, 0xff, 0xe0]).unwrap();
        let pictures = chat.pictures(&rows);
        assert_eq!(pictures.len(), 1);
        assert_eq!(pictures[0].1, "file:photos/x.jpg:4");
        assert_eq!(chat.bytes(&pictures[0].2).unwrap().len(), 4);

        let mut readings = readings_at(&chat.home);
        let model = "PaddleOCR-VL-1.5.gguf";
        let key = Readings::key(model, &pictures[0].1);
        readings.extend([(key, "Protest u petak".to_string())].into());
        let found = chat.pictures(&rows);
        assert!(kept_readings(&readings, model, &mut rows, found).is_empty());
        assert_eq!(rows[0].photo_text, "Protest u petak");
        // Another OCR has read nothing yet.
        let found = chat.pictures(&rows);
        assert_eq!(
            kept_readings(&readings, "other.gguf", &mut rows, found).len(),
            1
        );
        assert_eq!(
            chat.estimate(day("2025-01-04"), day("2025-01-04"), None)
                .photos,
            1
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_topic_name_becomes_a_safe_filename() {
        assert_eq!(
            slug("Mediji / PR & Društvene mreže"),
            "mediji-pr-društvene-mreže"
        );
        assert_eq!(slug("///"), "topic");
    }
}
