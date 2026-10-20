//! Catch up: what happened in the Analysis view's chats since you last looked, one
//! card per chat — the Summaries view's first page.
//!
//! **Chats down the left, each with what is new in it; cards on the right.**
//! What is new is counted from the databases' own indexes on a worker
//! (`tga_run::catchup::survey`) whenever the days or Analysis's rows change, so
//! the list says where there is anything to read before the model is asked.
//! One press writes a short summary of every chat with something new — or of
//! the chats ticked — busiest first, each card filling in as its chat is
//! written.
//!
//! **"Since the last catch-up" is counted in database updates**: see
//! `tga_run::catchup`. Each chat keeps its own bookmark, and only a summary
//! that was saved moves it.
//!
//! A card's **Ask about these days** opens the chat on the One chat page, on
//! the same days, with the card's summary and the question box under it.

use super::summaries::{
    band, dates_row, filter_row, form_row, hint, markdown, Mode, Span, BAR_H, HEAD_H,
};
use super::*;
use eframe::egui::{Align, Layout, Sense, Ui};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use tga_run::catchup::{self as run, Card, Days, Group, News, Progress, Record, Step};
use tga_run::state::Entry;
use tgx_ui::components::{
    action, block, button, disclosure, figure, meta, progress_bar, row, rule, segmented, text,
    tick_mark, EmptyState, GUTTER,
};
use tgx_ui::tokens::space;

/// A chat row in the list.
const ROW_HEIGHT: f32 = 30.0;

/// The earlier catch-ups' menu: a date and a time, and the arrow.
const EARLIER_W: f32 = 170.0;

/// The days on offer, as the segmented control lists them, before Dates.
///
/// **No Today.** 24 hours took its place: the row has to fit the window's
/// floor, and a settings file that remembers Today opens on 24 hours.
const DAYS: [(&str, Days); 4] = [
    ("Since last", Days::SinceLast),
    ("24 h", Days::Hours24),
    ("3 days", Days::Last(3)),
    ("7 days", Days::Last(7)),
];

/// Where one card has got to.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum CardState {
    Waiting,
    Reading,
    Read,
    Writing,
    Done(Card),
    /// Not written: why — "Nothing new" among the reasons.
    Failed(String),
}

/// One chat's card.
#[derive(Debug, Clone)]
pub(super) struct CardView {
    pub(super) group: Group,
    pub(super) state: CardState,
    /// The summary as it is written.
    pub(super) text: String,
    /// The days and how many messages, once they are read.
    pub(super) span: Option<(chrono::NaiveDate, chrono::NaiveDate, usize)>,
}

/// Everything the Catch up page remembers between frames.
#[derive(Default)]
pub(super) struct CatchupState {
    /// The days chosen. `Days::Between` is the dates below.
    pub(super) days: Days,
    pub(super) from: String,
    pub(super) to: String,
    /// What is new in each chat, by database path, for `counted`'s days.
    pub(super) news: HashMap<PathBuf, Result<News, String>>,
    survey: Option<Receiver<(PathBuf, Result<News, String>)>>,
    /// The days, where they count back from, and the Analysis scan the counts
    /// are of: any of them changing counts them again.
    counted: Option<(Days, bool, u64)>,
    job: Option<run::Job>,
    /// The cards on screen: the run going or last run, or a record opened.
    pub(super) cards: Vec<CardView>,
    /// The record the cards are, when they are an earlier catch-up's.
    pub(super) shown: Option<PathBuf>,
    pub(super) records: Vec<Record>,
    /// The folder the records were last listed from.
    listed: Option<PathBuf>,
    pub(super) status: String,
    /// The form is folded away above the cards. Set when a run starts or a
    /// record opens, cleared by its header; it only folds while there are
    /// cards to make room for.
    pub(super) folded: bool,
    /// The chat whose summary is open, by database path — stable while the
    /// strip is filtered and re-sorted. `None` follows the run: the chat
    /// being written, else the first.
    pub(super) selected: Option<PathBuf>,
    /// Bring the open chat's chip into view on the next frame: it was chosen
    /// by Previous or Next, so it may be off the strip's visible part.
    strip_jump: bool,
    /// When the run going was started.
    started: Option<std::time::Instant>,
    /// How long the last run took, kept with its cards so the bar still says
    /// how it ended; gone when an earlier catch-up is opened.
    took: Option<std::time::Duration>,
    /// The chat being written: its parts' notes in, of how many.
    part: Option<(usize, usize)>,
}

impl CatchupState {
    pub(super) fn running(&self) -> bool {
        self.job.is_some()
    }

    pub(super) fn busy(&self) -> bool {
        self.job.is_some() || self.survey.is_some()
    }

    pub(super) fn stop(&mut self) {
        if let Some(job) = &self.job {
            job.stop();
            self.status = "Stopping".into();
        }
    }

    /// The days the form says, or why they cannot be.
    pub(super) fn chosen_days(&self) -> Result<Days, String> {
        match self.days {
            Days::Between(..) => {
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
                Ok(Days::Between(from, to))
            }
            d => Ok(d),
        }
    }

    /// The written cards, and how far the run has got.
    pub(super) fn progress(&self) -> Option<(usize, usize)> {
        self.running().then(|| self.finished())
    }

    /// The chats finished — written, failed, or with nothing new — of how
    /// many. One a stop left unwritten is not finished, so a stopped run's
    /// bar stays where it stopped.
    fn finished(&self) -> (usize, usize) {
        let done = self
            .cards
            .iter()
            .filter(|c| match &c.state {
                CardState::Done(_) => true,
                CardState::Failed(why) => why != "Stopped",
                _ => false,
            })
            .count();
        (done, self.cards.len())
    }

    /// The bar and its line, while a run goes and after it: the chats
    /// finished and the clock. `None` for an earlier catch-up opened.
    fn bar(&self) -> Option<(f32, String)> {
        let clock = crate::pace::clock(self.elapsed()?);
        let (done, of) = self.finished();
        let line = if self.running() {
            format!("{done} of {of} chats · {clock} elapsed")
        } else {
            format!("{done} of {of} chats · took {clock}")
        };
        Some((self.fraction(), line))
    }

    /// How far the run has got, 0 to 1: the chats finished, and of the one
    /// being written the parts whose notes are in — the summary written from
    /// them counted as one part more, so a chat is not full before it is.
    fn fraction(&self) -> f32 {
        let (done, of) = self.finished();
        let within = match self.part {
            Some((d, n)) => d as f32 / (n + 1) as f32,
            None => 0.0,
        };
        (done as f32 + within) / of.max(1) as f32
    }

    /// The run's clock: how long it has been going, or how long the last
    /// one took.
    pub(super) fn elapsed(&self) -> Option<std::time::Duration> {
        self.started.map(|t| t.elapsed()).or(self.took)
    }

    /// Apply what the workers have said since the last frame.
    pub(super) fn drain(&mut self) {
        if let Some(rx) = &self.survey {
            loop {
                match rx.try_recv() {
                    Ok((path, found)) => {
                        self.news.insert(path, found);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.survey = None;
                        break;
                    }
                }
            }
        }
        let Some(job) = &self.job else { return };
        let batch: Vec<Progress> = job.events.try_iter().collect();
        for p in batch {
            self.apply(p);
        }
    }

    fn apply(&mut self, progress: Progress) {
        match progress {
            Progress::Step(s) => {
                // Counts, models and timings: never a message.
                log::info!("catch-up: {s}");
                self.status = s;
            }
            Progress::Group(i, step) => {
                let Some(card) = self.cards.get_mut(i) else {
                    return;
                };
                match step {
                    Step::Reading => {
                        card.state = CardState::Reading;
                        self.status = format!("Reading {}", card.group.title);
                    }
                    Step::Read { messages, from, to } => {
                        card.state = CardState::Read;
                        card.span = Some((from, to, messages));
                    }
                    Step::Writing => {
                        card.state = CardState::Writing;
                        card.text.clear();
                        self.part = None;
                        self.status = format!("Writing {}", card.group.title);
                    }
                    Step::Event(e) => match e {
                        tga_run::tga_sum::Event::Text(piece) => card.text.push_str(&piece),
                        tga_run::tga_sum::Event::Section(Some(name)) => {
                            card.text.push_str(&format!("\n\n## {name}\n\n"))
                        }
                        tga_run::tga_sum::Event::Section(None) => card.text.push_str("\n\n"),
                        tga_run::tga_sum::Event::Part { done, of, .. } => {
                            self.part = Some((done, of));
                            self.status = format!("{}: part {done} of {of}", card.group.title);
                        }
                        tga_run::tga_sum::Event::Say(s) => self.status = s,
                        tga_run::tga_sum::Event::Written(_) => {}
                    },
                    Step::Done(Ok(c)) => {
                        self.part = None;
                        card.text = c.text.clone();
                        card.span = Some((c.from, c.to, c.messages));
                        card.state = CardState::Done(c);
                    }
                    Step::Done(Err(e)) => {
                        self.part = None;
                        card.state = CardState::Failed(e);
                    }
                }
            }
            Progress::Done(done) => {
                self.job = None;
                self.part = None;
                self.took = self.started.take().map(|t| t.elapsed());
                // The bookmarks moved: what is new is counted again.
                self.counted = None;
                self.listed = None;
                for card in &mut self.cards {
                    if !matches!(card.state, CardState::Done(_) | CardState::Failed(_)) {
                        card.state = CardState::Failed("Stopped".into());
                    }
                }
                match done {
                    Ok(f) => {
                        self.shown = f.record;
                        self.status = match f.written {
                            0 => "Nothing new to summarise".into(),
                            1 => "Caught up: one chat".into(),
                            n => format!("Caught up: {n} chats"),
                        };
                    }
                    Err(e) if e == "cancelled" => self.status = "Stopped".into(),
                    Err(e) => self.status = e,
                }
            }
        }
    }
}

/// The Analysis view's databases, as a catch-up's chats, A to Z in either
/// alphabet (`crate::fold`). Folder exports have no updates to count, so they
/// are not among them.
fn groups(entries: &[Entry]) -> Vec<Group> {
    let mut out: Vec<Group> = entries
        .iter()
        .filter(|e| matches!(e.target.source, tga_run::refresh::Source::Database(_)))
        .map(|e| Group {
            path: e.target.path().to_path_buf(),
            chat_id: e.chat_id,
            title: e.title.clone(),
            home: e.target.home(),
        })
        .collect();
    out.sort_by_cached_key(|g| crate::fold::fold(&g.title));
    out
}

impl Shell {
    /// Where the records of earlier catch-ups go: `Catch-ups` in the
    /// Analysis folder, beside the chats they are of.
    fn catchup_records(&self) -> Option<PathBuf> {
        self.analysis
            .state
            .root
            .as_ref()
            .map(|r| r.join("Catch-ups"))
    }

    /// Count what is new again when the days or Analysis's rows changed, and
    /// list the records when their folder did.
    fn catchup_look(&mut self, ctx: &egui::Context) {
        let generation = self.analysis.state.generation;
        let records = self.catchup_records();
        let c = &mut self.summaries.catchup;
        if c.listed != records {
            c.records = records.as_deref().map(run::records).unwrap_or_default();
            c.listed = records;
        }
        let Ok(days) = c.chosen_days() else { return };
        let from_last = self.settings.summary_from_last;
        if c.counted == Some((days, from_last, generation)) || c.running() {
            return;
        }
        c.counted = Some((days, from_last, generation));
        c.news.clear();
        let all = groups(&self.analysis.state.entries);
        if all.is_empty() {
            c.survey = None;
            return;
        }
        let wake = ctx.clone();
        c.survey = Some(run::survey(
            all,
            days,
            from_last,
            std::sync::Arc::new(move || wake.request_repaint()),
        ));
    }

    /// The chats a press of Catch up would write, busiest first.
    fn catchup_targets(&self) -> Vec<Group> {
        let c = &self.summaries.catchup;
        let chosen: std::collections::HashSet<&str> = self
            .settings
            .catchup_chosen
            .iter()
            .map(String::as_str)
            .collect();
        let new = |g: &Group| match c.news.get(&g.path) {
            Some(Ok(n)) => n.messages,
            _ => 0,
        };
        let fresh = |g: &Group| matches!(c.news.get(&g.path), Some(Ok(n)) if n.fresh);
        let mut out: Vec<Group> = groups(&self.analysis.state.entries)
            .into_iter()
            .filter(|g| {
                if self.settings.catchup_auto {
                    fresh(g)
                } else {
                    chosen.contains(g.path.to_string_lossy().as_ref())
                }
            })
            .collect();
        out.sort_by_key(|g| std::cmp::Reverse(new(g)));
        out
    }

    pub(super) fn catchup_start(&mut self, ctx: &egui::Context) {
        if self.summaries.using_model() {
            return;
        }
        let Ok(days) = self.summaries.catchup.chosen_days() else {
            return;
        };
        let targets = self.catchup_targets();
        let Some(records) = self.catchup_records() else {
            return;
        };
        if targets.is_empty() {
            return;
        }
        let server = super::summaries::summary_server(&self.settings);
        let from_last = self.settings.summary_from_last;
        let options = run::Options {
            days,
            from_last,
            by_topic: self.settings.catchup_by_topic,
            lang: self.summaries.lang,
        };
        log::info!(
            "catch-up: {} chats, {}",
            targets.len(),
            run::describe(days, from_last)
        );
        let c = &mut self.summaries.catchup;
        c.cards = targets
            .iter()
            .map(|g| CardView {
                group: g.clone(),
                state: CardState::Waiting,
                text: String::new(),
                span: None,
            })
            .collect();
        c.shown = None;
        c.folded = true;
        c.selected = None;
        c.status = "Reading the chats".into();
        c.started = Some(std::time::Instant::now());
        c.took = None;
        c.part = None;
        let wake = ctx.clone();
        c.job = Some(run::spawn(
            targets,
            options,
            server,
            self.summaries.keeper.clone(),
            records,
            std::sync::Arc::new(move || wake.request_repaint()),
        ));
    }

    /// The top bar while the Catch up page shows: Catch up, and Stop.
    pub(super) fn catchup_actions(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let p = self.palette;
        let running = self.summaries.catchup.running();
        let ready = !self.summaries.using_model()
            && self.summaries.catchup.chosen_days().is_ok()
            && self.catchup_records().is_some()
            && !self.catchup_targets().is_empty();
        let (mut start, mut stop) = (false, false);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = if running {
                "Catching up…"
            } else {
                "Catch up"
            };
            start = button(ui, label, ready, true, &p).clicked();
            if running {
                stop = button(ui, "Stop", true, false, &p).clicked();
            }
        });
        if start {
            self.catchup_start(ctx);
        }
        if stop {
            self.summaries.catchup.stop();
        }
    }

    /// The left column: the chats with what is new in each, with All and None
    /// over them. The earlier catch-ups are a menu in the page's heading
    /// (`catchup_earlier`): under the list they took a third of the column
    /// from the chats.
    pub(super) fn catchup_list(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let s = &mut self.summaries;
        filter_row(ui, &mut s.filter, &mut s.focus_filter, &p);
        let needle = self.summaries.filter.clone();
        let all = groups(&self.analysis.state.entries);
        let listed: Vec<&Group> = all
            .iter()
            .filter(|g| crate::fold::matches(&g.title, &needle))
            .collect();
        let auto = self.settings.catchup_auto;
        let running = self.summaries.catchup.running();
        let mut toggle: Option<String> = None;
        let mut all_or_none: Option<bool> = None;
        // The chats "Every chat with news" picks, which a tick by hand starts
        // from.
        let with_news: Vec<String> = all
            .iter()
            .filter(|g| matches!(self.summaries.catchup.news.get(&g.path), Some(Ok(n)) if n.fresh))
            .map(|g| g.path.to_string_lossy().into_owned())
            .collect();
        let shown_keys: Vec<String> = listed
            .iter()
            .map(|g| g.path.to_string_lossy().into_owned())
            .collect();

        // How many are ticked, and All and None for the chats the filter
        // shows — every chat, with no filter.
        if !listed.is_empty() {
            let ticked = if auto {
                with_news.len()
            } else {
                all.iter()
                    .filter(|g| {
                        let key = g.path.to_string_lossy();
                        self.settings.catchup_chosen.iter().any(|k| *k == key)
                    })
                    .count()
            };
            row(ui, |ui| {
                ui.label(meta(format!("{ticked} of {} ticked", all.len()), &p));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if action(ui, text("None", &p), !running).clicked() {
                        all_or_none = Some(false);
                    }
                    if action(ui, text("All", &p), !running).clicked() {
                        all_or_none = Some(true);
                    }
                });
            });
            ui.add_space(space::TIGHT);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(p.bg))
            .show_inside(ui, |ui| {
                if all.is_empty() {
                    let why = if self.analysis.state.entries.is_empty() {
                        "No exports in the Analysis folder. Choose one in Analysis."
                    } else {
                        "No databases in the Analysis folder: a catch-up reads what each \
                         sync added, and only a database records that."
                    };
                    block(ui, |ui| ui.label(meta(why, &p)));
                    return;
                }
                if listed.is_empty() {
                    row(ui, |ui| ui.label(meta("No chat matches the filter.", &p)));
                    return;
                }
                let news = &self.summaries.catchup.news;
                let chosen = &self.settings.catchup_chosen;
                egui::ScrollArea::vertical()
                    .id_salt("catchup-chat-list")
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_HEIGHT, listed.len(), |ui, range| {
                        for g in &listed[range] {
                            let key = g.path.to_string_lossy().into_owned();
                            let found = news.get(&g.path);
                            let fresh = matches!(found, Some(Ok(n)) if n.fresh);
                            // What a press would write: under "Every chat with
                            // news", each chat with some.
                            let ticked = if auto { fresh } else { chosen.contains(&key) };
                            let (rect, hit) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), ROW_HEIGHT),
                                Sense::click(),
                            );
                            if !running && hit.hovered() {
                                ui.painter()
                                    .rect_filled(rect, egui::CornerRadius::ZERO, p.surface);
                            }
                            let inner = rect.shrink2(egui::vec2(GUTTER, 0.0));
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .max_rect(inner)
                                    .layout(Layout::left_to_right(Align::Center)),
                                |ui| {
                                    // **The row is the control.** A label is
                                    // selectable text by default, and selectable
                                    // text takes the click, so the name and the
                                    // count were the two places a click did
                                    // nothing.
                                    ui.style_mut().interaction.selectable_labels = false;
                                    tick_mark(ui, ticked, &p);
                                    ui.add_space(space::TIGHT);
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let count = match found {
                                            None => "…".to_string(),
                                            Some(Err(_)) => "!".to_string(),
                                            Some(Ok(n)) if n.messages == 0 => "–".to_string(),
                                            Some(Ok(n)) => tgx_ui::thousands(n.messages as i64),
                                        };
                                        let r = ui.label(figure(count, &p));
                                        match found {
                                            Some(Err(e)) => {
                                                r.on_hover_text(e);
                                            }
                                            Some(Ok(n)) if n.first => {
                                                r.on_hover_text(format!(
                                                    "Never caught up: the last {} days",
                                                    run::FIRST_DAYS
                                                ));
                                            }
                                            Some(Ok(n)) if n.updates > 0 => {
                                                r.on_hover_text(format!(
                                                    "New in {} update{}",
                                                    n.updates,
                                                    if n.updates == 1 { "" } else { "s" }
                                                ));
                                            }
                                            _ => {}
                                        }
                                        ui.with_layout(
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                // Nothing new is quiet; it is
                                                // still listed, so it can be
                                                // ticked.
                                                let title = if fresh {
                                                    text(&g.title, &p)
                                                } else {
                                                    text(&g.title, &p).color(p.muted)
                                                };
                                                ui.add(egui::Label::new(title).truncate());
                                            },
                                        );
                                    });
                                },
                            );
                            if hit.clicked() && !running {
                                toggle = Some(key);
                            }
                        }
                    });
            });

        if let Some(key) = toggle {
            self.toggle_setting(|s| {
                // A tick by hand under "Every chat with news" turns it into
                // the ticked list, starting from what it picked.
                if s.catchup_auto {
                    s.catchup_auto = false;
                    s.catchup_chosen = with_news.clone();
                }
                if let Some(at) = s.catchup_chosen.iter().position(|k| *k == key) {
                    s.catchup_chosen.remove(at);
                } else {
                    s.catchup_chosen.push(key);
                }
            });
        }
        if let Some(on) = all_or_none {
            self.toggle_setting(|s| {
                if s.catchup_auto {
                    s.catchup_auto = false;
                    s.catchup_chosen = with_news;
                }
                s.catchup_chosen.retain(|k| !shown_keys.contains(k));
                if on {
                    s.catchup_chosen.extend(shown_keys);
                }
            });
        }
    }

    /// Show an earlier catch-up's cards.
    fn catchup_open(&mut self, path: &std::path::Path) {
        let c = &mut self.summaries.catchup;
        match run::read_record(path) {
            Some(r) => {
                c.cards = r
                    .cards
                    .into_iter()
                    .map(|(group, card)| CardView {
                        span: Some((card.from, card.to, card.messages)),
                        text: card.text.clone(),
                        state: CardState::Done(card),
                        group,
                    })
                    .collect();
                c.status = format!("Catch-up of {}, {}", r.written, r.days);
                c.shown = Some(r.path);
                c.folded = true;
                c.selected = None;
                c.took = None;
            }
            None => c.status = format!("{} could not be read", path.display()),
        }
    }

    /// The right side: the choices, then the cards.
    pub(super) fn catchup_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.catchup_look(ctx);
        // A photograph cannot click: it opens the newest catch-up of the
        // folder it was given (`TGX_SHOT_ANALYSIS`), which names nobody.
        if super::shot::path().is_some() && self.summaries.catchup.cards.is_empty() {
            if let Some(r) = self
                .summaries
                .catchup
                .records
                .first()
                .map(|r| r.path.clone())
            {
                self.catchup_open(&r);
            }
        }
        let p = self.palette;
        // **The form folds up once there is something to read.** Its six
        // lines took the top half of the page, and while a catch-up runs or
        // after it the cards are what the page is for. The header stays, says
        // what the form is set to, and opens it again.
        let has_cards = !self.summaries.catchup.cards.is_empty();
        let folded = has_cards && self.summaries.catchup.folded;
        let (flip, earlier) = band(ui, HEAD_H, &p, |ui| {
            self.catchup_header(ui, has_cards, folded)
        });
        if flip {
            self.summaries.catchup.folded = !folded;
        }
        if let Some(path) = earlier {
            self.catchup_open(&path);
        }
        let folded = has_cards && self.summaries.catchup.folded;
        let id = ui.make_persistent_id("catchup-form");
        let mut form =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, !folded);
        form.set_open(!folded);
        let unfolded = form.openness(ui.ctx()) > 0.0;
        form.show_body_unindented(ui, |ui| self.catchup_form(ui));
        let holder = self.summaries.holder.is_some();
        self.summaries_holder(ui);
        // The band's own rule closes the heading; a second one only when
        // something stands between it and the strip.
        if unfolded || holder {
            ui.add_space(space::STEP);
            rule(ui, &p);
        }
        self.catchup_cards(ui);
    }

    /// The page's heading band: Catch up; once there are cards, what the form
    /// is set to and Change, which folds or unfolds it; and the earlier
    /// catch-ups. Whether Change was pressed, and an earlier one to open.
    fn catchup_header(
        &self,
        ui: &mut Ui,
        has_cards: bool,
        folded: bool,
    ) -> (bool, Option<PathBuf>) {
        let p = self.palette;
        let mut flip = false;
        let mut open = None;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if has_cards {
                let what = if folded { "Change" } else { "Hide" };
                let word = action(ui, text(what, &p), true);
                let mark = disclosure(ui, folded, &p);
                flip |= word.clicked() || mark.clicked();
                ui.add_space(space::BREAK);
            }
            open = self.catchup_earlier(ui);
            ui.add_space(space::STEP);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(super::chrome::heading("Catch up", &p));
                if has_cards && folded {
                    ui.add_space(space::STEP);
                    let choices = ui
                        .add(
                            egui::Label::new(meta(self.catchup_choices(), &p))
                                .truncate()
                                .sense(Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    flip |= choices.clicked();
                }
            });
        });
        (flip, open)
    }

    /// The earlier catch-ups, newest first, as a menu showing the one open.
    /// The one picked, if any.
    fn catchup_earlier(&self, ui: &mut Ui) -> Option<PathBuf> {
        let p = self.palette;
        let c = &self.summaries.catchup;
        if c.records.is_empty() {
            return None;
        }
        let shown = c
            .shown
            .as_ref()
            .and_then(|s| c.records.iter().find(|r| r.path == *s));
        let label = shown.map_or_else(
            || "Earlier catch-ups".to_string(),
            |r| r.written.to_string(),
        );
        let mut open = None;
        ui.add_enabled_ui(!c.running(), |ui| {
            egui::ComboBox::from_id_salt("catchup-earlier")
                .selected_text(text(label, &p))
                .width(EARLIER_W)
                .show_ui(ui, |ui| {
                    for r in &c.records {
                        let on = c.shown.as_ref() == Some(&r.path);
                        let line = format!("{} · {} chats", r.written, r.cards.len());
                        if ui
                            .selectable_label(on, text(line, &p))
                            .on_hover_text(format!("Read {}", r.days))
                            .clicked()
                        {
                            open = Some(r.path.clone());
                        }
                    }
                });
        });
        open
    }

    /// What the form is set to, in one line, for the header while it is folded.
    fn catchup_choices(&self) -> String {
        let c = &self.summaries.catchup;
        let days = match c.chosen_days() {
            Ok(Days::Between(from, to)) if from == to => from.format("%-d %b").to_string(),
            Ok(Days::Between(from, to)) => {
                format!("{} – {}", from.format("%-d %b"), to.format("%-d %b"))
            }
            Ok(d) => DAYS
                .iter()
                .find(|(_, x)| *x == d)
                .map(|(l, _)| l.to_string())
                .unwrap_or_default(),
            Err(e) => e,
        };
        let chats = if self.settings.catchup_auto {
            "every chat with news"
        } else {
            "ticked chats"
        };
        let forums = if self.settings.catchup_by_topic {
            "topic by topic"
        } else {
            "whole chats"
        };
        format!("{days} · {chats} · {forums}")
    }

    /// The form: the days, which chats, how forums are read, and what a press
    /// would cost.
    fn catchup_form(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let running = self.summaries.catchup.running();
        let mut remember_days: Option<Days> = None;
        let mut by_topic: Option<bool> = None;
        let mut auto: Option<bool> = None;

        ui.add_space(space::STEP);
        form_row(ui, "Days", &p, |ui| {
            let c = &self.summaries.catchup;
            let at = DAYS
                .iter()
                .position(|(_, d)| *d == c.days)
                .unwrap_or(DAYS.len());
            let labels: Vec<&str> = DAYS
                .iter()
                .map(|(l, _)| *l)
                .chain(std::iter::once("Dates"))
                .collect();
            if let Some(i) = segmented(ui, &labels, at, &p) {
                if !running {
                    let days = match DAYS.get(i) {
                        Some((_, d)) => *d,
                        None => {
                            let c = &mut self.summaries.catchup;
                            let today = run::today();
                            if c.from.trim().is_empty() {
                                c.from = (today - chrono::Duration::days(6)).to_string();
                                c.to = today.to_string();
                            }
                            Days::Between(today, today)
                        }
                    };
                    remember_days = Some(days);
                }
            }
        });
        if matches!(self.summaries.catchup.days, Days::Between(..)) {
            ui.add_space(space::TIGHT);
            let c = &mut self.summaries.catchup;
            dates_row(ui, &mut c.from, &mut c.to, !running, &p);
        }
        let counts = matches!(self.summaries.catchup.days, Days::Hours24 | Days::Last(_));
        let flip_from_last = super::summaries::from_last_row(
            ui,
            self.settings.summary_from_last,
            counts && !running,
            &p,
        );
        ui.add_space(space::TIGHT);
        form_row(ui, "Chats", &p, |ui| {
            let at = usize::from(!self.settings.catchup_auto);
            if let Some(i) = segmented(ui, &["Every chat with news", "Ticked"], at, &p) {
                if !running {
                    auto = Some(i == 0);
                }
            }
        });
        // A line of its own: beside Chats it was wider than the page at the
        // window's floor, a strip of segments cannot wrap, and the cards
        // below took the overflow as their width.
        ui.add_space(space::TIGHT);
        form_row(ui, "Forums", &p, |ui| {
            let at = usize::from(self.settings.catchup_by_topic);
            if let Some(i) = segmented(ui, &["Whole chat", "Topic by topic"], at, &p) {
                if !running {
                    by_topic = Some(i == 1);
                }
            }
        });
        ui.add_space(space::TIGHT);
        let line = self.catchup_line();
        block(ui, |ui| ui.label(figure(line, &p)));

        if let Some(days) = remember_days {
            self.summaries.catchup.days = days;
            if !matches!(days, Days::Between(..)) {
                self.toggle_setting(|s| s.catchup_days = days.code());
            }
        }
        if let Some(on) = auto {
            self.toggle_setting(|s| s.catchup_auto = on);
        }
        if flip_from_last {
            self.flip_from_last();
        }
        if let Some(on) = by_topic {
            self.toggle_setting(|s| s.catchup_by_topic = on);
        }
    }

    /// What a press would do: how many chats, how many messages.
    fn catchup_line(&self) -> String {
        let c = &self.summaries.catchup;
        if let Err(e) = c.chosen_days() {
            return e;
        }
        if self.catchup_records().is_none() {
            return "Choose the Analysis folder first.".into();
        }
        let targets = self.catchup_targets();
        let messages: usize = targets
            .iter()
            .filter_map(|g| c.news.get(&g.path)?.as_ref().ok())
            .map(|n| n.messages)
            .sum();
        let counting = c.survey.is_some();
        let span = self.catchup_span();
        match targets.len() {
            0 if counting => "Counting what is new…".into(),
            0 if self.settings.catchup_auto => format!("Nothing new {span}."),
            0 => "Tick the chats to catch up on, on the left.".into(),
            n => format!(
                "{} · {} message{} {}{}",
                match n {
                    1 => "One chat".to_string(),
                    n => format!("{n} chats"),
                },
                tgx_ui::thousands(messages as i64),
                if messages == 1 { "" } else { "s" },
                span,
                if counting { " · still counting" } else { "" }
            ),
        }
    }

    /// The days in words, and for "since the last catch-up" how long ago that
    /// was: the newest of the chats', since each chat keeps its own.
    fn catchup_span(&self) -> String {
        let c = &self.summaries.catchup;
        let from_last = self.settings.summary_from_last;
        if c.days != Days::SinceLast {
            return run::describe(c.days, from_last);
        }
        let last = c
            .news
            .values()
            .filter_map(|n| n.as_ref().ok()?.caught_up)
            .max();
        match last {
            Some(t) => format!(
                "since the last catch-up, {}",
                run::ago(t, chrono::Local::now().timestamp())
            ),
            None if c.survey.is_none() && !c.news.is_empty() => format!(
                "in the last {} days: there has been no catch-up yet",
                run::FIRST_DAYS
            ),
            None => run::describe(c.days, from_last),
        }
    }

    /// The summaries: **a strip of the chats, and one chat's summary under
    /// it.** A click on a chip, Previous and Next, or the left and right arrow
    /// keys move through them.
    ///
    /// They were all one column of cards, busiest first, so finding a chat
    /// meant scrolling past every other chat's summary. The strip is A to Z
    /// in either alphabet and follows the filter on the left; a chat with
    /// nothing new is one line under it, not a stop on the way through.
    fn catchup_cards(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let mut jump = std::mem::take(&mut self.summaries.catchup.strip_jump);
        let c = &self.summaries.catchup;
        if c.cards.is_empty() {
            hint(
                ui,
                "Press Catch up: each chat gets a short summary of what is new in it. \
                 A Gemma already running is used; otherwise one is started from \
                 Settings › Summaries.",
                &p,
            );
            return;
        }
        let running = c.running();
        let needle = self.summaries.filter.clone();
        let quiet = |card: &CardView| card.state == CardState::Failed(run::NOTHING_NEW.into());
        let wanted = |card: &CardView| crate::fold::matches(&card.group.title, &needle);
        let by_title = |i: &usize| crate::fold::fold(&c.cards[*i].group.title);
        let mut strip: Vec<usize> = (0..c.cards.len())
            .filter(|&i| !quiet(&c.cards[i]) && wanted(&c.cards[i]))
            .collect();
        strip.sort_by_cached_key(by_title);
        let mut nothing: Vec<usize> = (0..c.cards.len())
            .filter(|&i| quiet(&c.cards[i]) && wanted(&c.cards[i]))
            .collect();
        nothing.sort_by_cached_key(by_title);
        let nothing: Vec<String> = nothing
            .into_iter()
            .map(|i| c.cards[i].group.title.clone())
            .collect();

        // The open chat: the one chosen, else the one being written, else the
        // first.
        let busy = |card: &CardView| {
            matches!(
                card.state,
                CardState::Reading | CardState::Read | CardState::Writing
            )
        };
        let mut at = c
            .selected
            .as_ref()
            .and_then(|s| strip.iter().position(|&i| c.cards[i].group.path == *s))
            .or_else(|| {
                running
                    .then(|| strip.iter().position(|&i| busy(&c.cards[i])))
                    .flatten()
            })
            .unwrap_or(0);
        let mut moved = false;
        // The arrows, unless a field has the keyboard: there they move the
        // caret.
        if !strip.is_empty() && !ui.ctx().wants_keyboard_input() {
            let (left, right) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight),
                )
            });
            if left && at > 0 {
                at -= 1;
                moved = true;
            }
            if right && at + 1 < strip.len() {
                at += 1;
                moved = true;
            }
            jump |= moved;
        }

        let mut clicked: Option<usize> = None;
        let mut step: Option<usize> = None;
        // The strip is the second band, level with the filter beside it.
        band(ui, BAR_H, &p, |ui| {
            if strip.is_empty() {
                let why = if c.cards.iter().all(quiet) {
                    "Nothing new in any of these chats."
                } else if running {
                    "Waiting for the first summary…"
                } else {
                    "No summary matches the filter."
                };
                ui.label(meta(why, &p));
                return;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, "Next", at + 1 < strip.len(), false, &p)
                    .on_hover_text("Right arrow")
                    .clicked()
                {
                    step = Some(at + 1);
                }
                if button(ui, "Previous", at > 0, false, &p)
                    .on_hover_text("Left arrow")
                    .clicked()
                {
                    step = Some(at - 1);
                }
                ui.label(figure(format!("{} of {}", at + 1, strip.len()), &p));
                ui.add_space(space::STEP);
                let w = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(w, CHIP_H),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        // **No bar under the strip**, so it is one line high
                        // whether or not it overflows. The wheel scrolls it
                        // sideways, a drag pulls it, and the open chip is
                        // always brought into view.
                        ui.style_mut().always_scroll_the_only_direction = true;
                        egui::ScrollArea::horizontal()
                            .id_salt("catchup-strip")
                            .auto_shrink([false, true])
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                            )
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = space::TIGHT;
                                    for (n, &i) in strip.iter().enumerate() {
                                        let card = &c.cards[i];
                                        let hit = chip(
                                            ui,
                                            &card.group.title,
                                            &chip_note(card),
                                            n == at,
                                            &p,
                                        );
                                        if jump && n == at {
                                            hit.scroll_to_me(Some(Align::Center));
                                        }
                                        if hit.clicked() {
                                            clicked = Some(n);
                                        }
                                    }
                                });
                            });
                    },
                );
            });
        });
        // Under the strip, not above it: the strip is level with the filter
        // beside it, and a line above would push it out of step.
        if let Some((fraction, line)) = c.bar() {
            ui.add_space(space::STEP);
            block(ui, |ui| {
                progress_bar(ui, Some(fraction), &p);
                ui.add_space(space::TIGHT);
                ui.label(meta(line, &p));
            });
        }
        if !nothing.is_empty() {
            ui.add_space(space::TIGHT);
            block(ui, |ui| {
                ui.add(
                    egui::Label::new(meta(format!("Nothing new: {}", nothing.join(", ")), &p))
                        .wrap(),
                )
            });
        }

        // What the strip asked for, kept by path so a re-sort cannot move it.
        if let Some(n) = clicked.or(step).or(moved.then_some(at)) {
            let path = c.cards[strip[n]].group.path.clone();
            let c = &mut self.summaries.catchup;
            c.selected = Some(path);
            c.strip_jump = step.is_some();
            at = n;
        }
        let Some(&open_card) = strip.get(at) else {
            return;
        };
        let mut card = self.summaries.catchup.cards[open_card].clone();
        // A catch-up started before Analysis had opened this database has no
        // chat for it; see `known_chat_id`.
        if card.group.chat_id.is_none() {
            card.group.chat_id = self.known_chat_id(&card.group.path);
        }
        let mut cite: Option<(Group, i64)> = None;
        let mut ask: Option<(Group, Card)> = None;
        let mut open: Option<PathBuf> = None;
        let mut copy = false;
        ui.add_space(space::STEP);
        // One scroll position per chat: moving to the next starts at its top.
        egui::ScrollArea::vertical()
            .id_salt(("catchup-card", &card.group.path))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                super::summaries::card(ui, &p, |ui| {
                    // The name, and on the same line at the right its days.
                    row(ui, |ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if let Some(line) = card_meta(&card) {
                                ui.label(meta(line, &p));
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(
                                    egui::Label::new(super::chrome::heading(&card.group.title, &p))
                                        .truncate(),
                                );
                            });
                        });
                    });
                    ui.add_space(space::STEP);
                    let linkable = card.group.chat_id.is_some();
                    match &card.state {
                        CardState::Waiting => {
                            row(ui, |ui| ui.label(meta("Waiting its turn", &p)));
                        }
                        CardState::Reading => {
                            row(ui, |ui| ui.label(meta("Reading the days…", &p)));
                        }
                        CardState::Read => {
                            row(ui, |ui| ui.label(meta("Read; waiting for the model", &p)));
                        }
                        CardState::Failed(why) => {
                            row(ui, |ui| ui.label(meta(why, &p)));
                        }
                        CardState::Writing | CardState::Done(_) => {
                            if card.text.is_empty() {
                                row(ui, |ui| ui.label(meta("Writing…", &p)));
                            } else if let Some(id) = markdown(ui, &card.text, linkable, &p) {
                                cite = Some((card.group.clone(), id));
                            }
                        }
                    }
                    if let CardState::Done(done) = &card.state {
                        ui.add_space(space::STEP);
                        row(ui, |ui| {
                            if button(ui, "Copy", true, false, &p).clicked() {
                                copy = true;
                            }
                            if button(ui, "Ask about these days", !running, false, &p).clicked() {
                                ask = Some((card.group.clone(), done.clone()));
                            }
                            if button(ui, "Open file", true, false, &p).clicked() {
                                open = Some(done.path.clone());
                            }
                        });
                    }
                });
                ui.add_space(space::BREAK);
            });
        if let Some((g, id)) = cite {
            if let Some(chat_id) = g.chat_id {
                self.summaries.cite = Some((
                    super::analysis::Read {
                        path: g.path,
                        chat_id,
                        title: g.title,
                    },
                    id,
                ));
            }
        }
        if copy {
            // The card's text has no heading of its own: the chat's name and
            // days go first, so a paste says what it is about.
            let head = match card_meta(&card) {
                Some(line) => format!("{} · {line}", card.group.title),
                None => card.group.title.clone(),
            };
            ui.ctx()
                .copy_text(format!("{head}\n\n{}", card.text.trim()));
            self.summaries.catchup.status = format!("Copied the summary of {}", card.group.title);
        }
        if let Some(path) = open {
            if let Err(e) = tga_run::open::open(&path) {
                self.summaries.catchup.status = e;
            }
        }
        if let Some((g, card)) = ask {
            self.catchup_ask(&g, &card, ui.ctx());
        }
    }

    /// The One chat page, on this card's chat and days, its summary shown.
    fn catchup_ask(&mut self, group: &Group, card: &Card, ctx: &egui::Context) {
        let Some(entry) = self
            .analysis
            .state
            .entries
            .iter()
            .find(|e| e.target.path() == group.path)
            .cloned()
        else {
            self.summaries.catchup.status = format!("{} is no longer in Analysis", group.title);
            return;
        };
        let s = &mut self.summaries;
        s.mode = Mode::OneChat;
        // A card of a chat's last 24 hours asks about the same hours.
        s.span = if card.after.is_some() {
            Span::Hours24
        } else {
            Span::Dates
        };
        s.from = card.from.to_string();
        s.to = card.to.to_string();
        s.length = tga_run::tga_sum::Length::Brief;
        s.scope = if self.settings.catchup_by_topic {
            super::summaries::Scope::ByTopic
        } else {
            super::summaries::Scope::Whole
        };
        self.summaries_pick(&entry, ctx);
    }
}

/// A card's second line: the days and how many messages.
fn card_meta(card: &CardView) -> Option<String> {
    let (from, to, n) = card.span?;
    let days = if from == to {
        from.format("%-d %b").to_string()
    } else {
        format!("{} – {}", from.format("%-d %b"), to.format("%-d %b"))
    };
    Some(format!(
        "{days} · {} message{}",
        tgx_ui::thousands(n as i64),
        if n == 1 { "" } else { "s" }
    ))
}

/// What a chip says after the chat's name: how many messages, or where the
/// run has got to.
fn chip_note(card: &CardView) -> String {
    match &card.state {
        CardState::Waiting => String::new(),
        CardState::Reading | CardState::Read | CardState::Writing => "…".into(),
        CardState::Failed(_) => "!".into(),
        CardState::Done(_) => card
            .span
            .map(|(_, _, n)| tgx_ui::thousands(n as i64))
            .unwrap_or_default(),
    }
}

/// The widest a chip's name is drawn before it is cut with `…`.
const CHIP_NAME_W: f32 = 200.0;
/// How tall a chip is: a button's height.
const CHIP_H: f32 = 30.0;

/// One chat on the strip: its name and `note` in a hairline box, filled and
/// edged in ink when it is the one open. **Painted, not a button**, so the
/// name can be cut at a width and the note set in the mono beside it.
fn chip(ui: &mut Ui, name: &str, note: &str, open: bool, p: &Palette) -> egui::Response {
    let ink = if open { p.fg } else { p.muted };
    let name = egui::WidgetText::from(text(name, p).color(ink)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        CHIP_NAME_W,
        egui::TextStyle::Body,
    );
    let note = (!note.is_empty()).then(|| {
        egui::WidgetText::from(figure(note, p).color(p.muted)).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        )
    });
    let pad = space::STEP * 0.75;
    let gap = space::TIGHT;
    let width = pad * 2.0 + name.size().x + note.as_ref().map_or(0.0, |n| gap + n.size().x);
    let (rect, hit) = ui.allocate_exact_size(egui::vec2(width, CHIP_H), Sense::click());
    let edge = if open {
        p.fg
    } else if hit.hovered() {
        p.hairline
    } else {
        p.rule
    };
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::ZERO,
        if open { p.surface } else { p.bg },
    );
    painter.rect_stroke(
        rect,
        egui::CornerRadius::ZERO,
        egui::Stroke::new(1.0_f32, edge),
        egui::StrokeKind::Inside,
    );
    let y = |h: f32| rect.center().y - h / 2.0;
    let name_w = name.size().x;
    painter.galley(egui::pos2(rect.left() + pad, y(name.size().y)), name, ink);
    if let Some(note) = note {
        let x = rect.left() + pad + name_w + gap;
        painter.galley(egui::pos2(x, y(note.size().y)), note, p.muted);
    }
    hit.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The empty page, before Analysis has a folder.
pub(super) fn no_folder(ui: &mut Ui, p: &tgx_ui::tokens::Palette) {
    EmptyState::new(
        "No exports folder yet",
        Some("Choose the folder your exports are in, in Analysis.".into()),
    )
    .show(ui, p, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tga_run::tga_sum::Event;

    fn waiting(n: usize) -> Vec<CardView> {
        (0..n)
            .map(|i| CardView {
                group: Group {
                    path: PathBuf::from(format!("{i}.sqlite")),
                    chat_id: None,
                    title: format!("Chat {i}"),
                    home: PathBuf::new(),
                },
                state: CardState::Waiting,
                text: String::new(),
                span: None,
            })
            .collect()
    }

    /// The bar moves with each chat finished, and within the one being
    /// written with its parts; a stop leaves it where it stopped, and the
    /// clock stays with the cards.
    #[test]
    fn the_bar_follows_the_chats_and_the_parts_and_keeps_the_time() {
        let mut c = CatchupState {
            cards: waiting(4),
            started: Some(std::time::Instant::now()),
            ..Default::default()
        };
        c.apply(Progress::Group(0, Step::Done(Err(run::NOTHING_NEW.into()))));
        c.apply(Progress::Group(1, Step::Writing));
        assert_eq!(c.finished(), (1, 4));
        assert_eq!(c.fraction(), 0.25);
        // Two parts of three: the summary after them is a fourth.
        c.apply(Progress::Group(
            1,
            Step::Event(Event::Part {
                done: 2,
                of: 3,
                cached: false,
                secs: 1.0,
                timings: None,
            }),
        ));
        assert_eq!(c.fraction(), (1.0 + 0.5) / 4.0);
        c.apply(Progress::Group(
            1,
            Step::Done(Err("the server failed".into())),
        ));
        assert_eq!(c.fraction(), 0.5);

        c.apply(Progress::Done(Err("cancelled".into())));
        assert_eq!(c.finished(), (2, 4), "the stopped chats are not finished");
        let (fraction, line) = c.bar().expect("a finished run keeps its bar");
        assert_eq!(fraction, 0.5);
        assert!(line.starts_with("2 of 4 chats · took "), "{line}");
        assert!(c.started.is_none());
    }
}
