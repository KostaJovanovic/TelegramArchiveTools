//! The Summaries view: what happened in a chat between two days, in the local
//! model's words.
//!
//! **Chats down the left, the summary on the right.** The chats are the
//! Library's — the same folder, the same rows — and under them the summaries
//! already saved for the one picked. On the right: the days, the length, the
//! scope and the language, a line saying what that range will cost before
//! anything is asked of the model, then the summary as it is written.
//!
//! **The model is on this machine.** The worker (`tga_run::summary`) uses a
//! Gemma already serving on the port, or starts the `llama-server.exe` and
//! the model in the models folder Settings › Summaries names
//! ([`launch_files`]); [`Keeper`] holds what it started, and the shell stops
//! it after the idle minutes and when the window closes.
//!
//! **A citation opens the Viewer on that message.** `[#18234]` in a summary of
//! a database is a link; the Viewer's way back comes here.
//!
//! The text on screen is other people's conversation, retold. **None of it
//! goes to the log**: the status line names counts and steps.

use super::*;
use crate::holder::{self, Holder};
use eframe::egui::{Align, Layout, Sense, Ui};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use tga_run::state::Entry;
use tga_run::summary::{
    self as run, Asking, Chat, Estimate, Job, Options, Progress, Reply, Saved, Server,
};
use tga_run::tga_sum::{server, Event, Grounding, Keeper, Lang, Launch, Length, Timings, Turn};
use tgx_ui::components::{
    action, block, button, eyebrow, field, figure, meta, row, rule, segmented, text, EmptyState,
    GUTTER,
};
use tgx_ui::tokens::space;

/// The server and the model a summary starts, from Settings › Summaries: the
/// models folder's `llama-server.exe` unless another is named, and the model
/// chosen there.
pub(super) fn launch_files(
    settings: &tgx_tg::config::Settings,
) -> (Option<PathBuf>, Option<PathBuf>) {
    let folder = settings.summary_models_dir();
    let exe = if settings.summary_server.is_empty() {
        server::server_in(&folder)
    } else {
        Some(PathBuf::from(&settings.summary_server))
    };
    (exe, chosen_model(settings, &server::models_in(&folder)))
}

/// The model Settings › Summaries chose, while its file is there; else the
/// folder's first Gemma, else its first model. `models` is the folder's.
pub(super) fn chosen_model(
    settings: &tgx_tg::config::Settings,
    models: &[PathBuf],
) -> Option<PathBuf> {
    let chosen = settings.summary_models_dir().join(&settings.summary_model);
    if !settings.summary_model.is_empty() && chosen.is_file() {
        Some(chosen)
    } else {
        server::default_model(models)
    }
}

/// How often the port is looked at while the view is open and idle: often
/// enough that a server another program starts or stops shows within a few
/// seconds.
const LOOK_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// The chat list. Wide enough for a chat's title and its count, and narrow
/// enough that at the window's 900pt floor the form keeps the larger half.
const LIST_W: f32 = 240.0;

/// A chat row in the list.
const ROW_HEIGHT: f32 = 30.0;

/// A date field: `2026-01-31` and a little air.
const DATE_W: f32 = 104.0;

/// What a summary covers: the whole group, each topic apart, or one topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub(super) enum Scope {
    #[default]
    Whole,
    ByTopic,
    One(usize),
}

/// The chat picked on the left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Picked {
    /// The database file or the export folder.
    pub(super) path: PathBuf,
    /// A database's chat; `None` for a folder export, which the Viewer cannot
    /// open, so its citations are not links.
    pub(super) chat_id: Option<i64>,
    pub(super) title: String,
    /// Where its report, digest and summaries live.
    pub(super) home: PathBuf,
}

/// Everything the Summaries view remembers between frames.
pub(super) struct SummariesState {
    /// The model server this window started, if it did. Shared with the
    /// worker that starts it.
    pub(super) keeper: Arc<Keeper>,
    filter: String,
    pub(super) picked: Option<Picked>,
    chat: Option<Arc<Chat>>,
    reading: Option<Receiver<Result<Arc<Chat>, String>>>,
    pub(super) from: String,
    pub(super) to: String,
    pub(super) length: Length,
    pub(super) lang: Lang,
    pub(super) scope: Scope,
    /// The estimate, and the inputs it is of — recounted when they change.
    estimate: Option<((String, String, Scope), Estimate)>,
    job: Option<Job>,
    pub(super) status: String,
    part: Option<(usize, usize)>,
    speed: Option<Timings>,
    /// What the reading pane shows: the summary being written, or a saved one.
    pub(super) text: String,
    /// The file `text` is, once it is one.
    shown: Option<PathBuf>,
    saved: Vec<Saved>,
    /// Ctrl+F: the chat filter takes the focus when next drawn.
    pub(super) focus_filter: bool,
    /// A citation clicked this frame. The shell takes it and opens the Viewer.
    pub(super) cite: Option<(super::library::Read, i64)>,
    /// Somebody else's process on the model server's port — another
    /// program's llama-server, usually — as last looked; `None` when the port is free
    /// or the server on it is this window's.
    pub(super) holder: Option<Holder>,
    /// A look at the port in flight, and when the last one was asked for.
    looking: Option<Receiver<Option<Holder>>>,
    looked: Option<std::time::Instant>,
    /// Stop was pressed once; the second press stops it.
    stop_armed: bool,
    stopping: Option<(Holder, Receiver<Result<(), String>>)>,
    /// The conversation about the form's days, under the summary.
    pub(super) turns: Vec<Turn>,
    /// The question being typed.
    pub(super) question: String,
    asking: Option<Asking>,
    /// What the conversation rests on, and the days it was read for: a
    /// question about other days starts a new conversation.
    grounding: Option<(AskKey, Arc<Grounding>)>,
    /// The days the question in flight is about.
    ask_key: Option<AskKey>,
    /// What Settings › Summaries found in the models folder — the folder,
    /// when it looked, the models and the `llama-server.exe` — so a frame
    /// does not read a directory on what may be a slow disk. See
    /// [`SummariesState::look_for_models`].
    pub(super) found: Option<Found>,
    /// A look at the models folder in flight.
    looking_models: Option<Receiver<Found>>,
}

/// The models folder, as last looked at.
pub(super) struct Found {
    pub(super) folder: PathBuf,
    pub(super) at: std::time::Instant,
    pub(super) models: Vec<PathBuf>,
    pub(super) server: Option<PathBuf>,
}

impl Found {
    /// How long a look stands: a model copied in shows within this.
    pub(super) const FOR: std::time::Duration = std::time::Duration::from_secs(5);

    pub(super) fn look(folder: PathBuf) -> Found {
        Found {
            models: server::models_in(&folder),
            server: server::server_in(&folder),
            folder,
            at: std::time::Instant::now(),
        }
    }
}

/// The chat, the days and the scope a conversation is about.
type AskKey = (PathBuf, String, String, Scope);

impl Default for SummariesState {
    fn default() -> Self {
        SummariesState {
            keeper: Arc::new(Keeper::new()),
            filter: String::new(),
            picked: None,
            chat: None,
            reading: None,
            from: String::new(),
            to: String::new(),
            length: Length::Short,
            lang: Lang::Serbian,
            scope: Scope::Whole,
            estimate: None,
            job: None,
            status: String::new(),
            part: None,
            speed: None,
            text: String::new(),
            shown: None,
            saved: Vec::new(),
            focus_filter: false,
            cite: None,
            holder: None,
            looking: None,
            looked: None,
            stop_armed: false,
            stopping: None,
            turns: Vec::new(),
            question: String::new(),
            asking: None,
            grounding: None,
            ask_key: None,
            found: None,
            looking_models: None,
        }
    }
}

impl SummariesState {
    pub(super) fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Anything still to arrive — what a photograph of the window waits for.
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
            || self.reading.is_some()
            || self.asking.is_some()
            || self.looking_models.is_some()
    }

    /// A question is being answered.
    pub(super) fn asking(&self) -> bool {
        self.asking.is_some()
    }

    /// The model is in use: a summary being written or a question answered.
    /// One at a time — the server has one slot.
    pub(super) fn using_model(&self) -> bool {
        self.job.is_some() || self.asking.is_some()
    }

    /// The days a question asked now would be about.
    fn ask_key(&self) -> Option<AskKey> {
        Some((
            self.picked.as_ref()?.path.clone(),
            self.from.trim().to_string(),
            self.to.trim().to_string(),
            self.scope,
        ))
    }

    /// What a question about `key`'s days rests on: the conversation's own
    /// reading while it is about the same days. **Other days than the
    /// conversation so far start a new one**, its turns going with the old
    /// reading — or the history would be about messages the model is no
    /// longer given.
    fn grounding_for(&mut self, key: &AskKey) -> Option<Arc<Grounding>> {
        match &self.grounding {
            Some((k, g)) if k == key => Some(g.clone()),
            _ => {
                if self.grounding.is_some() {
                    self.turns.clear();
                }
                self.grounding = None;
                None
            }
        }
    }

    /// The options the form says, or why they cannot be.
    pub(super) fn options(&self) -> Result<Options, String> {
        let day = |s: &str, which: &str| {
            chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
                .map_err(|_| format!("{which} is not a day like 2026-01-31"))
        };
        let from = day(&self.from, "From")?;
        let to = if self.to.trim().is_empty() {
            from
        } else {
            day(&self.to, "To")?
        };
        if to < from {
            return Err("To is before From".into());
        }
        let forum = self.chat.as_ref().is_some_and(|c| c.forum());
        let (by_topic, topic) = match self.scope {
            Scope::ByTopic if forum => (true, None),
            Scope::One(t) if forum => (false, Some(t)),
            _ => (false, None),
        };
        Ok(Options {
            from,
            to,
            length: self.length,
            lang: self.lang,
            by_topic,
            topic,
        })
    }

    /// The range ending on the chat's last day, `days` long.
    fn last(&mut self, days: i64) {
        let Some((first, last)) = self.chat.as_ref().and_then(|c| c.span()) else {
            return;
        };
        let from = (last - chrono::Duration::days(days - 1)).max(first);
        self.from = from.to_string();
        self.to = last.to_string();
    }

    /// The estimate for the form as it stands, recounted only when it changed.
    fn estimate(&mut self) -> Option<Estimate> {
        let chat = self.chat.clone()?;
        let options = self.options().ok()?;
        let key = (self.from.clone(), self.to.clone(), self.scope);
        if self.estimate.as_ref().map(|(k, _)| k) != Some(&key) {
            let e = chat.estimate(options.from, options.to, options.topic);
            self.estimate = Some((key, e));
            // A summary of exactly these options already on disk is shown at
            // once — unless one is being written, which is what is shown.
            if self.job.is_none() {
                if let Some(home) = self.picked.as_ref().map(|p| p.home.clone()) {
                    let path = options.path(&home, &chat);
                    if path.is_file() && self.shown.as_ref() != Some(&path) {
                        self.show_file(&path);
                    }
                }
            }
        }
        self.estimate.as_ref().map(|(_, e)| *e)
    }

    /// Read `picked` on a worker, and list its saved summaries. Nothing while
    /// the model is in use: what it is writing is about the chat picked now.
    ///
    /// **The same chat again is read again unless it is read or being
    /// read.** A failed read leaves it picked with nothing to show, and the
    /// click that should have retried it was taken for a click on the chat
    /// already open — the only way to try again was to pick another and come
    /// back.
    fn pick(&mut self, picked: Picked, wake: tga_run::job::Wake) {
        if self.using_model() {
            return;
        }
        if self.picked.as_ref() == Some(&picked) && (self.chat.is_some() || self.reading.is_some())
        {
            return;
        }
        self.saved = run::saved(&picked.home);
        self.reading = Some(run::load(
            picked.path.clone(),
            picked.chat_id.map(|id| id.to_string()),
            wake,
        ));
        self.status = format!("Reading {}", picked.title);
        self.picked = Some(picked);
        self.chat = None;
        self.estimate = None;
        self.text.clear();
        self.shown = None;
        self.turns.clear();
        self.grounding = None;
    }

    /// Keep `found` current for `folder`: take in a look that has finished,
    /// and start another on a worker once the last is older than
    /// [`Found::FOR`] or of another folder. What was found stays on screen
    /// while the next look is in flight.
    ///
    /// **Never on the frame.** Models are many gigabytes and often on a
    /// second drive; a spun-down or network one held the whole window for the
    /// scan every five seconds while Settings was open.
    pub(super) fn look_for_models(&mut self, folder: &std::path::Path, ctx: &egui::Context) {
        if let Some(rx) = &self.looking_models {
            match rx.try_recv() {
                Ok(found) => {
                    self.looking_models = None;
                    self.found = Some(found);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.looking_models = None,
            }
        }
        let stale = self
            .found
            .as_ref()
            .is_none_or(|f| f.folder != folder || f.at.elapsed() > Found::FOR);
        if !stale || self.looking_models.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ctx.clone();
        let folder = folder.to_path_buf();
        std::thread::Builder::new()
            .name("tt-summary-models".into())
            .spawn(move || {
                let _ = tx.send(Found::look(folder));
                wake.request_repaint();
            })
            .expect("the OS refused a worker thread");
        self.looking_models = Some(rx);
    }

    fn show_file(&mut self, path: &std::path::Path) {
        match std::fs::read_to_string(path) {
            Ok(t) => {
                self.text = t;
                self.shown = Some(path.to_path_buf());
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    /// Apply what the workers have said since the last frame.
    pub(super) fn drain(&mut self) {
        if let Some(rx) = &self.reading {
            match rx.try_recv() {
                Ok(Ok(chat)) => {
                    self.reading = None;
                    let n = chat.export.msgs.len();
                    self.chat = Some(chat);
                    self.estimate = None;
                    if self.options().is_err() {
                        self.last(7);
                    }
                    if let Scope::One(t) = self.scope {
                        if t >= self.chat.as_ref().map_or(0, |c| c.topics().len()) {
                            self.scope = Scope::Whole;
                        }
                    }
                    self.status = format!("{} messages", tgx_ui::thousands(n as i64));
                }
                Ok(Err(e)) => {
                    self.reading = None;
                    self.status = e;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.reading = None,
            }
        }
        if let Some(asking) = &self.asking {
            let batch: Vec<Reply> = asking.events.try_iter().collect();
            for r in batch {
                self.reply(r);
            }
        }
        let Some(job) = &self.job else { return };
        let batch: Vec<Progress> = job.events.try_iter().collect();
        for p in batch {
            self.apply(p);
        }
    }

    fn reply(&mut self, reply: Reply) {
        match reply {
            Reply::Step(s) | Reply::Event(Event::Say(s)) => {
                // Counts and steps: never a message, never the question.
                log::info!("summary chat: {s}");
                self.status = s;
            }
            Reply::Event(Event::Text(piece)) => {
                if let Some(turn) = self.turns.last_mut() {
                    turn.answer.push_str(&piece);
                }
            }
            Reply::Event(Event::Written(t)) => self.speed = Some(t),
            Reply::Event(_) => {}
            Reply::Done(done) => {
                self.asking = None;
                let key = self.ask_key.take();
                match done {
                    Ok(a) => {
                        if let Some(turn) = self.turns.last_mut() {
                            turn.answer = a.answer;
                        }
                        if let Some(key) = key {
                            self.grounding = Some((key, a.grounding));
                        }
                        self.status = "Answered".into();
                    }
                    Err(e) => {
                        let e = if e == "cancelled" {
                            "Stopped".into()
                        } else {
                            e
                        };
                        if let Some(turn) = self.turns.last_mut() {
                            if turn.answer.trim().is_empty() {
                                turn.answer = format!("*No answer: {e}*");
                            }
                        }
                        self.status = e;
                    }
                }
            }
        }
    }

    fn apply(&mut self, progress: Progress) {
        match progress {
            Progress::Step(s) => {
                // Counts, models and timings: never a message.
                log::info!("summary: {s}");
                self.status = s;
            }
            Progress::Event(e) => match e {
                Event::Say(s) => self.status = s,
                Event::Part {
                    done,
                    of,
                    cached,
                    timings,
                    ..
                } => {
                    self.part = Some((done, of));
                    if let Some(t) = timings {
                        self.speed = Some(t);
                    }
                    self.status = if cached {
                        format!("Part {done} of {of}: kept from an earlier run")
                    } else {
                        format!("Read part {done} of {of}")
                    };
                }
                Event::Section(Some(name)) => {
                    self.text.push_str(&format!("\n\n## {name}\n\n"));
                }
                Event::Section(None) => self.text.push_str("\n\n"),
                Event::Text(piece) => {
                    if self.text.is_empty() || self.status.starts_with("Read part") {
                        self.status = "Writing".into();
                    }
                    self.text.push_str(&piece);
                }
                Event::Written(t) => self.speed = Some(t),
            },
            Progress::Done(done) => {
                self.job = None;
                self.part = None;
                match done {
                    Ok(f) => {
                        self.text = f.document;
                        self.status = format!(
                            "Saved as {}",
                            f.path.file_name().unwrap_or_default().to_string_lossy()
                        );
                        if f.summary.failed > 0 {
                            self.status.push_str(&format!(
                                " — {} of {} parts got no answer and are missing",
                                f.summary.failed, f.summary.parts
                            ));
                        }
                        if f.summary.cut {
                            self.status
                                .push_str(" — the answer hit its length limit and stops short");
                        }
                        if f.unread_photos > 0 {
                            self.status.push_str(&format!(
                                " — the text in {} photo{} was not read",
                                f.unread_photos,
                                if f.unread_photos == 1 { "" } else { "s" }
                            ));
                        }
                        self.shown = Some(f.path);
                        if let Some(p) = &self.picked {
                            self.saved = run::saved(&p.home);
                        }
                    }
                    Err(e) if e == "cancelled" => self.status = "Stopped".into(),
                    Err(e) => self.status = e,
                }
            }
        }
    }

    pub(super) fn stop(&mut self) {
        if let Some(job) = &self.job {
            job.stop();
            self.status = "Stopping".into();
        }
        if let Some(asking) = &self.asking {
            asking.stop();
            self.status = "Stopping".into();
        }
    }
}

/// The model server as Settings › Summaries describes it: its port, how to
/// start the model, and the OCR from the same models folder.
fn summary_server(settings: &tgx_tg::config::Settings) -> Server {
    let (exe, model) = launch_files(settings);
    let launch = exe.zip(model).map(|(exe, model)| Launch {
        exe,
        model,
        mmproj: None,
        template: None,
        options: settings.summary_options.clone(),
        log: tgx_tg::logging::log_file().with_file_name("llama-server.log"),
    });
    let models = settings.summary_models_dir();
    let ocr = launch.as_ref().and_then(|l| Launch::ocr(l, &models));
    Server {
        port: settings.summary_port,
        launch,
        photos: settings.summary_photos,
        ocr,
    }
}

/// What the wake callback needs: a repaint of this window.
fn wake(ctx: &egui::Context) -> tga_run::job::Wake {
    let ctx = ctx.clone();
    Arc::new(move || ctx.request_repaint())
}

impl Shell {
    /// Pick a chat: read it on a worker, and list its saved summaries.
    fn summaries_pick(&mut self, entry: &Entry, ctx: &egui::Context) {
        let picked = Picked {
            path: entry.target.path().to_path_buf(),
            chat_id: entry.chat_id,
            title: entry.title.clone(),
            home: entry.target.home(),
        };
        self.summaries.pick(picked, wake(ctx));
    }

    /// Summarise: the form's options, the settings' server, on a worker.
    pub(super) fn summaries_start(&mut self, ctx: &egui::Context) {
        let s = &mut self.summaries;
        let (Some(chat), Ok(options)) = (s.chat.clone(), s.options()) else {
            return;
        };
        if s.using_model() {
            return;
        }
        let server = summary_server(&self.settings);
        s.text.clear();
        s.shown = None;
        s.part = None;
        s.speed = None;
        s.status = "Asking the model".into();
        log::info!(
            "summary: {} to {}, {}, {}",
            options.from,
            options.to,
            options.length.code(),
            options.lang.code()
        );
        s.job = Some(run::spawn(
            chat,
            options,
            server,
            s.keeper.clone(),
            wake(ctx),
        ));
    }

    /// Ask the question typed under the summary, about the form's days.
    fn summaries_ask(&mut self, ctx: &egui::Context) {
        let server = summary_server(&self.settings);
        let s = &mut self.summaries;
        let question = s.question.trim().to_string();
        let (Some(chat), Ok(options), Some(key)) = (s.chat.clone(), s.options(), s.ask_key())
        else {
            return;
        };
        if question.is_empty() || s.using_model() {
            return;
        }
        let grounding = s.grounding_for(&key);
        let history = s.turns.clone();
        s.turns.push(Turn {
            question: question.clone(),
            answer: String::new(),
        });
        s.question.clear();
        s.ask_key = Some(key);
        s.status = "Asking the model".into();
        // A question may quote the chat: its length goes to the log, never
        // its words.
        log::info!(
            "summary chat: a question of {} characters, {} to {}",
            question.chars().count(),
            options.from,
            options.to
        );
        s.asking = Some(run::ask(
            chat,
            options,
            server,
            s.keeper.clone(),
            grounding,
            history,
            question,
            wake(ctx),
        ));
    }

    /// The view's half of the top bar: Stop, and the one red.
    pub(super) fn summaries_actions(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let p = self.palette;
        let s = &self.summaries;
        let ready = !s.using_model()
            && s.chat.is_some()
            && s.options().is_ok()
            && s.estimate.as_ref().is_some_and(|(_, e)| e.messages > 0);
        let running = s.running();
        let mut start = false;
        let mut stop = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = if running { "Writing…" } else { "Summarise" };
            start = button(ui, label, ready, true, &p).clicked();
            if running {
                stop = button(ui, "Stop", true, false, &p).clicked();
            }
        });
        if start {
            self.summaries_start(ctx);
        }
        if stop {
            self.summaries.stop();
        }
    }

    /// The status bar's line while this view shows.
    pub(super) fn summaries_status(&self) -> (String, String) {
        let s = &self.summaries;
        let right = match (s.part, s.speed) {
            (Some((done, of)), _) if s.running() => format!("{done} of {of}"),
            (_, Some(t)) if t.gen_tps > 0.0 => format!("{:.0} tok/s", t.gen_tps),
            _ => String::new(),
        };
        (s.status.clone(), right)
    }

    /// Look at who holds the model server's port, every [`LOOK_EVERY`] while
    /// the view is open and nothing of ours is using it, and take in a stop
    /// that has finished.
    fn summaries_look(&mut self, ctx: &egui::Context) {
        let port = self.settings.summary_port;
        let s = &mut self.summaries;
        if let Some(rx) = &s.looking {
            match rx.try_recv() {
                Ok(found) => {
                    s.looking = None;
                    if found != s.holder {
                        s.stop_armed = false;
                    }
                    s.holder = found;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => s.looking = None,
            }
        }
        let mut stopped = None;
        if let Some((who, rx)) = &s.stopping {
            match rx.try_recv() {
                Ok(done) => stopped = Some((who.clone(), done)),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    stopped = Some((who.clone(), Err("it did not say".into())))
                }
            }
        }
        if let Some((who, done)) = stopped {
            s.stopping = None;
            s.looked = None;
            match done {
                Ok(()) => {
                    let line = format!("Stopped {} (process {})", who.who(), who.pid);
                    log::info!("summary: {line}");
                    s.holder = None;
                    s.status = line.clone();
                    self.journal.push(line);
                }
                Err(e) => {
                    log::warn!("summary: could not stop process {}: {e}", who.pid);
                    s.status = format!("Could not stop {}: {e}", who.who());
                }
            }
        }
        let s = &mut self.summaries;
        // The server on the port is ours, or a run is using whatever is.
        if s.keeper.running() || s.using_model() {
            s.holder = None;
            return;
        }
        let due = s.looked.is_none_or(|t| t.elapsed() >= LOOK_EVERY);
        if due && s.looking.is_none() && s.stopping.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = ctx.clone();
            std::thread::Builder::new()
                .name("tt-summary-port".into())
                .spawn(move || {
                    let _ = tx.send(holder::probe(port));
                    wake.request_repaint();
                })
                .expect("the OS refused a worker thread");
            s.looking = Some(rx);
            s.looked = Some(std::time::Instant::now());
        }
        ctx.request_repaint_after(LOOK_EVERY);
    }

    /// Somebody else's server on the port: what it is, what that means for a
    /// summary, and Stop — pressed twice, because it ends another program's
    /// process.
    fn summaries_holder(&mut self, ui: &mut Ui) {
        let Some(h) = self.summaries.holder.clone() else {
            return;
        };
        let p = self.palette;
        let port = self.settings.summary_port;
        let who = h.who();
        let what = if !h.is_llama() {
            format!("Port {port} is taken by {who}. Choose another port in Settings › Summaries.")
        } else if !h.ready {
            format!("{who} is loading a model on port {port}.")
        } else if server::is_gemma(&h.model) {
            format!(
                "{who} has Gemma loaded on port {port} ({}). A summary uses it as it is, \
                 but while it holds the card the text in photos cannot be read.",
                h.model
            )
        } else {
            format!(
                "{who} has {} loaded on port {port}, not Gemma: no summary can run \
                 until it stops.",
                if h.model.is_empty() {
                    "a model"
                } else {
                    &h.model
                }
            )
        };
        ui.add_space(space::TIGHT);
        block(ui, |ui| ui.label(text(what, &p)));
        if !h.is_llama() {
            return;
        }
        let stopping = self.summaries.stopping.is_some();
        let label = if stopping {
            "Stopping…"
        } else if self.summaries.stop_armed {
            "Press again to stop it"
        } else {
            "Stop it"
        };
        let enabled = !stopping && !self.summaries.using_model();
        let pressed = row(ui, |ui| {
            button(ui, label, enabled, false, &p)
                .on_hover_text(format!(
                    "Ends process {} ({}). If the program that started it is in the \
                     middle of a job, that job fails, or it starts the server again.",
                    h.pid,
                    h.exe.display()
                ))
                .clicked()
        });
        if !pressed {
            return;
        }
        if !self.summaries.stop_armed {
            self.summaries.stop_armed = true;
            return;
        }
        self.summaries.stop_armed = false;
        log::info!("summary: stopping {who} (process {})", h.pid);
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ui.ctx().clone();
        let target = h.clone();
        std::thread::Builder::new()
            .name("tt-summary-stop".into())
            .spawn(move || {
                let _ = tx.send(holder::stop(&target));
                wake.request_repaint();
            })
            .expect("the OS refused a worker thread");
        self.summaries.stopping = Some((h, rx));
    }

    pub(super) fn summaries_panel(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.summaries_look(ctx);
        let p = self.palette;
        // A photograph cannot click: it picks the first chat of the folder it
        // was given (`TGX_SHOT_LIBRARY`), which names nobody.
        if super::shot::path().is_some() && self.summaries.picked.is_none() {
            if let Some(e) = self.library.state.entries.first().cloned() {
                self.summaries_pick(&e, ctx);
            }
        }
        let bare = egui::Frame::NONE.fill(p.bg);
        egui::SidePanel::left("summaries-chats")
            .frame(bare)
            .resizable(false)
            .exact_width(LIST_W)
            .show_inside(ui, |ui| self.summaries_list(ui, ctx));
        egui::CentralPanel::default()
            .frame(bare)
            .show_inside(ui, |ui| {
                if self.summaries.picked.is_none() {
                    EmptyState::new(
                        "Pick a chat",
                        Some(
                            "The chats are the Library's. Pick one, choose the days, and \
                             the local model writes what happened."
                                .into(),
                        ),
                    )
                    .show(ui, &p, true);
                    return;
                }
                self.summaries_form(ui);
                ui.add_space(space::STEP);
                rule(ui, &p);
                self.summaries_reading(ui);
            });
    }

    /// The chats, filtered, and the picked one's saved summaries.
    fn summaries_list(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let p = self.palette;
        ui.add_space(space::STEP);
        row(ui, |ui| {
            let f = ui.add(
                field(&mut self.summaries.filter, &p)
                    .hint_text("Filter chats…  Ctrl+F")
                    .desired_width(ui.available_width()),
            );
            if std::mem::take(&mut self.summaries.focus_filter) {
                f.request_focus();
            }
        });
        ui.add_space(space::STEP);
        let needle = self.summaries.filter.trim().to_lowercase();
        let entries: Vec<Entry> = self
            .library
            .state
            .entries
            .iter()
            .filter(|e| needle.is_empty() || e.title.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        let saved = self.summaries.saved.clone();
        let picked = self.summaries.picked.as_ref().map(|p| p.path.clone());
        let mut pick = None;
        let mut open = None;

        // The saved list takes what it needs at the foot; the chats the rest.
        egui::TopBottomPanel::bottom("summaries-saved")
            .frame(egui::Frame::NONE.fill(p.bg))
            .show_separator_line(false)
            .resizable(false)
            .max_height(ui.available_height() * 0.45)
            .show_inside(ui, |ui| {
                if picked.is_none() {
                    return;
                }
                rule(ui, &p);
                ui.add_space(space::STEP);
                row(ui, |ui| ui.label(eyebrow("Saved", &p)));
                ui.add_space(space::TIGHT);
                if saved.is_empty() {
                    row(ui, |ui| ui.label(meta("None yet for this chat.", &p)));
                }
                egui::ScrollArea::vertical()
                    .id_salt("summaries-saved-list")
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for s in &saved {
                            let on = self.summaries.shown.as_ref() == Some(&s.path);
                            let label = if on {
                                text(&s.name, &p)
                            } else {
                                meta(&s.name, &p)
                            };
                            if row(ui, |ui| action(ui, label, true)).clicked() {
                                open = Some(s.path.clone());
                            }
                        }
                    });
                ui.add_space(space::STEP);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(p.bg))
            .show_inside(ui, |ui| {
                if entries.is_empty() {
                    let why = if self.library.state.entries.is_empty() {
                        "No exports in the Library's folder. Choose one in the Library."
                    } else {
                        "No chat matches the filter."
                    };
                    row(ui, |ui| ui.label(meta(why, &p)));
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("summaries-chat-list")
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_HEIGHT, entries.len(), |ui, range| {
                        for e in &entries[range] {
                            let on = picked.as_deref() == Some(e.target.path());
                            let (rect, hit) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), ROW_HEIGHT),
                                Sense::click(),
                            );
                            if on || hit.hovered() {
                                ui.painter()
                                    .rect_filled(rect, egui::CornerRadius::ZERO, p.surface);
                            }
                            let inner = rect.shrink2(egui::vec2(GUTTER, 0.0));
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .max_rect(inner)
                                    .layout(Layout::left_to_right(Align::Center)),
                                |ui| {
                                    // The count first, from the right; the
                                    // title forwards in what is left, or a
                                    // truncating label in a right-to-left
                                    // layout sits against the far edge.
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let n = e
                                            .messages
                                            .map(|n| tgx_ui::thousands(n as i64))
                                            .unwrap_or_default();
                                        ui.label(figure(n, &p));
                                        ui.with_layout(
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                ui.add(
                                                    egui::Label::new(text(&e.title, &p)).truncate(),
                                                );
                                            },
                                        );
                                    });
                                },
                            );
                            if hit.clicked() {
                                pick = Some(e.clone());
                            }
                        }
                    });
            });

        if let Some(e) = pick {
            self.summaries_pick(&e, ctx);
        }
        if let Some(path) = open {
            if !self.summaries.running() {
                self.summaries.show_file(&path);
            }
        }
    }

    /// Days, length, scope, language, and what that will cost.
    fn summaries_form(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let running = self.summaries.running();
        let forum = self.summaries.chat.as_ref().is_some_and(|c| c.forum());
        let topics = self
            .summaries
            .chat
            .as_ref()
            .map(|c| c.topics())
            .unwrap_or_default();
        let span = self.summaries.chat.as_ref().and_then(|c| c.span());
        let mut remember = false;

        ui.add_space(space::STEP);
        row(ui, |ui| {
            let title = self
                .summaries
                .picked
                .as_ref()
                .map(|p| p.title.clone())
                .unwrap_or_default();
            ui.add(egui::Label::new(super::chrome::heading(&title, &p)).truncate());
            if let Some((a, b)) = span {
                ui.label(meta(format!("{a} – {b}"), &p));
            }
        });
        ui.add_space(space::STEP);

        wrapped(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            ui.label(meta("From", &p));
            ui.add_enabled(
                !running,
                field(&mut self.summaries.from, &p)
                    .hint_text("YYYY-MM-DD")
                    .desired_width(DATE_W),
            );
            ui.label(meta("To", &p));
            ui.add_enabled(
                !running,
                field(&mut self.summaries.to, &p)
                    .hint_text("YYYY-MM-DD")
                    .desired_width(DATE_W),
            );
            for (label, days) in [("Last day", 1), ("Last 7 days", 7), ("Last 30 days", 30)] {
                if action(ui, text(label, &p), !running && span.is_some()).clicked() {
                    self.summaries.last(days);
                }
            }
        });
        ui.add_space(space::TIGHT);

        wrapped(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            ui.label(meta("Length", &p));
            let at = Length::ALL
                .iter()
                .position(|l| *l == self.summaries.length)
                .unwrap_or(0);
            if let Some(i) = segmented(ui, &["Short", "Detailed", "Sections"], at, &p) {
                if !running {
                    self.summaries.length = Length::ALL[i];
                    remember = true;
                }
            }
        });
        ui.add_space(space::TIGHT);
        // A line of its own: beside Length it was wider than the panel at
        // the window's floor, and a strip of segments cannot wrap.
        wrapped(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            ui.label(meta("Language", &p));
            let at = Lang::ALL
                .iter()
                .position(|l| *l == self.summaries.lang)
                .unwrap_or(0);
            if let Some(i) = segmented(ui, &["Serbian", "English", "As the chat"], at, &p) {
                if !running {
                    self.summaries.lang = Lang::ALL[i];
                    remember = true;
                }
            }
        });

        // **Only a forum has topics**, so only a forum gets the choice.
        if forum {
            ui.add_space(space::TIGHT);
            wrapped(ui, |ui| {
                ui.spacing_mut().item_spacing.x = space::STEP;
                ui.label(meta("Scope", &p));
                let at = match self.summaries.scope {
                    Scope::Whole => 0,
                    Scope::ByTopic => 1,
                    Scope::One(_) => 2,
                };
                if let Some(i) = segmented(ui, &["Whole group", "By topic", "One topic"], at, &p) {
                    if !running {
                        self.summaries.scope = match i {
                            0 => Scope::Whole,
                            1 => Scope::ByTopic,
                            _ => Scope::One(0),
                        };
                    }
                }
                if let Scope::One(t) = self.summaries.scope {
                    let current = topics.get(t).cloned().unwrap_or_default();
                    egui::ComboBox::from_id_salt("summaries-topic")
                        .selected_text(text(current, &p))
                        .width(220.0)
                        .truncate()
                        .show_ui(ui, |ui| {
                            for (i, name) in topics.iter().enumerate() {
                                if ui.selectable_label(i == t, text(name, &p)).clicked() && !running
                                {
                                    self.summaries.scope = Scope::One(i);
                                }
                            }
                        });
                }
            });
        }

        ui.add_space(space::TIGHT);
        let line = match (self.summaries.reading.is_some(), self.summaries.options()) {
            (true, _) => "Reading the chat…".to_string(),
            (false, Err(e)) => e,
            (false, Ok(_)) => match self.summaries.estimate() {
                None => String::new(),
                Some(e) if e.messages == 0 => "Nothing was said in these days.".into(),
                Some(e) => {
                    let how = match e.parts {
                        0 => "one call".to_string(),
                        n => format!("about {n} parts, then the summary"),
                    };
                    let photos = match e.photos {
                        0 => String::new(),
                        _ if !self.settings.summary_photos => String::new(),
                        1 => " · 1 photo to read first".into(),
                        n => format!(" · {} photos to read first", tgx_ui::thousands(n as i64)),
                    };
                    format!(
                        "{} messages · about {} tokens · {how}{photos}",
                        tgx_ui::thousands(e.messages as i64),
                        tgx_ui::thousands(e.tokens as i64)
                    )
                }
            },
        };
        block(ui, |ui| ui.label(figure(line, &p)));
        self.summaries_holder(ui);

        if remember {
            self.settings.summary_length = self.summaries.length.code().into();
            self.settings.summary_lang = self.summaries.lang.code().into();
            self.commit_settings();
        }
    }

    /// The summary, as it is written or as it was saved; the conversation
    /// about the same days under it; the question box at the foot.
    fn summaries_reading(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let ask = egui::TopBottomPanel::bottom("summaries-ask")
            .frame(egui::Frame::NONE.fill(p.bg))
            .show_inside(ui, |ui| self.summaries_ask_row(ui))
            .inner;
        if ask {
            self.summaries_ask(ui.ctx());
        }
        let s = &self.summaries;
        if s.text.is_empty() && s.turns.is_empty() {
            ui.add_space(space::BREAK);
            let hint = if s.running() {
                "The summary appears here as it is written."
            } else {
                "Press Summarise, or ask about these days below. A Gemma already \
                 running is used; otherwise one is started from Settings › Summaries."
            };
            row(ui, |ui| ui.label(meta(hint, &p)));
            return;
        }
        let shown = s.shown.clone();
        let has_summary = !s.text.is_empty();
        let mut copy = false;
        let mut open = false;
        let mut reveal = false;
        if has_summary {
            ui.add_space(space::TIGHT);
            row(ui, |ui| {
                ui.spacing_mut().item_spacing.x = space::STEP;
                copy = action(ui, text("Copy", &p), true).clicked();
                if shown.is_some() {
                    open = action(ui, text("Open file", &p), true).clicked();
                    reveal = action(ui, text("Show in folder", &p), true).clicked();
                }
            });
        }
        // Citations are links only into a database: the Viewer reads nothing
        // else.
        let linkable = s.picked.as_ref().and_then(|p| p.chat_id).is_some();
        let text_now = s.text.clone();
        let turns = s.turns.clone();
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .id_salt("summaries-text")
            .auto_shrink([false, false])
            .stick_to_bottom(s.using_model())
            .show(ui, |ui| {
                ui.add_space(space::STEP);
                if has_summary {
                    clicked = markdown(ui, &text_now, linkable, &p);
                }
                for turn in &turns {
                    ui.add_space(space::BREAK);
                    ui.add(egui::Label::new(meta(format!("You: {}", turn.question), &p)).wrap());
                    ui.add_space(space::TIGHT);
                    if turn.answer.is_empty() {
                        ui.label(meta("…", &p));
                    } else if let Some(id) = markdown(ui, &turn.answer, linkable, &p) {
                        clicked = Some(id);
                    }
                }
                ui.add_space(space::BREAK);
            });
        if copy {
            ui.ctx().copy_text(text_now);
            self.summaries.status = "Copied".into();
        }
        if let Some(path) = &shown {
            if open {
                if let Err(e) = tga_run::open::open(path) {
                    self.summaries.status = e;
                }
            }
            if reveal {
                if let Err(e) = tga_run::open::reveal(path.parent().unwrap_or(path)) {
                    self.summaries.status = e;
                }
            }
        }
        if let (Some(id), Some(picked)) = (clicked, self.summaries.picked.clone()) {
            if let Some(chat_id) = picked.chat_id {
                self.summaries.cite = Some((
                    super::library::Read {
                        path: picked.path,
                        chat_id,
                        title: picked.title,
                    },
                    id,
                ));
            }
        }
    }

    /// The question box: a field, Ask (or Enter), Stop while it answers, and
    /// Clear for a new conversation. Whether a question was sent.
    fn summaries_ask_row(&mut self, ui: &mut Ui) -> bool {
        let p = self.palette;
        let s = &mut self.summaries;
        let ready = s.chat.is_some()
            && s.options().is_ok()
            && s.estimate.as_ref().is_some_and(|(_, e)| e.messages > 0)
            && !s.using_model();
        let (mut send, mut stop, mut clear) = (false, false, false);
        rule(ui, &p);
        ui.add_space(space::STEP);
        row(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = space::STEP;
                if s.asking() {
                    stop = button(ui, "Stop", true, false, &p).clicked();
                } else {
                    let has = !s.question.trim().is_empty();
                    send = button(ui, "Ask", ready && has, false, &p).clicked();
                }
                if !s.turns.is_empty() && !s.using_model() {
                    clear = action(ui, text("Clear", &p), true)
                        .on_hover_text("Start a new conversation about these days")
                        .clicked();
                }
                let edit = ui.add(
                    field(&mut s.question, &p)
                        .hint_text("Ask about these days: who said what, what was decided…")
                        .desired_width(ui.available_width()),
                );
                if edit.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && ready
                    && !s.question.trim().is_empty()
                {
                    send = true;
                    edit.request_focus();
                }
            });
        });
        ui.add_space(space::STEP);
        if stop {
            s.stop();
        }
        if clear {
            s.turns.clear();
            s.grounding = None;
        }
        send
    }

    /// A citation clicked: the Viewer, on that message, with its way back
    /// here.
    pub(super) fn summaries_cite(&mut self, read: super::library::Read, id: i64) {
        let db = tgx_tg::viewer::Database::from(read);
        if self.viewer.is_open(&db.path, db.chat_id) {
            self.viewer_select(id, true);
        } else {
            self.viewer_open(db, Some(id));
        }
        self.viewer.back = Some(View::Summaries);
        self.show(View::Viewer);
    }

    /// The idle model server stopped once its minutes are up, and a frame asked
    /// for at the moment they will be. Every frame, whichever view shows.
    pub(super) fn summaries_idle(&mut self, ctx: &egui::Context) {
        let Some(idle) = self.summaries.keeper.idle() else {
            return;
        };
        let limit = std::time::Duration::from_secs(
            u64::from(self.settings.summary_idle_minutes.max(1)) * 60,
        );
        let in_use = self.summaries.using_model() || self.summaries_here();
        match idle_step(idle, limit, in_use) {
            Idle::Touch => self.summaries.keeper.touch(),
            Idle::Stop => {
                self.summaries.keeper.stop();
                self.journal.push(format!(
                    "Stopped the model server: unused for {} min",
                    self.settings.summary_idle_minutes
                ));
            }
            Idle::Wait(left) => ctx.request_repaint_after(left),
        }
    }

    /// Whether the Summaries view counts as open for the idle clock.
    ///
    /// **Loaded while the view is open.** Reading a summary takes longer than
    /// the idle minutes, and a question after it then waited for 15 GB to load
    /// again. So the clock runs only once the view is left — a citation's trip
    /// into the Viewer, which comes back here, does not count as leaving. Any
    /// other way into the Viewer does: [`Shell::show`] drops the way back as
    /// soon as the Viewer is left.
    pub(super) fn summaries_here(&self) -> bool {
        self.body == View::Summaries
            || (self.body == View::Viewer && self.viewer.back == Some(View::Summaries))
    }
}

/// What the idle clock does on a frame, given how long the server this
/// window started has gone unused.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Idle {
    /// In use, or looked at: the clock starts again.
    Touch,
    /// Its minutes are up.
    Stop,
    /// Not yet: a frame is wanted when they will be.
    Wait(std::time::Duration),
}

/// The idle clock's rule, apart from the clock and the server, so it can be
/// held by a test.
pub(super) fn idle_step(
    idle: std::time::Duration,
    limit: std::time::Duration,
    in_use: bool,
) -> Idle {
    if in_use {
        Idle::Touch
    } else if idle >= limit {
        Idle::Stop
    } else {
        Idle::Wait(limit - idle)
    }
}

/// One piece of a line: text, or a citation's ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Piece {
    Text(String),
    Cite(Vec<i64>),
}

/// A line cut into text and citations — `[#1]`, `[#1, #2]`.
pub(super) fn pieces(line: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut rest = line;
    let mut plain = String::new();
    while let Some(open) = rest.find("[#") {
        let after = &rest[open + 1..];
        let ids: Option<Vec<i64>> = after.find(']').and_then(|close| {
            after[..close]
                .split(',')
                .map(|s| s.trim().strip_prefix('#')?.parse().ok())
                .collect()
        });
        match ids {
            Some(ids) if !ids.is_empty() => {
                plain.push_str(&rest[..open]);
                if !plain.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut plain)));
                }
                out.push(Piece::Cite(ids));
                rest = &after[after.find(']').unwrap_or(0) + 1..];
            }
            _ => {
                plain.push_str(&rest[..open + 2]);
                rest = &rest[open + 2..];
            }
        }
    }
    plain.push_str(rest);
    if !plain.is_empty() {
        out.push(Piece::Text(plain));
    }
    out
}

/// A padded row that wraps onto the next line when the panel is too narrow
/// for it — at the window's 900pt floor the form does not fit one line.
fn wrapped<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    block(ui, |ui| ui.horizontal_wrapped(add).inner)
}

/// Draw the summary's Markdown — the little of it a summary uses: `#`
/// headings, `- ` bullets, `> ` notes, `*emphasis*` markers dropped, and
/// citations as links. Returns the message a citation click asked for.
fn markdown(ui: &mut Ui, md: &str, linkable: bool, p: &tgx_ui::tokens::Palette) -> Option<i64> {
    let mut clicked = None;
    let mut after_title = false;
    for raw in md.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            ui.add_space(space::TIGHT);
            continue;
        }
        if let Some(h) = line.strip_prefix("# ") {
            block(ui, |ui| ui.label(super::chrome::heading(h, p)));
            after_title = true;
            continue;
        }
        if after_title {
            // The line under the title says what this is: scope, length,
            // language, size, model, when.
            after_title = false;
            block(ui, |ui| ui.label(meta(line, p)));
            continue;
        }
        if let Some(h) = line
            .strip_prefix("## ")
            .or_else(|| line.strip_prefix("### "))
        {
            ui.add_space(space::STEP);
            block(ui, |ui| ui.label(eyebrow(h, p)));
            continue;
        }
        let (lead, body, quiet) = if let Some(b) = line.strip_prefix("- ") {
            ("•  ", b, false)
        } else if let Some(b) = line.strip_prefix("> ") {
            ("", b, true)
        } else {
            ("", line, false)
        };
        let body = body.replace('*', "");
        block(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                // Pieces abut; a wrapped line sits as close as a label's own
                // would.
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                if !lead.is_empty() {
                    ui.label(text(lead, p));
                }
                for piece in pieces(&body) {
                    match piece {
                        Piece::Text(t) if quiet => {
                            ui.label(meta(t, p));
                        }
                        Piece::Text(t) => {
                            ui.label(text(t, p));
                        }
                        // **Each citation one unbreakable piece**: a label
                        // that wraps would leave `[#2` on one line and `]`
                        // on the next. A link per id, so each opens its own
                        // message.
                        Piece::Cite(ids) if linkable => {
                            for id in ids {
                                let label = figure(format!("[#{id}]"), p).color(p.accent);
                                let hit = ui
                                    .add(
                                        egui::Label::new(label)
                                            .wrap_mode(egui::TextWrapMode::Extend)
                                            .sense(Sense::click()),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text("Open this message in the Viewer");
                                if hit.clicked() {
                                    clicked = Some(id);
                                }
                            }
                        }
                        Piece::Cite(ids) => {
                            let list: Vec<String> = ids.iter().map(|i| format!("#{i}")).collect();
                            ui.add(
                                egui::Label::new(figure(format!("[{}]", list.join(", ")), p))
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    }
                }
            });
        });
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_comes_apart_into_text_and_citations() {
        assert_eq!(
            pieces("A strike [#12]. Split [#3, #4] over [photo]."),
            vec![
                Piece::Text("A strike ".into()),
                Piece::Cite(vec![12]),
                Piece::Text(". Split ".into()),
                Piece::Cite(vec![3, 4]),
                Piece::Text(" over [photo].".into()),
            ]
        );
        assert_eq!(pieces("[#x] no"), vec![Piece::Text("[#x] no".into())]);
    }

    #[test]
    fn the_form_needs_a_from_day_and_to_defaults_to_it() {
        let mut s = SummariesState::default();
        assert!(s.options().is_err());
        s.from = "2024-12-02".into();
        let o = s.options().unwrap();
        assert_eq!(o.from, o.to);
        s.to = "2024-12-01".into();
        assert_eq!(s.options().unwrap_err(), "To is before From");
        s.to = "2024-12-09".into();
        s.scope = Scope::ByTopic;
        // No chat read yet, so no forum: the scope does not apply.
        let o = s.options().unwrap();
        assert!(!o.by_topic && o.topic.is_none());
    }

    #[test]
    fn nothing_to_stop_when_nothing_was_started() {
        let s = SummariesState::default();
        assert!(!s.busy());
        assert_eq!(s.keeper.idle(), None);
    }

    // -- the state machine, driven without a window or a model -------------
    //
    // `reply` and `apply` are what `drain` hands each event to, so feeding
    // them directly is the worker's half of the conversation with none of
    // its threads. Nothing here starts a server or reads a real chat.

    fn no_wake() -> tga_run::job::Wake {
        Arc::new(|| {})
    }

    /// A chat that is not there: reading it fails at once, offline.
    fn nowhere(title: &str) -> Picked {
        let path = std::env::temp_dir()
            .join("tt-summaries-test-none")
            .join(title);
        Picked {
            home: path.clone(),
            path,
            chat_id: Some(7),
            title: title.into(),
        }
    }

    /// Drain until the read in flight has answered.
    fn read_through(s: &mut SummariesState) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while s.reading.is_some() {
            assert!(std::time::Instant::now() < until, "the read never answered");
            std::thread::sleep(std::time::Duration::from_millis(5));
            s.drain();
        }
    }

    fn grounding(text: &str) -> Arc<Grounding> {
        Arc::new(Grounding {
            text: text.into(),
            notes: false,
            known: Default::default(),
            topic: None,
            messages: 1,
            listed: Vec::new(),
            days: Vec::new(),
        })
    }

    fn key(from: &str) -> AskKey {
        (
            PathBuf::from("chat"),
            from.into(),
            String::new(),
            Scope::Whole,
        )
    }

    #[test]
    fn a_failed_read_is_said_and_picking_the_chat_again_retries_it() {
        let mut s = SummariesState::default();
        let chat = nowhere("Gone");
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Err("database is locked".to_string())).unwrap();
        s.picked = Some(chat.clone());
        s.reading = Some(rx);
        s.drain();
        assert!(s.reading.is_none() && s.chat.is_none());
        assert_eq!(s.status, "database is locked");

        // The same row again is a retry, not a click on the chat open.
        s.pick(chat.clone(), no_wake());
        assert!(s.reading.is_some(), "read again");
        assert_eq!(s.status, "Reading Gone");
        // While that read is in flight, another click on it is not a third.
        s.status = "still reading".into();
        s.pick(chat.clone(), no_wake());
        assert_eq!(s.status, "still reading");
        read_through(&mut s);
        assert!(s.chat.is_none());
        assert!(s.status.contains("Not a database"), "got {}", s.status);
    }

    #[test]
    fn a_chat_read_opens_on_its_last_week() {
        // Two invented people, eleven days apart: the last seven days are the
        // last day alone, and the form opens on them.
        let dir = std::env::temp_dir().join(format!("tt-summaries-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("result.json"),
            r#"{"name": "Scratch", "type": "private_supergroup", "id": 1, "messages": [
 {"id": 1, "type": "message", "date": "2025-01-01T09:00:00", "date_unixtime": "1735718400",
  "from": "Ana", "from_id": "user1", "text": "a", "text_entities": [{"type": "plain", "text": "a"}]},
 {"id": 2, "type": "message", "date": "2025-01-12T09:00:00", "date_unixtime": "1736668800",
  "from": "Bob", "from_id": "user2", "text": "b", "text_entities": [{"type": "plain", "text": "b"}]}
]}"#,
        )
        .unwrap();
        let mut s = SummariesState::default();
        s.pick(
            Picked {
                path: dir.clone(),
                chat_id: None,
                title: "Scratch".into(),
                home: dir.clone(),
            },
            no_wake(),
        );
        read_through(&mut s);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(s.chat.is_some(), "status: {}", s.status);
        assert_eq!(s.status, "2 messages");
        assert_eq!(
            (s.from.as_str(), s.to.as_str()),
            ("2025-01-06", "2025-01-12")
        );
    }

    #[test]
    fn an_answer_streams_into_the_last_turn_and_keeps_its_reading() {
        let mut s = SummariesState::default();
        s.turns.push(Turn {
            question: "who?".into(),
            answer: String::new(),
        });
        s.ask_key = Some(key("2025-01-01"));
        s.reply(Reply::Step("Reading the days".into()));
        assert_eq!(s.status, "Reading the days");
        s.reply(Reply::Event(Event::Text("An".into())));
        s.reply(Reply::Event(Event::Text("a".into())));
        assert_eq!(s.turns[0].answer, "Ana");
        s.reply(Reply::Done(Ok(run::Answered {
            answer: "Ana [#1]".into(),
            grounding: grounding("days"),
        })));
        assert_eq!(
            s.turns[0].answer, "Ana [#1]",
            "the checked answer replaces the stream"
        );
        assert_eq!(s.status, "Answered");
        assert!(s.ask_key.is_none());
        let (k, g) = s.grounding.as_ref().expect("kept for the next question");
        assert_eq!((k, g.text.as_str()), (&key("2025-01-01"), "days"));
    }

    #[test]
    fn a_stopped_answer_says_stopped_and_keeps_what_was_written() {
        let mut s = SummariesState::default();
        s.turns.push(Turn::default());
        s.reply(Reply::Done(Err("cancelled".into())));
        assert_eq!(s.status, "Stopped");
        assert_eq!(s.turns[0].answer, "*No answer: Stopped*");

        s.turns.push(Turn::default());
        s.reply(Reply::Event(Event::Text("half an".into())));
        s.reply(Reply::Done(Err("the server went away".into())));
        assert_eq!(s.status, "the server went away");
        assert_eq!(
            s.turns[1].answer, "half an",
            "a partial answer is not replaced"
        );
        assert!(s.grounding.is_none(), "a failure keeps no reading");
    }

    #[test]
    fn a_question_about_other_days_starts_a_new_conversation() {
        let mut s = SummariesState {
            grounding: Some((key("2025-01-01"), grounding("first week"))),
            turns: vec![Turn {
                question: "q".into(),
                answer: "a".into(),
            }],
            ..Default::default()
        };
        let same = s.grounding_for(&key("2025-01-01"));
        assert_eq!(same.map(|g| g.text.clone()).as_deref(), Some("first week"));
        assert_eq!(s.turns.len(), 1, "the same days carry on");

        assert!(s.grounding_for(&key("2025-02-01")).is_none());
        assert!(
            s.turns.is_empty(),
            "the old turns were about other messages"
        );
        assert!(s.grounding.is_none());
    }

    #[test]
    fn a_summary_says_its_parts_as_it_writes_and_stopped_when_stopped() {
        let mut s = SummariesState::default();
        s.apply(Progress::Event(Event::Part {
            done: 1,
            of: 3,
            cached: true,
            secs: 0.0,
            timings: None,
        }));
        assert!(s.status.contains("kept from an earlier run"));
        s.apply(Progress::Event(Event::Part {
            done: 3,
            of: 3,
            cached: false,
            secs: 1.0,
            timings: None,
        }));
        assert_eq!(s.part, Some((3, 3)));
        assert_eq!(s.status, "Read part 3 of 3");
        s.apply(Progress::Event(Event::Text("Overview.".into())));
        assert_eq!(
            s.status, "Writing",
            "the parts are read; the summary is begun"
        );
        s.apply(Progress::Event(Event::Section(Some("Foto".into()))));
        s.apply(Progress::Event(Event::Text("A day.".into())));
        assert_eq!(s.text, "Overview.\n\n## Foto\n\nA day.");
        s.apply(Progress::Done(Err("cancelled".into())));
        assert_eq!(s.status, "Stopped");
        assert!(s.part.is_none() && s.job.is_none());
    }

    #[test]
    fn a_finished_summary_names_its_file_and_what_it_is_missing() {
        let mut s = SummariesState {
            text: "streamed".into(),
            ..Default::default()
        };
        s.apply(Progress::Done(Ok(run::Finished {
            path: PathBuf::from("2025-01-01_short_sr.md"),
            document: "# Saved".into(),
            summary: tga_run::tga_sum::Summary {
                parts: 4,
                failed: 1,
                ..Default::default()
            },
            unread_photos: 1,
        })));
        assert_eq!(s.text, "# Saved", "the file's text, header and all");
        assert!(s.status.starts_with("Saved as 2025-01-01_short_sr.md"));
        assert!(s.status.contains("1 of 4 parts got no answer"));
        assert!(s.status.contains("the text in 1 photo was not read"));
        assert_eq!(s.shown, Some(PathBuf::from("2025-01-01_short_sr.md")));
    }

    #[test]
    fn the_models_folder_is_looked_at_off_the_frame_and_once_at_a_time() {
        let ctx = egui::Context::default();
        let folder = std::env::temp_dir().join("tt-summaries-test-no-models");
        let mut s = SummariesState::default();
        s.look_for_models(&folder, &ctx);
        assert!(s.found.is_none(), "the frame did not wait for the disk");
        assert!(s.looking_models.is_some());
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while s.found.is_none() {
            assert!(std::time::Instant::now() < until, "the look never answered");
            std::thread::sleep(std::time::Duration::from_millis(5));
            s.look_for_models(&folder, &ctx);
        }
        let found = s.found.as_ref().unwrap();
        assert_eq!(found.folder, folder);
        assert!(found.models.is_empty() && found.server.is_none());
        // Fresh: the next frames ask nothing.
        s.look_for_models(&folder, &ctx);
        assert!(s.looking_models.is_none());
        // Another folder is looked at, and the last look stays meanwhile.
        s.look_for_models(&folder.join("other"), &ctx);
        assert!(s.looking_models.is_some() && s.found.is_some());
    }

    #[test]
    fn the_idle_clock_runs_only_once_the_server_is_left_alone() {
        use std::time::Duration;
        let limit = Duration::from_secs(600);
        assert_eq!(
            idle_step(Duration::from_secs(900), limit, true),
            Idle::Touch
        );
        assert_eq!(
            idle_step(Duration::from_secs(600), limit, false),
            Idle::Stop
        );
        assert_eq!(
            idle_step(Duration::from_secs(60), limit, false),
            Idle::Wait(Duration::from_secs(540))
        );
    }
}
