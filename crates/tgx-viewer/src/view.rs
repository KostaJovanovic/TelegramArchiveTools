//! The Viewer: one chat's database, every message in it, read-only.
//!
//! **A component, not a window.** TelegramTools puts it in its body, behind
//! Analysis; TelegramViewer.exe puts it beside a list of the databases in its
//! folder. Each gives it a [`Host`] — where work runs, where the status line
//! and the warnings go — and hands every [`Reply`] back to [`Viewer::apply`].
//! It was methods on TelegramTools' `Shell` until the second program needed
//! the same screen; one copy is the point.
//!
//! **Nothing here touches JSON.** The rows arrive from [`crate::data::load`]
//! already flattened — a date, a sender, a first line, the flags — on the
//! blocking pool, so a chat of a hundred thousand messages costs a frame what
//! nine do: the list is virtualised, and asks only for what is on screen.
//!
//! **What an archive holds that Telegram does not is the point of looking**,
//! so the markers are the loudest thing on a row: a message gone from
//! Telegram says so in the accent with the date it was noticed, an edited one
//! says how many times it changed, and one put back from an old export says
//! it was recovered. The detail pane shows every earlier version in full.
//!
//! The text on screen is other people's conversation. **None of it goes to the
//! log or the transcript**: failures name files and errors, the status line
//! names counts.

use crate::bubbles;
use crate::data::{Database, Detail, Loaded, Media, Row, Topic};
use crate::jobs::{Job, Reply};
use crate::search::Found;
use eframe::egui::{self, Align, Layout, Sense, Ui};
use std::collections::{HashMap, HashSet};
use tgx_ui::components::{
    action, badge, block, button, eyebrow, field, figure, human_bytes, meta, row, rule, segmented,
    text, thousands, tick_box, EmptyState, Tone, GUTTER,
};
use tgx_ui::tokens::{space, Palette};

/// A row of the every-database results list: one line, date, chat, excerpt.
const ROW_HEIGHT: f32 = 30.0;

/// Items measured either side of a jump target before the list scrolls to
/// it, so the guessed heights above it do not land the target off screen.
const JUMP_MEASURE: usize = 60;

/// The detail pane. Wide enough for a paragraph to read as one, narrow enough
/// that at the window's 900pt floor the list keeps the larger half.
const DETAIL_W: f32 = 300.0;

/// The search box. A word or two goes in it, like the chat filter's.
const SEARCH_W: f32 = 220.0;

/// The sender column. A name, clipped; the full one is in the detail pane.
const FROM_W: f32 = 110.0;

/// Pictures kept as textures at once. At most 720px a side, so this bounds
/// the viewer's video memory at a little under 300 MB in the worst case and
/// far less in practice.
const PICTURES_KEPT: usize = 160;
/// Pictures being read and decoded at once. Past it, the rest wait for the
/// next frame — so a fast scroll asks for what it stops on, not for every
/// photo it flew past.
const PICTURES_IN_FLIGHT: usize = 6;

/// What the window the Viewer sits in does for it.
pub trait Host {
    /// Do `job` off the window's thread; its replies come back to
    /// [`Viewer::apply`], and each must wake the window.
    fn run(&mut self, job: Job);
    /// The status line. Counts and file names, never message text.
    fn status(&mut self, line: String);
    /// A failure, kept where the window keeps warnings. The status line has
    /// already said it.
    fn warn(&mut self, line: String);
}

/// What the head row offers besides the Viewer's own controls, and what the
/// empty Viewer says.
#[derive(Debug, Clone, Copy, Default)]
pub struct Head<'a> {
    /// The way back, by the name of where it goes; `None` draws none.
    pub back: Option<&'a str>,
    /// Offer Export as folder — TelegramTools can write one, the Viewer alone
    /// cannot.
    pub unpack: bool,
    /// The hint under "No chat open": where chats are opened from.
    pub none_open: &'a str,
}

/// Head-row buttons that are the window's to act on, pressed this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Asked {
    pub back: bool,
    pub unpack: bool,
}

/// Which messages the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    /// Gone from Telegram, kept here.
    Deleted,
    /// Changed since first stored — an edit, or reactions and counters moving.
    Edited,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::All, Filter::Deleted, Filter::Edited];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Deleted => "Deleted",
            Filter::Edited => "Edited",
        }
    }

    pub fn keeps(self, r: &Row) -> bool {
        match self {
            Filter::All => true,
            Filter::Deleted => r.deleted.is_some(),
            Filter::Edited => r.edited(),
        }
    }
}

/// Everything the Viewer remembers between frames.
///
/// **The rows are a cache of the database**, and `shown` a cache of the filter
/// over them — rebuilt when either changes and nowhere else, for the reason
/// the chat list caches its rows: the list asks for its visible range on every
/// frame, and re-filtering a hundred thousand rows inside that callback is
/// work done sixty times a second for an answer that changed once.
#[derive(Default)]
pub struct Viewer {
    /// The folder the databases are in. The window keeps it current;
    /// a change means listing again.
    pub folder: String,
    /// The databases in `folder`, by title.
    pub databases: Vec<Database>,
    /// Which folder `databases` describes. `None` until it is first asked; a
    /// different folder from `folder` means ask again.
    pub listed_for: Option<String>,
    pub listing: bool,

    /// The chat on screen.
    pub open: Option<Database>,
    pub loading: bool,
    /// Why the open chat could not be read. **Shown in the list's place**:
    /// with only the status line saying it, an unreadable database looked
    /// like one that "holds no messages", which invites exporting it again
    /// over a good file.
    pub load_error: Option<String>,
    pub rows: Vec<Row>,
    /// Message id to its index in `rows`.
    pub index: HashMap<i64, usize>,
    /// Indices into `rows` the filter keeps, in order.
    pub shown: Vec<usize>,
    pub filter: Filter,
    /// `(deleted, edited, recovered)` across `rows`, counted once per load.
    pub counts: (usize, usize, usize),

    /// The message in the detail pane, and what the database says of it.
    pub selected: Option<i64>,
    pub detail: Option<Box<Detail>>,
    /// Why the selected message could not be read, in the pane's place —
    /// which otherwise said "Reading…" for good.
    pub detail_error: Option<String>,
    /// A row to bring into view on the next frame, as an index into `shown`.
    pub scroll_to: Option<usize>,
    /// Select this message once the chat being opened has loaded — a hit in
    /// another database, clicked.
    pub select_after_load: Option<i64>,

    /// The search box's buffer.
    pub query: String,
    /// The search box takes the focus when next drawn: Ctrl+F.
    pub focus_search: bool,
    /// Search every database rather than the one open.
    pub everywhere: bool,
    /// The question in flight: the text and the chat, `None` for every
    /// database. An answer to anything else is dropped.
    pub asked: Option<(String, Option<i64>)>,
    /// This chat's hits, oldest first, and which one Prev / Next is on.
    pub hits: Vec<i64>,
    pub hit_set: HashSet<i64>,
    pub hit_at: Option<usize>,
    /// The last search of every database, and its hits as `(chat, hit)`
    /// pairs into it — the list `show_rows` pages through.
    pub found: Option<Box<Found>>,
    pub found_rows: Vec<(usize, usize)>,
    /// The list is showing `found` rather than the open chat.
    pub showing_found: bool,
    /// Export as folder is writing; its button is off until it answers.
    pub unpacking: bool,

    /// The forum topics the open chat's messages are filed under; more than
    /// one only for a forum. `topic` is the one shown, `None` for all.
    pub topics: Vec<Topic>,
    pub topic: Option<i64>,
    pub topic_names: HashMap<i64, String>,

    /// Everyone who wrote in the open chat, `(from_id, name, messages)`, most
    /// messages first — what the sender picker offers. By id, because two
    /// people can share a display name.
    pub senders: Vec<(String, String, usize)>,
    /// The one sender shown, `None` for everyone.
    pub sender: Option<String>,
    /// The Go to date box's buffer, `YYYY-MM-DD`.
    pub go_to: String,

    /// What the list draws — day separators and messages in runs — and, for
    /// each entry of `shown`, which item it is.
    pub items: Vec<bubbles::Item>,
    pub item_of_shown: Vec<usize>,
    /// Each item's height and where it starts. Guessed until it is drawn,
    /// then measured; `offsets` has one more entry than `items`, the total.
    ///
    /// **Kept across a change of width.** Every item on screen is drawn and
    /// measured on every frame anyway, so a height from another width is
    /// corrected the moment it matters, and off screen it is as good a guess
    /// as [`bubbles::estimate`]'s. Re-guessing them all on a width change read
    /// every message's text on the window's thread — on the first click,
    /// which opens the detail pane, and on every frame of a resize drag — and
    /// threw away every measurement, so the list jumped under the reader.
    pub heights: Vec<f32>,
    pub offsets: Vec<f32>,
    /// Where the list was scrolled to last frame, and how far to move it this
    /// frame so that an item above the view growing on measurement does not
    /// push what is being read down the screen.
    pub last_offset: f32,
    pub scroll_adjust: f32,

    /// The pictures asked for, by message id, and the frame each was last
    /// painted on — what decides which go when there are too many.
    pub pictures: HashMap<i64, bubbles::Pic>,
    pub pic_used: HashMap<i64, u64>,
    pub frame: u64,
    /// The open chat's profile pictures, by sender. Read whole when the chat
    /// opens: one small picture a person, a few hundred at most.
    pub userpics: HashMap<String, bubbles::Pic>,

    /// Open this database once the folder is listed, rather than the first —
    /// a `.sqlite` handed to TelegramViewer on its command line.
    pub want_path: Option<std::path::PathBuf>,
    /// Open the first database whose title contains this, rather than the
    /// first one — for a photograph of the window (TelegramTools' `TGX_SHOT_DB`).
    pub want_title: Option<String>,
    /// Once a chat loads, select the first message whose media label contains
    /// this (`TGX_SHOT_FIND`), so a photograph is of the thing being checked.
    pub want_media: Option<String>,
}

impl Viewer {
    fn rebuild_shown(&mut self) {
        let filter = self.filter;
        let topic = self.topic;
        let sender = self.sender.as_deref();
        self.shown = (0..self.rows.len())
            .filter(|&i| {
                let r = &self.rows[i];
                filter.keeps(r)
                    && topic.is_none_or(|t| r.topic_id == t)
                    && sender.is_none_or(|s| r.from_id == s)
            })
            .collect();
        let (items, item_of) = bubbles::build(&self.rows, &self.shown);
        self.items = items;
        self.item_of_shown = item_of;
        // Guessed afresh on the next frame, at that frame's width.
        self.heights.clear();
        self.offsets.clear();
    }

    /// Several topics, and all of them on screen: each bubble names its own.
    fn mixing_topics(&self) -> bool {
        self.topics.len() > 1 && self.topic.is_none()
    }

    fn set_hits(&mut self, hits: Vec<i64>) {
        self.hit_set = hits.iter().copied().collect();
        self.hits = hits;
        self.hit_at = None;
    }

    /// Who wrote in `rows`, most messages first; a tie goes by name.
    fn senders_of(rows: &[Row]) -> Vec<(String, String, usize)> {
        let mut by_id: HashMap<&str, (&str, usize)> = HashMap::new();
        for r in rows.iter().filter(|r| !r.from_id.is_empty()) {
            let entry = by_id.entry(&r.from_id).or_insert((&r.from, 0));
            entry.1 += 1;
        }
        let mut out: Vec<(String, String, usize)> = by_id
            .into_iter()
            .map(|(id, (name, n))| {
                let name = if name.is_empty() { id } else { name };
                (id.to_string(), name.to_string(), n)
            })
            .collect();
        out.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
        out
    }

    /// The first message shown on or after `day` (`YYYY-MM-DD`), as an index
    /// into `shown` — or the last one, when the chat ends before that day.
    fn first_on_or_after(&self, day: &str) -> Option<usize> {
        if self.shown.is_empty() {
            return None;
        }
        // `day` sorts as text because it is zero-padded ISO, and `shown` is
        // in date order because the rows are.
        let at = self
            .shown
            .partition_point(|&i| self.rows[i].day.as_str() < day);
        Some(at.min(self.shown.len() - 1))
    }

    pub fn is_open(&self, path: &std::path::Path, chat_id: i64) -> bool {
        self.open
            .as_ref()
            .is_some_and(|d| d.path == path && d.chat_id == chat_id)
    }

    /// Something to wait for before a photograph: a database or a picture
    /// still being read.
    pub fn busy(&self) -> bool {
        self.listing
            || self.loading
            || self
                .pictures
                .values()
                .chain(self.userpics.values())
                .any(|p| matches!(p, bubbles::Pic::Loading | bubbles::Pic::Decoded(_)))
    }

    // -- painting ------------------------------------------------------------

    /// **Head, foot, detail, then the list — as panels**, the chat list's
    /// arrangement and for its reason: each measures itself and the list gets
    /// exactly what is left, at any window size.
    pub fn panel(&mut self, ui: &mut Ui, p: &Palette, host: &mut dyn Host, head: Head) -> Asked {
        // The folder can change under the view; the list of databases is
        // asked for again when it does.
        if self.listed_for.as_deref() != Some(self.folder.as_str()) && !self.listing {
            self.list_databases(host);
        }
        let bare = egui::Frame::NONE.fill(p.bg);
        let mut asked = Asked::default();

        egui::TopBottomPanel::top("viewer-head")
            .frame(bare)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.add_space(space::STEP);
                asked = self.picker_row(ui, p, host, head);
                ui.add_space(space::TIGHT);
                self.search_row(ui, p, host);
                ui.add_space(space::STEP);
                rule(ui, p);
            });

        egui::TopBottomPanel::bottom("viewer-foot")
            .frame(bare)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                rule(ui, p);
                ui.add_space(space::TIGHT);
                row(ui, |ui| ui.label(figure(self.summary(), p)));
                ui.add_space(space::TIGHT);
            });

        // **Only once something is selected.** Empty, it said "no message
        // selected" in a third of the window at the minimum size — the
        // chat's width, spent on telling the user they had not clicked yet.
        if self.open.is_some() && !self.showing_found && self.selected.is_some() {
            egui::SidePanel::right("viewer-detail")
                .frame(bare)
                .resizable(false)
                .exact_width(DETAIL_W)
                .show_inside(ui, |ui| self.detail_pane(ui, p, host));
        }

        egui::CentralPanel::default()
            .frame(bare)
            .show_inside(ui, |ui| {
                if self.showing_found {
                    self.found_list(ui, p, host);
                } else {
                    self.rows_list(ui, p, host, head.none_open);
                }
            });
        asked
    }

    /// The way back, the chat's name, the topic picker, Reload and — where
    /// the window can write one — Export as folder.
    fn picker_row(&mut self, ui: &mut Ui, p: &Palette, host: &mut dyn Host, head: Head) -> Asked {
        let mut asked = Asked::default();
        let mut topic = None;
        let mut sender = None;
        let mut go_to = false;
        let mut refresh = false;
        row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            if let Some(back) = head.back {
                if action(ui, text(format!("\u{2190} {back}"), p), true).clicked() {
                    asked.back = true;
                }
            }
            let current = match &self.open {
                Some(db) => db.title.clone(),
                None => "No chat open".into(),
            };
            ui.add(egui::Label::new(text(current, p)).truncate());
            // **Only a forum has topics to pick between**, so only a forum
            // gets the picker; for any other chat it would offer one choice.
            if self.topics.len() > 1 {
                ui.label(meta("Topic", p));
                let current = match self.topic {
                    None => "All topics".to_string(),
                    Some(id) => self.topic_names.get(&id).cloned().unwrap_or_default(),
                };
                egui::ComboBox::from_id_salt("viewer-topic")
                    .selected_text(text(current, p))
                    .width(170.0)
                    .truncate()
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.topic.is_none(), text("All topics", p))
                            .clicked()
                        {
                            topic = Some(None);
                        }
                        for t in &self.topics {
                            let on = self.topic == Some(t.id);
                            if ui.selectable_label(on, text(&t.title, p)).clicked() {
                                topic = Some(Some(t.id));
                            }
                        }
                    });
            }
            // **One person's messages**, most prolific first. Only where
            // there is more than one person to choose between.
            if self.senders.len() > 1 {
                ui.label(meta("From", p));
                let current = match &self.sender {
                    None => "Everyone".to_string(),
                    Some(id) => self
                        .senders
                        .iter()
                        .find(|s| &s.0 == id)
                        .map(|s| s.1.clone())
                        .unwrap_or_default(),
                };
                egui::ComboBox::from_id_salt("viewer-sender")
                    .selected_text(text(current, p))
                    .width(150.0)
                    .truncate()
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.sender.is_none(), text("Everyone", p))
                            .clicked()
                        {
                            sender = Some(None);
                        }
                        for (id, name, n) in &self.senders {
                            let on = self.sender.as_ref() == Some(id);
                            let label = format!("{name}  ({})", thousands(*n as i64));
                            if ui.selectable_label(on, text(label, p)).clicked() {
                                sender = Some(Some(id.clone()));
                            }
                        }
                    });
            }
            if self.open.is_some() {
                let typed = ui.add(
                    field(&mut self.go_to, p)
                        .hint_text("Go to date")
                        .desired_width(96.0),
                );
                if typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go_to = true;
                }
                typed.on_hover_text("A day like 2026-01-31, then Enter");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if head.unpack {
                    let can_unpack = self.open.is_some() && !self.unpacking;
                    if button(ui, "Export as folder…", can_unpack, false, p)
                        .on_hover_text("Write this database out as Telegram Desktop's folders")
                        .clicked()
                    {
                        asked.unpack = true;
                    }
                }
                // Re-reads the folder *and* the chat on screen: the reason to
                // press it is usually that a sync has just written to both.
                if button(ui, "Reload", !self.listing, false, p).clicked() {
                    refresh = true;
                }
            });
        });
        if let Some(t) = topic {
            self.set_topic(t);
        }
        if let Some(s) = sender {
            self.set_sender(s);
        }
        if go_to {
            self.go_to_date(host);
        }
        if refresh {
            self.list_databases(host);
            if let Some(db) = self.open.clone() {
                let keep = self.selected;
                self.open_db(db, keep, host);
            }
        }
        asked
    }

    /// Show one topic, or every topic with `None`. The selection stays in
    /// view when it is in the topic chosen.
    pub fn set_topic(&mut self, topic: Option<i64>) {
        self.topic = topic;
        self.rebuild_shown();
        self.keep_selection_in_view();
    }

    /// Show one sender's messages, or everyone's with `None`. Like a topic,
    /// the selection stays in view when it survives.
    pub fn set_sender(&mut self, sender: Option<String>) {
        self.sender = sender;
        self.rebuild_shown();
        self.keep_selection_in_view();
    }

    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.rebuild_shown();
        self.keep_selection_in_view();
    }

    fn keep_selection_in_view(&mut self) {
        if let Some(at) = self.selected.and_then(|id| self.index.get(&id)) {
            self.scroll_to = self.shown.binary_search(at).ok();
        }
    }

    /// Go to date: select the first message shown on or after the day typed.
    /// A day that does not parse says so on the status line and goes nowhere.
    pub fn go_to_date(&mut self, host: &mut dyn Host) {
        let day = self.go_to.trim().to_string();
        if chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d").is_err() {
            host.status(format!("\"{day}\" is not a day like 2026-01-31"));
            return;
        }
        match self.first_on_or_after(&day) {
            Some(i) => {
                let id = self.rows[self.shown[i]].id;
                self.select(id, true, host);
            }
            None => host.status("Nothing shown to go to".into()),
        }
    }

    /// The search box, Prev / Next through this chat's hits, and the switch
    /// that widens a search to every database.
    fn search_row(&mut self, ui: &mut Ui, p: &Palette, host: &mut dyn Host) {
        let mut go = false;
        let mut step = 0isize;
        let mut toggle = false;
        let mut back_to_found = false;
        let mut filter = None;
        row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::TIGHT;
            let hint = if self.everywhere {
                "Search every database…"
            } else {
                "Search this chat…"
            };
            let typed = ui.add(
                field(&mut self.query, p)
                    .hint_text(hint)
                    .desired_width(SEARCH_W),
            );
            if std::mem::take(&mut self.focus_search) {
                typed.request_focus();
            }
            // **Enter searches, typing does not.** A search reads every
            // message of the chat, and of every chat when widened; doing that
            // per keystroke would queue a scan per letter.
            if typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            let can_search =
                !self.query.trim().is_empty() && (self.everywhere || self.open.is_some());
            if action(ui, text("Search", p), can_search).clicked() {
                go = true;
            }
            ui.add_space(space::TIGHT);
            let stepping = !self.hits.is_empty() && !self.showing_found;
            if action(ui, text("Previous", p), stepping).clicked() {
                step = -1;
            }
            if action(ui, text("Next", p), stepping).clicked() {
                step = 1;
            }
            if !self.hits.is_empty() {
                let at = self.hit_at.map(|i| i + 1).unwrap_or(0);
                ui.label(figure(format!("{at} / {}", self.hits.len()), p));
            }
            ui.add_space(space::STEP);
            // Box and label are one control, as everywhere else in the window.
            let hit = tick_box(ui, self.everywhere, true, p).union(action(
                ui,
                text("All databases", p),
                true,
            ));
            if hit.clicked() {
                toggle = true;
            }
            if let Some(found) = &self.found {
                if !self.showing_found {
                    let label = format!("Results ({})", found.total());
                    if action(ui, text(label, p), true).clicked() {
                        back_to_found = true;
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let labels: Vec<&str> = Filter::ALL.iter().map(|f| f.label()).collect();
                let at = Filter::ALL
                    .iter()
                    .position(|f| *f == self.filter)
                    .unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, at, p) {
                    filter = Some(Filter::ALL[i]);
                }
            });
        });
        if let Some(f) = filter {
            self.set_filter(f);
        }
        if toggle {
            self.everywhere = !self.everywhere;
        }
        if back_to_found {
            self.showing_found = true;
        }
        if go {
            self.search(host);
        }
        if step != 0 {
            self.step_hit(step, host);
        }
    }

    /// What the list holds, in one line under it.
    fn summary(&self) -> String {
        if self.showing_found {
            return match &self.found {
                Some(f) => format!(
                    "{} hit(s) in {} chat(s) across every database",
                    f.total(),
                    f.chats.len()
                ),
                None => String::new(),
            };
        }
        // Not "0 messages" for a file that could not be read.
        if self.open.is_none() || self.loading || self.load_error.is_some() {
            return String::new();
        }
        let (deleted, edited, recovered) = self.counts;
        let mut line = format!(
            "{} messages · {} deleted · {} edited · {} recovered",
            thousands(self.rows.len() as i64),
            thousands(deleted as i64),
            thousands(edited as i64),
            thousands(recovered as i64)
        );
        if self.filter != Filter::All {
            line.push_str(&format!(
                "  —  showing {}",
                thousands(self.shown.len() as i64)
            ));
        }
        line
    }

    /// Why the list is empty, when it is. `none_open` is the window's hint
    /// for where a chat is opened from.
    pub fn empty(&self, none_open: &str) -> Option<EmptyState> {
        if self.open.is_none() {
            let hint = (!none_open.is_empty()).then(|| none_open.to_string());
            return Some(EmptyState::new("No chat open", hint));
        }
        if self.loading {
            return Some(EmptyState::new("Reading the database…", None));
        }
        if let Some(e) = &self.load_error {
            return Some(EmptyState::new(
                "Could not read this database",
                Some(format!("{e} — Reload tries again.")),
            ));
        }
        if self.rows.is_empty() {
            return Some(EmptyState::new("This database holds no messages", None));
        }
        if self.shown.is_empty() {
            return Some(EmptyState::new(
                format!("No {} messages", self.filter.label().to_lowercase()),
                Some("Choose All to see every message.".into()),
            ));
        }
        None
    }

    /// The chat, as bubbles.
    ///
    /// **Virtualised over measured heights.** A bubble is as tall as its text,
    /// so `show_rows` and its one fixed height cannot page this list. Every
    /// item starts with a guess ([`bubbles::estimate`]); what is drawn is
    /// measured, and the guess replaced. Only what is on screen is ever laid
    /// out, so a chat of half a million messages costs a frame what a short
    /// one does.
    fn rows_list(&mut self, ui: &mut Ui, p: &Palette, host: &mut dyn Host, none_open: &str) {
        let ink = bubbles::ink(p);
        ui.painter()
            .rect_filled(ui.max_rect(), egui::CornerRadius::ZERO, ink.bg);
        if let Some(empty) = self.empty(none_open) {
            empty.show(ui, p, true);
            return;
        }
        let width = ui.available_width();
        self.frame += 1;
        // Decoded pictures become textures here, the one place with a
        // context to upload them to.
        for (id, pic) in self.pictures.iter_mut() {
            if let bubbles::Pic::Decoded(img) = pic {
                let tex = ui.ctx().load_texture(
                    format!("viewer-pic-{id}"),
                    (**img).clone(),
                    egui::TextureOptions::LINEAR,
                );
                *pic = bubbles::Pic::Ready(tex);
            }
        }
        for (peer, pic) in self.userpics.iter_mut() {
            if let bubbles::Pic::Decoded(img) = pic {
                let tex = ui.ctx().load_texture(
                    format!("viewer-userpic-{peer}"),
                    (**img).clone(),
                    egui::TextureOptions::LINEAR,
                );
                *pic = bubbles::Pic::Ready(tex);
            }
        }
        // Only when the items themselves changed; see `heights` for why a
        // new width is not a reason.
        if self.heights.len() != self.items.len() {
            self.heights = self
                .items
                .iter()
                .map(|it| bubbles::estimate(it, &self.rows, width))
                .collect();
            offsets_of(&self.heights, &mut self.offsets);
        }

        let names = self.mixing_topics().then_some(&self.topic_names);
        let painted = std::cell::RefCell::new(Vec::new());
        let scene = bubbles::Scene {
            rows: &self.rows,
            index: &self.index,
            selected: self.selected,
            hits: &self.hit_set,
            topic_names: names,
            ink,
            pictures: &self.pictures,
            painted: &painted,
            userpics: &self.userpics,
        };

        let mut area = egui::ScrollArea::vertical()
            .id_salt("viewer-rows")
            .auto_shrink([false, false]);
        let mut measured: Vec<(usize, f32)> = Vec::new();
        // **A jump lands the message mid-pane, not at an edge**, so the ones
        // around a hit — what it answered, what answered it — are on screen
        // with it. Its neighbours are measured first, so it lands where the
        // list will really draw it.
        if let Some(i) = self.scroll_to.take() {
            if let Some(&item) = self.item_of_shown.get(i) {
                let from = item.saturating_sub(JUMP_MEASURE);
                let to = (item + JUMP_MEASURE).min(self.items.len());
                for k in from..to {
                    let (h, _) =
                        bubbles::draw(ui, &self.items[k], &scene, ui.max_rect().min, width, false);
                    measured.push((k, h));
                }
                for &(k, h) in &measured {
                    self.heights[k] = h;
                }
                measured.clear();
                offsets_of(&self.heights, &mut self.offsets);
                let offset =
                    self.offsets[item] - ui.available_height() / 2.0 + self.heights[item] / 2.0;
                area = area.vertical_scroll_offset(offset.max(0.0));
            }
        } else if self.scroll_adjust != 0.0 {
            area = area.vertical_scroll_offset((self.last_offset + self.scroll_adjust).max(0.0));
        }
        self.scroll_adjust = 0.0;

        let total = self.offsets.last().copied().unwrap_or(0.0);
        let mut click = None;
        let mut adjust = 0.0;
        let (items, offsets, heights) = (&self.items, &self.offsets, &self.heights);
        let out = area.show_viewport(ui, |ui, viewport| {
            ui.set_height(total);
            let top = ui.max_rect().top();
            let left = ui.max_rect().left();
            let first = offsets
                .partition_point(|&o| o <= viewport.min.y)
                .saturating_sub(1);
            for k in first..items.len() {
                let y = offsets[k];
                if y > viewport.max.y {
                    break;
                }
                let (h, c) = bubbles::draw(
                    ui,
                    &items[k],
                    &scene,
                    egui::pos2(left, top + y),
                    width,
                    true,
                );
                if c.is_some() {
                    click = c;
                }
                if (h - heights[k]).abs() > 0.5 {
                    // Growing above the top edge pushes the view's contents
                    // down; the scroll follows it, so what is read stays put.
                    if y < viewport.min.y {
                        adjust += h - heights[k];
                    }
                    measured.push((k, h));
                }
            }
        });
        self.last_offset = out.state.offset.y;
        if !measured.is_empty() {
            for (k, h) in measured {
                self.heights[k] = h;
            }
            offsets_of(&self.heights, &mut self.offsets);
            self.scroll_adjust = adjust;
            // The guesses just became measurements; draw again with them
            // rather than leave this frame's overlaps on screen.
            ui.ctx().request_repaint();
        }
        let painted = painted.into_inner();
        self.want_pictures(&painted, host);
        match click {
            Some(bubbles::Click::Select(id)) => self.select(id, false, host),
            Some(bubbles::Click::Jump(id)) => self.select(id, true, host),
            Some(bubbles::Click::Open(id)) => {
                self.select(id, false, host);
                self.open_message_media(id, host);
            }
            Some(bubbles::Click::Url(url)) => {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
            None => {}
        }
    }

    /// The pictures painted this frame: keep the ones held, ask for the ones
    /// not, and let the least recently seen go once too many are held.
    fn want_pictures(&mut self, painted: &[i64], host: &mut dyn Host) {
        let Some(db) = self.open.clone() else {
            return;
        };
        let frame = self.frame;
        let in_flight = self
            .pictures
            .values()
            .filter(|p| matches!(p, bubbles::Pic::Loading))
            .count();
        let mut ask = Vec::new();
        for &id in painted {
            self.pic_used.insert(id, frame);
            if !self.pictures.contains_key(&id) && in_flight + ask.len() < PICTURES_IN_FLIGHT {
                self.pictures.insert(id, bubbles::Pic::Loading);
                ask.push(id);
            }
        }
        let held = self
            .pictures
            .values()
            .filter(|p| matches!(p, bubbles::Pic::Ready(_)))
            .count();
        if held > PICTURES_KEPT {
            let mut ready: Vec<(u64, i64)> = self
                .pictures
                .iter()
                .filter(|(_, p)| matches!(p, bubbles::Pic::Ready(_)))
                .map(|(id, _)| (self.pic_used.get(id).copied().unwrap_or(0), *id))
                .collect();
            ready.sort_unstable();
            for (used, id) in ready.into_iter().take(held - PICTURES_KEPT) {
                if used == frame {
                    break;
                }
                self.pictures.remove(&id);
                self.pic_used.remove(&id);
            }
        }
        if ask.is_empty() {
            return;
        }
        host.run(Job::Pictures {
            path: db.path,
            chat_id: db.chat_id,
            ids: ask,
        });
    }

    fn open_message_media(&mut self, id: i64, host: &mut dyn Host) {
        let Some(db) = self.open.clone() else {
            return;
        };
        host.status("Opening…".into());
        host.run(Job::OpenMessageMedia {
            path: db.path,
            chat_id: db.chat_id,
            id,
        });
    }

    /// The hits of a search across every database. A click opens that chat at
    /// that message.
    fn found_list(&mut self, ui: &mut Ui, p: &Palette, host: &mut dyn Host) {
        let Some(found) = &self.found else {
            return;
        };
        if self.found_rows.is_empty() {
            EmptyState::new(
                "Nothing matches in any database",
                Some("Earlier versions and deleted messages were searched too.".into()),
            )
            .show(ui, p, true);
            return;
        }
        let mut clicked = None;
        let rows = &self.found_rows;
        egui::ScrollArea::vertical()
            .id_salt("viewer-found")
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                for i in range {
                    let Some(&(c, h)) = rows.get(i) else { continue };
                    let chat = &found.chats[c];
                    let hit = &chat.hits[h];
                    let (rect, response) = cell(ui);
                    if response.hovered() {
                        ui.painter()
                            .rect_filled(rect, egui::CornerRadius::ZERO, p.surface);
                    }
                    inside(ui, rect, |ui| {
                        ui.label(figure(hit.date.replacen('T', " ", 1), p));
                        fixed(ui, FROM_W + 40.0, |ui| {
                            ui.add(egui::Label::new(text(&chat.title, p)).truncate());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if hit.deleted {
                                badge(ui, "Deleted", Tone::Ink, p);
                            }
                            if hit.in_earlier_version {
                                badge(ui, "Earlier version", Tone::Quiet, p);
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(
                                    egui::Label::new(meta(first_line(&hit.excerpt), p)).truncate(),
                                );
                            });
                        });
                    });
                    if response.clicked() {
                        clicked = Some((c, h));
                    }
                }
            });
        if let Some((c, h)) = clicked {
            self.open_hit(c, h, host);
        }
    }

    fn detail_pane(&mut self, ui: &mut Ui, p: &Palette, host: &mut dyn Host) {
        let Some(d) = self.detail.clone() else {
            if let Some(e) = &self.detail_error {
                EmptyState::new(
                    "Could not read this message",
                    Some(format!("{e} — click the message again to retry.")),
                )
                .show(ui, p, true);
                return;
            }
            let headline = if self.selected.is_some() {
                "Reading…"
            } else {
                "No message selected"
            };
            EmptyState::new(headline, Some("Click a row to read it in full.".into()))
                .show(ui, p, true);
            return;
        };
        let mut jump = None;
        let mut open = None;
        let mut save = None;
        let mut copy = false;
        egui::ScrollArea::vertical()
            .id_salt("viewer-detail-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(space::STEP);
                block(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = space::TIGHT;
                    // The message and its history are content: they select
                    // and copy, as the bubble's words do.
                    ui.style_mut().interaction.selectable_labels = true;
                    ui.label(figure(format!("Message {}", d.id), p));
                    ui.label(meta(format!("{}  ·  {}", d.date, d.from), p));
                    if let Some(at) = &d.deleted {
                        ui.label(text(format!("Deleted on Telegram — noticed {at}"), p));
                    }
                    if let Some(at) = &d.edited {
                        ui.label(meta(format!("Edited {at}"), p));
                    }
                    if d.recovered {
                        ui.label(meta("Recovered from an old export", p));
                    }
                    if let Some(to) = d.reply_to {
                        // A link only when the message it answers is here to
                        // jump to; otherwise just the number.
                        let here = self.index.contains_key(&to);
                        if action(ui, text(format!("Reply to {to}"), p), here).clicked() {
                            jump = Some(to);
                        }
                    }
                    if let Some(a) = &d.action {
                        ui.label(meta(format!("[{}]", a.replace('_', " ")), p));
                    }
                    ui.add_space(space::TIGHT);
                    if !d.text.is_empty() {
                        ui.add(egui::Label::new(text(&d.text, p)).wrap());
                        // To the clipboard and nowhere else: the text is
                        // other people's, and nothing here logs it.
                        if action(ui, meta("Copy text", p), true).clicked() {
                            copy = true;
                        }
                    }

                    if !d.versions.is_empty() {
                        ui.add_space(space::TIGHT);
                        rule(ui, p);
                        ui.label(eyebrow("Earlier versions", p));
                        for v in &d.versions {
                            let edited = v
                                .edited
                                .as_ref()
                                .map(|e| format!(", edited {e}"))
                                .unwrap_or_default();
                            ui.label(meta(
                                format!("Version {} — replaced {}{edited}", v.version, v.replaced),
                                p,
                            ));
                            let body = if v.text.is_empty() {
                                "(no text)"
                            } else {
                                &v.text
                            };
                            ui.add(egui::Label::new(text(body, p)).wrap());
                        }
                    }

                    if !d.media.is_empty() {
                        ui.add_space(space::TIGHT);
                        rule(ui, p);
                        ui.label(eyebrow("Media", p));
                        for m in &d.media {
                            ui.horizontal(|ui| {
                                let stored = m.size.is_some();
                                if button(ui, "Open", stored, false, p).clicked() {
                                    open = Some(m.clone());
                                }
                                // Open's copy is deleted when the window
                                // closes; this one is kept where it is put.
                                if button(ui, "Save as…", stored, false, p).clicked() {
                                    save = Some(m.clone());
                                }
                                let size = m
                                    .size
                                    .map(|s| human_bytes(s.max(0) as u64))
                                    .unwrap_or_else(|| "not stored".into());
                                ui.label(figure(size, p));
                                ui.add(egui::Label::new(meta(&m.name, p)).truncate());
                            });
                        }
                    }
                });
                ui.add_space(space::STEP);
            });
        if let Some(id) = jump {
            self.select(id, true, host);
        }
        if let Some(m) = open {
            self.open_media(m, host);
        }
        if let Some(m) = save {
            self.save_media(m, host);
        }
        if copy {
            ui.ctx().copy_text(d.text.clone());
            host.status("Message text copied".into());
        }
    }

    /// Save as: ask where, then copy the stored file there.
    fn save_media(&mut self, media: Media, host: &mut dyn Host) {
        let Some(db) = self.open.clone() else {
            return;
        };
        let Some(to) = rfd::FileDialog::new()
            .set_title("Save file as")
            .set_file_name(&media.name)
            .save_file()
        else {
            return;
        };
        host.status(format!("Saving {}…", media.name));
        host.run(Job::SaveMedia {
            path: db.path,
            media,
            to,
        });
    }

    fn open_media(&mut self, media: Media, host: &mut dyn Host) {
        let Some(db) = self.open.clone() else {
            return;
        };
        host.status(format!("Opening {}…", media.name));
        host.run(Job::OpenMedia {
            path: db.path,
            media,
        });
    }

    // -- acting --------------------------------------------------------------

    /// Ask for the databases in the folder.
    pub fn list_databases(&mut self, host: &mut dyn Host) {
        self.listing = true;
        self.listed_for = Some(self.folder.clone());
        host.run(Job::Databases {
            dir: self.folder.clone(),
        });
    }

    /// Open one chat's database, and select `then` once it has loaded.
    pub fn open_db(&mut self, db: Database, then: Option<i64>, host: &mut dyn Host) {
        let path = db.path.clone();
        let chat_id = db.chat_id;
        if !self.is_open(&path, chat_id) {
            // Another chat's hits, topics and pictures mean nothing in this
            // one — a picture is keyed by message id, and ids repeat.
            self.set_hits(Vec::new());
            self.topic = None;
            self.topics.clear();
            self.topic_names.clear();
            self.sender = None;
            self.senders.clear();
            self.pictures.clear();
            self.pic_used.clear();
            self.userpics.clear();
        }
        self.open = Some(db);
        self.loading = true;
        self.load_error = None;
        self.rows.clear();
        self.index.clear();
        self.shown.clear();
        self.items.clear();
        self.item_of_shown.clear();
        self.heights.clear();
        self.offsets.clear();
        self.counts = (0, 0, 0);
        self.selected = None;
        self.detail = None;
        self.detail_error = None;
        self.scroll_to = None;
        self.select_after_load = then;
        self.showing_found = false;
        host.run(Job::Load {
            path: path.clone(),
            chat_id,
        });
        host.run(Job::Userpics { path, chat_id });
    }

    /// Select a message: highlight it, ask for it in full, and — when `scroll`
    /// — bring it into view. A click on a row passes `false`: it is already
    /// where the user is looking, and moving it would move the list under the
    /// pointer.
    ///
    /// A message the filter is hiding switches the filter to All rather than
    /// selecting something nobody can see; one in another topic switches to
    /// its topic.
    pub fn select(&mut self, id: i64, scroll: bool, host: &mut dyn Host) {
        let Some(&at) = self.index.get(&id) else {
            return;
        };
        let row_topic = self.rows[at].topic_id;
        let hidden_by_filter = !self.filter.keeps(&self.rows[at]);
        let hidden_by_topic = self.topic.is_some_and(|t| t != row_topic);
        let hidden_by_sender = self
            .sender
            .as_ref()
            .is_some_and(|s| *s != self.rows[at].from_id);
        if hidden_by_filter {
            self.filter = Filter::All;
        }
        if hidden_by_topic {
            self.topic = Some(row_topic);
        }
        if hidden_by_sender {
            self.sender = None;
        }
        if hidden_by_filter || hidden_by_topic || hidden_by_sender {
            self.rebuild_shown();
        }
        if scroll {
            self.scroll_to = self.shown.binary_search(&at).ok();
        }
        if self.selected == Some(id) && self.detail.is_some() {
            return;
        }
        self.selected = Some(id);
        self.detail = None;
        self.detail_error = None;
        if let Some(i) = self.hits.iter().position(|h| *h == id) {
            self.hit_at = Some(i);
        }
        let Some(db) = self.open.clone() else {
            return;
        };
        host.run(Job::Detail {
            path: db.path,
            chat_id: db.chat_id,
            id,
        });
    }

    /// Search this chat, or every database in the folder.
    pub fn search(&mut self, host: &mut dyn Host) {
        self.search_in(self.folder.clone(), host);
    }

    /// Search this chat, or every database in `dir` — which may not be
    /// [`Self::folder`]: TelegramTools' Analysis searches the folder it lists.
    pub fn search_in(&mut self, dir: String, host: &mut dyn Host) {
        let needle = self.query.trim().to_string();
        if needle.is_empty() {
            return;
        }
        let scope = if self.everywhere {
            None
        } else {
            match &self.open {
                Some(db) => Some((db.path.clone(), db.chat_id, db.title.clone())),
                None => return,
            }
        };
        self.asked = Some((needle.clone(), scope.as_ref().map(|s| s.1)));
        host.status("Searching…".into());
        host.run(Job::Search { dir, scope, needle });
    }

    /// Prev (-1) or Next (+1) through this chat's hits, wrapping.
    pub fn step_hit(&mut self, step: isize, host: &mut dyn Host) {
        let n = self.hits.len() as isize;
        if n == 0 {
            return;
        }
        let at = match self.hit_at {
            Some(i) => (i as isize + step).rem_euclid(n),
            None if step < 0 => n - 1,
            None => 0,
        } as usize;
        self.hit_at = Some(at);
        let id = self.hits[at];
        self.select(id, true, host);
    }

    /// A hit in the every-database list, clicked: open its chat at it, with
    /// that chat's hits loaded for Prev / Next.
    pub fn open_hit(&mut self, chat: usize, hit: usize, host: &mut dyn Host) {
        let Some(found) = &self.found else {
            return;
        };
        let Some(c) = found.chats.get(chat) else {
            return;
        };
        let Some(h) = c.hits.get(hit) else {
            return;
        };
        let db = Database {
            chat_id: c.chat_id,
            title: c.title.clone(),
            path: c.path.clone(),
        };
        let id = h.id;
        let ids: Vec<i64> = c.hits.iter().map(|h| h.id).collect();
        self.showing_found = false;
        if self.is_open(&db.path, db.chat_id) && !self.loading {
            self.set_hits(ids);
            self.hit_at = Some(hit);
            self.select(id, true, host);
        } else {
            self.open_db(db, Some(id), host);
            self.set_hits(ids);
            self.hit_at = Some(hit);
        }
    }

    // -- applying what came back ---------------------------------------------

    /// One answer from a [`Job`]. Anything about a chat no longer open, a
    /// folder no longer shown or a question no longer asked is dropped.
    pub fn apply(&mut self, reply: Reply, host: &mut dyn Host) {
        match reply {
            Reply::Databases { dir, found } => self.databases_found(dir, found, host),
            Reply::Loaded {
                path,
                chat_id,
                rows,
            } => self.loaded(path, chat_id, rows, host),
            Reply::Detail {
                chat_id,
                id,
                detail,
            } => self.detail_read(chat_id, id, detail, host),
            Reply::Picture {
                path,
                chat_id,
                id,
                image,
                has_file,
            } => {
                if self.is_open(&path, chat_id) {
                    let pic = match image {
                        Some(img) => bubbles::Pic::Decoded(img),
                        None if has_file => bubbles::Pic::NoPreview,
                        None => bubbles::Pic::Missing,
                    };
                    self.pictures.insert(id, pic);
                }
            }
            Reply::Userpics {
                path,
                chat_id,
                pics,
            } => {
                if self.is_open(&path, chat_id) {
                    self.userpics = pics
                        .into_iter()
                        .map(|(peer, img)| (peer, bubbles::Pic::Decoded(img)))
                        .collect();
                }
            }
            Reply::Hits {
                needle,
                scope,
                found,
            } => self.hits_found(needle, scope, found, host),
            Reply::Status(line) => host.status(line),
            Reply::Warn(line) => {
                host.status(line.clone());
                host.warn(line);
            }
        }
    }

    fn fail(host: &mut dyn Host, e: String) {
        host.status(format!("Failed: {e}"));
        host.warn(e);
    }

    fn databases_found(
        &mut self,
        dir: String,
        found: Result<Vec<Database>, String>,
        host: &mut dyn Host,
    ) {
        // **Cleared before the answer is judged.** Left set by a stale one, it
        // stopped the view from ever listing again and greyed Reload out for
        // the rest of the session.
        self.listing = false;
        if dir != self.folder {
            // The folder changed while the old one was being read: ask about
            // the new one. Cannot loop — `listed_for` is now the current
            // folder, so its answer is not stale.
            self.list_databases(host);
            return;
        }
        match found {
            Ok(dbs) => {
                self.databases = dbs;
                // **Open the first, when nothing is open yet.** A view whose
                // first screen is an empty list under a picker is a question
                // the user has to answer before they can see anything at all.
                // **Not under a search of every database** — a search box
                // elsewhere comes here with no chat open, and opening one
                // would put a chat over the results that were asked for.
                let searching_all = self.showing_found || matches!(self.asked, Some((_, None)));
                if self.open.is_none() && !searching_all {
                    let by_path = self
                        .want_path
                        .take()
                        .and_then(|want| self.databases.iter().find(|d| d.path == want).cloned());
                    let wanted = by_path.or_else(|| {
                        self.want_title.as_ref().and_then(|want| {
                            self.databases
                                .iter()
                                .find(|d| d.title.to_lowercase().contains(want))
                                .cloned()
                        })
                    });
                    if let Some(first) = wanted.or_else(|| self.databases.first().cloned()) {
                        self.open_db(first, None, host);
                    }
                }
            }
            Err(e) => {
                self.databases.clear();
                Self::fail(host, e);
            }
        }
    }

    fn loaded(
        &mut self,
        path: std::path::PathBuf,
        chat_id: i64,
        loaded: Result<Loaded, String>,
        host: &mut dyn Host,
    ) {
        if !self.is_open(&path, chat_id) {
            return;
        }
        self.loading = false;
        let Loaded { rows, topics } = match loaded {
            Ok(loaded) => loaded,
            Err(e) => {
                self.load_error = Some(e.clone());
                Self::fail(host, e);
                return;
            }
        };
        self.index = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        self.counts = rows.iter().fold((0, 0, 0), |(d, e, r), row| {
            (
                d + usize::from(row.deleted.is_some()),
                e + usize::from(row.edited()),
                r + usize::from(row.recovered),
            )
        });
        self.senders = Self::senders_of(&rows);
        if self
            .sender
            .as_ref()
            .is_some_and(|s| !self.senders.iter().any(|x| &x.0 == s))
        {
            self.sender = None;
        }
        self.rows = rows;
        self.topic_names = topics.iter().map(|t| (t.id, t.title.clone())).collect();
        // A topic picked in this chat before a Reload stays picked; one from
        // another chat, or no longer here, does not.
        if self
            .topic
            .is_some_and(|t| !self.topic_names.contains_key(&t))
        {
            self.topic = None;
        }
        self.topics = topics;
        self.rebuild_shown();
        if self.select_after_load.is_none() {
            if let Some(want) = &self.want_media {
                self.select_after_load = self
                    .rows
                    .iter()
                    .find(|r| r.media.as_deref().is_some_and(|m| m.contains(want)))
                    .map(|r| r.id);
            }
        }
        if let Some(id) = self.select_after_load.take() {
            self.select(id, true, host);
        }
    }

    fn detail_read(
        &mut self,
        chat_id: i64,
        id: i64,
        detail: Result<Option<Box<Detail>>, String>,
        host: &mut dyn Host,
    ) {
        let current =
            self.selected == Some(id) && self.open.as_ref().is_some_and(|d| d.chat_id == chat_id);
        if !current {
            return;
        }
        match detail {
            Ok(Some(d)) => self.detail = Some(d),
            Ok(None) => {
                let line = format!("Message {id} is no longer in the database");
                self.detail_error = Some(line.clone());
                host.status(line);
            }
            Err(e) => {
                self.detail_error = Some(e.clone());
                Self::fail(host, e);
            }
        }
    }

    fn hits_found(
        &mut self,
        needle: String,
        scope: Option<i64>,
        found: Result<Box<Found>, String>,
        host: &mut dyn Host,
    ) {
        if self.asked.as_ref() != Some(&(needle.clone(), scope)) {
            return;
        }
        self.asked = None;
        let found = match found {
            Ok(found) => found,
            Err(e) => {
                Self::fail(host, e);
                return;
            }
        };
        // The count, never the text: the status line is read over shoulders.
        let total = found.total();
        match scope {
            None => {
                host.status(format!("{total} hit(s) in {} chat(s)", found.chats.len()));
                self.found_rows = found
                    .chats
                    .iter()
                    .enumerate()
                    .flat_map(|(c, chat)| (0..chat.hits.len()).map(move |h| (c, h)))
                    .collect();
                self.found = Some(found);
                self.showing_found = true;
            }
            Some(_) => {
                let ids: Vec<i64> = found
                    .chats
                    .first()
                    .map(|c| c.hits.iter().map(|h| h.id).collect())
                    .unwrap_or_default();
                host.status(if ids.is_empty() {
                    "Nothing matches in this chat".into()
                } else {
                    format!("{} hit(s) in this chat", ids.len())
                });
                self.set_hits(ids);
                self.showing_found = false;
                self.step_hit(1, host);
            }
        }
    }
}

/// Where each item starts, from the heights before it — a free function, so
/// the list can call it while the rest of the viewer's state is lent out.
fn offsets_of(heights: &[f32], offsets: &mut Vec<f32>) {
    offsets.clear();
    offsets.reserve(heights.len() + 1);
    let mut y = 0.0;
    for h in heights {
        offsets.push(y);
        y += h;
    }
    offsets.push(y);
}

/// The first line of `s`.
fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// One list row's rect: full width, exactly [`ROW_HEIGHT`], clickable.
fn cell(ui: &mut Ui) -> (egui::Rect, egui::Response) {
    ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::click())
}

/// Lay a row out inside its rect, vertically centred, inside the gutters — the
/// chat list's `inside`, for the same reason it exists.
fn inside<R>(ui: &mut Ui, rect: egui::Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + GUTTER, rect.top()),
        egui::pos2(rect.right() - GUTTER, rect.bottom()),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            add(ui)
        },
    )
    .inner
}

/// A column of fixed width, so the text after it starts at the same place on
/// every row whatever the name in it.
fn fixed(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui)) {
    let height = ui.available_height();
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_width(width);
            add(ui)
        },
    );
}
