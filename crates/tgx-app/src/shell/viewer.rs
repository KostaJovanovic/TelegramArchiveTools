//! The Viewer: one chat's database, every message in it, read-only.
//!
//! **Nothing here touches JSON.** The rows arrive from
//! `tgx_tg::viewer::load` already flattened — a date, a sender, a first line,
//! the flags — on the blocking pool, so a chat of a hundred thousand messages
//! costs a frame what nine do: `show_rows` is virtualised exactly as the chat
//! list is, and asks only for what is on screen.
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

use super::bubbles;
use super::*;
use eframe::egui::{Align, Layout, Sense, Ui};
use std::collections::{HashMap, HashSet};
use tgx_tg::search::Found;
use tgx_tg::viewer::{Database, Detail, Loaded, Media, Row, Topic};
use tgx_ui::components::{
    action, block, button, caps, eyebrow, field, figure, meta, row, rule, segmented, text,
    thousands, tick_box, EmptyState, GUTTER,
};
use tgx_ui::tokens::space;

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

/// Which messages the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Filter {
    #[default]
    All,
    /// Gone from Telegram, kept here.
    Deleted,
    /// Changed since first stored — an edit, or reactions and counters moving.
    Edited,
}

impl Filter {
    pub(super) const ALL: [Filter; 3] = [Filter::All, Filter::Deleted, Filter::Edited];

    fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Deleted => "Deleted",
            Filter::Edited => "Edited",
        }
    }

    pub(super) fn keeps(self, r: &Row) -> bool {
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
pub(super) struct ViewerState {
    /// The databases in the destination, by title.
    pub(super) databases: Vec<Database>,
    /// Which destination `databases` describes. `None` until it is first
    /// asked; a different folder from the settings' means ask again.
    pub(super) listed_for: Option<String>,
    pub(super) listing: bool,

    /// The chat on screen.
    pub(super) open: Option<Database>,
    pub(super) loading: bool,
    pub(super) rows: Vec<Row>,
    /// Message id to its index in `rows`.
    pub(super) index: HashMap<i64, usize>,
    /// Indices into `rows` the filter keeps, in order.
    pub(super) shown: Vec<usize>,
    pub(super) filter: Filter,
    /// `(deleted, edited, recovered)` across `rows`, counted once per load.
    pub(super) counts: (usize, usize, usize),

    /// The message in the detail pane, and what the database says of it.
    pub(super) selected: Option<i64>,
    pub(super) detail: Option<Box<Detail>>,
    /// A row to bring into view on the next frame, as an index into `shown`.
    pub(super) scroll_to: Option<usize>,
    /// Select this message once the chat being opened has loaded — a hit in
    /// another database, clicked.
    pub(super) select_after_load: Option<i64>,

    /// The search box's buffer.
    pub(super) query: String,
    /// Search every database rather than the one open.
    pub(super) everywhere: bool,
    /// The question in flight: the text and the chat, `None` for every
    /// database. An answer to anything else is dropped.
    pub(super) asked: Option<(String, Option<i64>)>,
    /// This chat's hits, oldest first, and which one Prev / Next is on.
    pub(super) hits: Vec<i64>,
    pub(super) hit_set: HashSet<i64>,
    pub(super) hit_at: Option<usize>,
    /// The last search of every database, and its hits as `(chat, hit)`
    /// pairs into it — the list `show_rows` pages through.
    pub(super) found: Option<Box<Found>>,
    pub(super) found_rows: Vec<(usize, usize)>,
    /// The list is showing `found` rather than the open chat.
    pub(super) showing_found: bool,
    /// Export as Classic is writing; its button is off until it answers.
    pub(super) unpacking: bool,

    /// The forum topics the open chat's messages are filed under; more than
    /// one only for a forum. `topic` is the one shown, `None` for all.
    pub(super) topics: Vec<Topic>,
    pub(super) topic: Option<i64>,
    pub(super) topic_names: HashMap<i64, String>,

    /// What the list draws — day separators and messages in runs — and, for
    /// each entry of `shown`, which item it is.
    pub(super) items: Vec<bubbles::Item>,
    pub(super) item_of_shown: Vec<usize>,
    /// Each item's height and where it starts. Guessed until it is drawn,
    /// then measured; `offsets` has one more entry than `items`, the total.
    pub(super) heights: Vec<f32>,
    pub(super) offsets: Vec<f32>,
    /// The width the heights are for. A different width re-guesses them all.
    pub(super) laid_width: f32,
    /// Where the list was scrolled to last frame, and how far to move it this
    /// frame so that an item above the view growing on measurement does not
    /// push what is being read down the screen.
    pub(super) last_offset: f32,
    pub(super) scroll_adjust: f32,

    /// The pictures asked for, by message id, and the frame each was last
    /// painted on — what decides which go when there are too many.
    pub(super) pictures: HashMap<i64, bubbles::Pic>,
    pub(super) pic_used: HashMap<i64, u64>,
    pub(super) frame: u64,
}

/// Pictures kept as textures at once. At most 720px a side, so this bounds
/// the viewer's video memory at a little under 300 MB in the worst case and
/// far less in practice.
const PICTURES_KEPT: usize = 160;
/// Pictures being read and decoded at once. Past it, the rest wait for the
/// next frame — so a fast scroll asks for what it stops on, not for every
/// photo it flew past.
const PICTURES_IN_FLIGHT: usize = 6;

impl ViewerState {
    fn rebuild_shown(&mut self) {
        let filter = self.filter;
        let topic = self.topic;
        self.shown = (0..self.rows.len())
            .filter(|&i| {
                let r = &self.rows[i];
                filter.keeps(r) && topic.is_none_or(|t| r.topic_id == t)
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

    fn is_open(&self, path: &std::path::Path, chat_id: i64) -> bool {
        self.open
            .as_ref()
            .is_some_and(|d| d.path == path && d.chat_id == chat_id)
    }
}

impl Shell {
    // -- painting ------------------------------------------------------------

    /// **Head, foot, detail, then the list — as panels**, the chat list's
    /// arrangement and for its reason: each measures itself and the list gets
    /// exactly what is left, at any window size.
    pub(super) fn viewer_panel(&mut self, ui: &mut Ui) {
        let p = self.palette;
        // The destination is free text and can change under the view; the
        // list of databases is asked for again when it does.
        if self.viewer.listed_for.as_deref() != Some(self.settings.output_dir.as_str())
            && !self.viewer.listing
        {
            self.viewer_list_databases();
        }
        let bare = egui::Frame::NONE.fill(p.bg);

        egui::TopBottomPanel::top("viewer-head")
            .frame(bare)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.add_space(space::STEP);
                self.viewer_picker_row(ui);
                ui.add_space(space::TIGHT);
                self.viewer_search_row(ui);
                ui.add_space(space::STEP);
                rule(ui, &p);
            });

        egui::TopBottomPanel::bottom("viewer-foot")
            .frame(bare)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                rule(ui, &p);
                ui.add_space(space::TIGHT);
                row(ui, |ui| ui.label(eyebrow(&self.viewer_summary(), &p)));
                ui.add_space(space::TIGHT);
            });

        // **Only once something is selected.** Empty, it said "no message
        // selected" in a third of the window at the minimum size — the
        // chat's width, spent on telling the user they had not clicked yet.
        if self.viewer.open.is_some()
            && !self.viewer.showing_found
            && self.viewer.selected.is_some()
        {
            egui::SidePanel::right("viewer-detail")
                .frame(bare)
                .resizable(false)
                .exact_width(DETAIL_W)
                .show_inside(ui, |ui| self.viewer_detail_pane(ui));
        }

        egui::CentralPanel::default()
            .frame(bare)
            .show_inside(ui, |ui| {
                if self.viewer.showing_found {
                    self.viewer_found_list(ui);
                } else {
                    self.viewer_rows(ui);
                }
            });
    }

    /// The database picker, the topic picker, Refresh and Export as Classic.
    fn viewer_picker_row(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let mut chosen = None;
        let mut topic = None;
        let mut refresh = false;
        let mut unpack = false;
        row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            ui.label(eyebrow("Database", &p));
            let v = &self.viewer;
            let current = match (&v.open, v.databases.is_empty(), v.listing) {
                (Some(db), _, _) => db.title.clone(),
                (None, true, true) => "Looking…".into(),
                (None, true, false) => "None in the destination".into(),
                (None, false, _) => "Choose a database".into(),
            };
            ui.add_enabled_ui(!v.databases.is_empty(), |ui| {
                egui::ComboBox::from_id_salt("viewer-db")
                    .selected_text(text(current, &p))
                    .width(220.0)
                    .truncate()
                    .show_ui(ui, |ui| {
                        for db in &v.databases {
                            let on = v.is_open(&db.path, db.chat_id);
                            if ui.selectable_label(on, text(&db.title, &p)).clicked() {
                                chosen = Some(db.clone());
                            }
                        }
                    });
            });
            // **Only a forum has topics to pick between**, so only a forum
            // gets the picker; for any other chat it would offer one choice.
            if v.topics.len() > 1 {
                ui.label(eyebrow("Topic", &p));
                let current = match v.topic {
                    None => "All topics".to_string(),
                    Some(id) => v.topic_names.get(&id).cloned().unwrap_or_default(),
                };
                egui::ComboBox::from_id_salt("viewer-topic")
                    .selected_text(text(current, &p))
                    .width(170.0)
                    .truncate()
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(v.topic.is_none(), text("All topics", &p))
                            .clicked()
                        {
                            topic = Some(None);
                        }
                        for t in &v.topics {
                            let on = v.topic == Some(t.id);
                            if ui.selectable_label(on, text(&t.title, &p)).clicked() {
                                topic = Some(Some(t.id));
                            }
                        }
                    });
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let can_unpack = v.open.is_some() && !v.unpacking;
                if button(ui, "Export as Classic…", can_unpack, false, &p).clicked() {
                    unpack = true;
                }
                // Re-reads the folder *and* the chat on screen: the reason to
                // press it is usually that a sync has just written to both.
                if button(ui, "Refresh", !v.listing, false, &p).clicked() {
                    refresh = true;
                }
            });
        });
        if let Some(db) = chosen {
            self.viewer_open(db, None);
        }
        if let Some(t) = topic {
            self.viewer_set_topic(t);
        }
        if refresh {
            self.viewer_list_databases();
            if let Some(db) = self.viewer.open.clone() {
                let keep = self.viewer.selected;
                self.viewer_open(db, keep);
            }
        }
        if unpack {
            self.viewer_unpack();
        }
    }

    /// Show one topic, or every topic with `None`. The selection stays in
    /// view when it is in the topic chosen.
    pub(super) fn viewer_set_topic(&mut self, topic: Option<i64>) {
        self.viewer.topic = topic;
        self.viewer.rebuild_shown();
        if let Some(at) = self
            .viewer
            .selected
            .and_then(|id| self.viewer.index.get(&id))
        {
            self.viewer.scroll_to = self.viewer.shown.binary_search(at).ok();
        }
    }

    /// Export as Classic: ask where, then write the open database out as the
    /// folders a Classic run would have made — deleted messages included.
    fn viewer_unpack(&mut self) {
        let Some(db) = self.viewer.open.clone() else {
            return;
        };
        let Some(out) = rfd::FileDialog::new()
            .set_title("Export as Classic into…")
            .set_directory(&self.settings.output_dir)
            .pick_folder()
        else {
            return;
        };
        self.viewer.unpacking = true;
        self.status = format!("Writing {} out as a Classic folder…", db.title);
        let tx = self.bridge.sender();
        let settings = self.settings.clone();
        self.bridge
            .spawn(async move { crate::actions::unpack(db.path, out, settings, tx).await });
    }

    /// Export as Classic finished. Logged, so the folder it went to can be
    /// copied from the log.
    pub(super) fn viewer_unpacked(&mut self, done: Result<Vec<String>, String>) {
        self.viewer.unpacking = false;
        self.log_copied = false;
        match done {
            Ok(lines) => {
                self.status = lines
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Nothing written".into());
                for line in lines {
                    self.journal.push(line);
                }
            }
            Err(e) => {
                self.status = e.clone();
                self.journal.warn(e);
            }
        }
    }

    /// The search box, Prev / Next through this chat's hits, and the switch
    /// that widens a search to every database.
    fn viewer_search_row(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let mut go = false;
        let mut step = 0isize;
        let mut toggle = false;
        let mut back_to_found = false;
        let mut filter = None;
        row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::TIGHT;
            let hint = if self.viewer.everywhere {
                "Search every database…"
            } else {
                "Search this chat…"
            };
            let typed = ui.add(
                field(&mut self.viewer.query, &p)
                    .hint_text(hint)
                    .desired_width(SEARCH_W),
            );
            // **Enter searches, typing does not.** A search reads every
            // message of the chat, and of every chat when widened; doing that
            // per keystroke would queue a scan per letter.
            if typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            let can_search = !self.viewer.query.trim().is_empty()
                && (self.viewer.everywhere || self.viewer.open.is_some());
            if action(ui, caps("Search", p.fg), can_search).clicked() {
                go = true;
            }
            ui.add_space(space::TIGHT);
            let stepping = !self.viewer.hits.is_empty() && !self.viewer.showing_found;
            if action(ui, caps("Prev", p.fg), stepping).clicked() {
                step = -1;
            }
            if action(ui, caps("Next", p.fg), stepping).clicked() {
                step = 1;
            }
            if !self.viewer.hits.is_empty() {
                let at = self.viewer.hit_at.map(|i| i + 1).unwrap_or(0);
                ui.label(figure(format!("{at} / {}", self.viewer.hits.len()), &p));
            }
            ui.add_space(space::STEP);
            // Box and label are one control, as everywhere else in the window.
            let hit = tick_box(ui, self.viewer.everywhere, true, &p).union(action(
                ui,
                eyebrow("All databases", &p),
                true,
            ));
            if hit.clicked() {
                toggle = true;
            }
            if let Some(found) = &self.viewer.found {
                if !self.viewer.showing_found {
                    let label = format!("Results  {}", found.total());
                    if action(ui, caps(&label, p.fg), true).clicked() {
                        back_to_found = true;
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let v = &self.viewer;
                let labels: Vec<&str> = Filter::ALL.iter().map(|f| f.label()).collect();
                let at = Filter::ALL.iter().position(|f| *f == v.filter).unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, at, &p) {
                    filter = Some(Filter::ALL[i]);
                }
            });
        });
        if let Some(f) = filter {
            self.viewer_set_filter(f);
        }
        if toggle {
            self.viewer.everywhere = !self.viewer.everywhere;
        }
        if back_to_found {
            self.viewer.showing_found = true;
        }
        if go {
            self.viewer_search();
        }
        if step != 0 {
            self.viewer_step_hit(step);
        }
    }

    /// What the list holds, in one line under it.
    fn viewer_summary(&self) -> String {
        let v = &self.viewer;
        if v.showing_found {
            return match &v.found {
                Some(f) => format!(
                    "{} hit(s) in {} chat(s) across every database",
                    f.total(),
                    f.chats.len()
                ),
                None => String::new(),
            };
        }
        if v.open.is_none() || v.loading {
            return String::new();
        }
        let (deleted, edited, recovered) = v.counts;
        let mut line = format!(
            "{} messages · {} deleted · {} edited · {} recovered",
            thousands(v.rows.len() as i64),
            thousands(deleted as i64),
            thousands(edited as i64),
            thousands(recovered as i64)
        );
        if v.filter != Filter::All {
            line.push_str(&format!("  —  showing {}", thousands(v.shown.len() as i64)));
        }
        line
    }

    /// Why the list is empty, when it is.
    fn viewer_empty(&self) -> Option<EmptyState> {
        let v = &self.viewer;
        if v.open.is_none() {
            return Some(if v.listing {
                EmptyState::new("Looking for databases…", None)
            } else if v.databases.is_empty() {
                EmptyState::new(
                    "No databases in the destination",
                    Some(
                        "Database mode keeps one per chat there. Sync a chat, then press Refresh."
                            .into(),
                    ),
                )
            } else {
                EmptyState::new("Choose a database", None)
            });
        }
        if v.loading {
            return Some(EmptyState::new("Reading the database…", None));
        }
        if v.rows.is_empty() {
            return Some(EmptyState::new("This database holds no messages", None));
        }
        if v.shown.is_empty() {
            return Some(EmptyState::new(
                format!("No {} messages", v.filter.label().to_lowercase()),
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
    fn viewer_rows(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let ink = bubbles::ink(&p);
        ui.painter()
            .rect_filled(ui.max_rect(), egui::CornerRadius::ZERO, ink.bg);
        if let Some(empty) = self.viewer_empty() {
            empty.show(ui, &p, true);
            return;
        }
        let width = ui.available_width();
        let v = &mut self.viewer;
        v.frame += 1;
        // Decoded pictures become textures here, the one place with a
        // context to upload them to.
        for (id, pic) in v.pictures.iter_mut() {
            if let bubbles::Pic::Decoded(img) = pic {
                let tex = ui.ctx().load_texture(
                    format!("viewer-pic-{id}"),
                    (**img).clone(),
                    egui::TextureOptions::LINEAR,
                );
                *pic = bubbles::Pic::Ready(tex);
            }
        }
        if v.heights.len() != v.items.len() || (v.laid_width - width).abs() > 0.5 {
            v.laid_width = width;
            v.heights = v
                .items
                .iter()
                .map(|it| bubbles::estimate(it, &v.rows, width))
                .collect();
            offsets_of(&v.heights, &mut v.offsets);
        }

        let names = v.mixing_topics().then_some(&v.topic_names);
        let painted = std::cell::RefCell::new(Vec::new());
        let scene = bubbles::Scene {
            rows: &v.rows,
            index: &v.index,
            selected: v.selected,
            hits: &v.hit_set,
            topic_names: names,
            ink,
            pictures: &v.pictures,
            painted: &painted,
        };

        let mut area = egui::ScrollArea::vertical()
            .id_salt("viewer-rows")
            .auto_shrink([false, false]);
        let mut measured: Vec<(usize, f32)> = Vec::new();
        // **A jump lands the message mid-pane, not at an edge**, so the ones
        // around a hit — what it answered, what answered it — are on screen
        // with it. Its neighbours are measured first, so it lands where the
        // list will really draw it.
        if let Some(i) = v.scroll_to.take() {
            if let Some(&item) = v.item_of_shown.get(i) {
                let from = item.saturating_sub(JUMP_MEASURE);
                let to = (item + JUMP_MEASURE).min(v.items.len());
                for k in from..to {
                    let (h, _) =
                        bubbles::draw(ui, &v.items[k], &scene, ui.max_rect().min, width, false);
                    measured.push((k, h));
                }
                for &(k, h) in &measured {
                    v.heights[k] = h;
                }
                measured.clear();
                offsets_of(&v.heights, &mut v.offsets);
                let offset = v.offsets[item] - ui.available_height() / 2.0 + v.heights[item] / 2.0;
                area = area.vertical_scroll_offset(offset.max(0.0));
            }
        } else if v.scroll_adjust != 0.0 {
            area = area.vertical_scroll_offset((v.last_offset + v.scroll_adjust).max(0.0));
        }
        v.scroll_adjust = 0.0;

        let total = v.offsets.last().copied().unwrap_or(0.0);
        let mut click = None;
        let mut adjust = 0.0;
        let out = area.show_viewport(ui, |ui, viewport| {
            ui.set_height(total);
            let top = ui.max_rect().top();
            let left = ui.max_rect().left();
            let first = v
                .offsets
                .partition_point(|&o| o <= viewport.min.y)
                .saturating_sub(1);
            for k in first..v.items.len() {
                let y = v.offsets[k];
                if y > viewport.max.y {
                    break;
                }
                let (h, c) = bubbles::draw(
                    ui,
                    &v.items[k],
                    &scene,
                    egui::pos2(left, top + y),
                    width,
                    true,
                );
                if c.is_some() {
                    click = c;
                }
                if (h - v.heights[k]).abs() > 0.5 {
                    // Growing above the top edge pushes the view's contents
                    // down; the scroll follows it, so what is read stays put.
                    if y < viewport.min.y {
                        adjust += h - v.heights[k];
                    }
                    measured.push((k, h));
                }
            }
        });
        v.last_offset = out.state.offset.y;
        if !measured.is_empty() {
            for (k, h) in measured {
                v.heights[k] = h;
            }
            offsets_of(&v.heights, &mut v.offsets);
            v.scroll_adjust = adjust;
            // The guesses just became measurements; draw again with them
            // rather than leave this frame's overlaps on screen.
            ui.ctx().request_repaint();
        }
        let painted = painted.into_inner();
        self.viewer_want_pictures(&painted);
        match click {
            Some(bubbles::Click::Select(id)) => self.viewer_select(id, false),
            Some(bubbles::Click::Jump(id)) => self.viewer_select(id, true),
            Some(bubbles::Click::Open(id)) => {
                self.viewer_select(id, false);
                self.viewer_open_message_media(id);
            }
            Some(bubbles::Click::Url(url)) => {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
            None => {}
        }
    }

    /// The pictures painted this frame: keep the ones held, ask for the ones
    /// not, and let the least recently seen go once too many are held.
    fn viewer_want_pictures(&mut self, painted: &[i64]) {
        let Some(db) = self.viewer.open.clone() else {
            return;
        };
        let v = &mut self.viewer;
        let frame = v.frame;
        let in_flight = v
            .pictures
            .values()
            .filter(|p| matches!(p, bubbles::Pic::Loading))
            .count();
        let mut ask = Vec::new();
        for &id in painted {
            v.pic_used.insert(id, frame);
            if !v.pictures.contains_key(&id) && in_flight + ask.len() < PICTURES_IN_FLIGHT {
                v.pictures.insert(id, bubbles::Pic::Loading);
                ask.push(id);
            }
        }
        let held = v
            .pictures
            .values()
            .filter(|p| matches!(p, bubbles::Pic::Ready(_)))
            .count();
        if held > PICTURES_KEPT {
            let mut ready: Vec<(u64, i64)> = v
                .pictures
                .iter()
                .filter(|(_, p)| matches!(p, bubbles::Pic::Ready(_)))
                .map(|(id, _)| (v.pic_used.get(id).copied().unwrap_or(0), *id))
                .collect();
            ready.sort_unstable();
            for (used, id) in ready.into_iter().take(held - PICTURES_KEPT) {
                if used == frame {
                    break;
                }
                v.pictures.remove(&id);
                v.pic_used.remove(&id);
            }
        }
        if ask.is_empty() {
            return;
        }
        let tx = self.bridge.sender();
        self.bridge.spawn(async move {
            crate::actions::viewer::pictures(db.path, db.chat_id, ask, tx).await
        });
    }

    /// A picture came back. Kept only if its chat is still the one open.
    pub(super) fn viewer_picture(
        &mut self,
        path: &std::path::Path,
        chat_id: i64,
        id: i64,
        image: Option<std::sync::Arc<egui::ColorImage>>,
        has_file: bool,
    ) {
        if !self.viewer.is_open(path, chat_id) {
            return;
        }
        let pic = match image {
            Some(img) => bubbles::Pic::Decoded(img),
            None if has_file => bubbles::Pic::NoPreview,
            None => bubbles::Pic::Missing,
        };
        self.viewer.pictures.insert(id, pic);
    }

    fn viewer_open_message_media(&mut self, id: i64) {
        let Some(db) = self.viewer.open.clone() else {
            return;
        };
        self.status = "Opening…".into();
        let tx = self.bridge.sender();
        self.bridge.spawn(async move {
            crate::actions::viewer::open_message_media(db.path, db.chat_id, id, tx).await
        });
    }

    /// The hits of a search across every database. A click opens that chat at
    /// that message.
    fn viewer_found_list(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let Some(found) = &self.viewer.found else {
            return;
        };
        if self.viewer.found_rows.is_empty() {
            EmptyState::new(
                "Nothing matches in any database",
                Some("Earlier versions and deleted messages were searched too.".into()),
            )
            .show(ui, &p, true);
            return;
        }
        let mut clicked = None;
        let rows = &self.viewer.found_rows;
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
                        ui.label(figure(hit.date.replacen('T', " ", 1), &p));
                        fixed(ui, FROM_W + 40.0, |ui| {
                            ui.add(egui::Label::new(text(&chat.title, &p)).truncate());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if hit.deleted {
                                ui.label(caps("deleted", p.accent));
                            }
                            if hit.in_earlier_version {
                                ui.label(caps("earlier version", p.muted));
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(
                                    egui::Label::new(meta(first_line(&hit.excerpt), &p)).truncate(),
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
            self.viewer_open_hit(c, h);
        }
    }

    fn viewer_detail_pane(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let Some(d) = self.viewer.detail.clone() else {
            let headline = if self.viewer.selected.is_some() {
                "Reading…"
            } else {
                "No message selected"
            };
            EmptyState::new(headline, Some("Click a row to read it in full.".into()))
                .show(ui, &p, true);
            return;
        };
        let mut jump = None;
        let mut open = None;
        egui::ScrollArea::vertical()
            .id_salt("viewer-detail-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(space::STEP);
                block(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = space::TIGHT;
                    ui.label(eyebrow(&format!("Message {}", d.id), &p));
                    ui.label(meta(format!("{}  ·  {}", d.date, d.from), &p));
                    if let Some(at) = &d.deleted {
                        ui.label(caps(
                            &format!("Deleted on Telegram — noticed {at}"),
                            p.accent,
                        ));
                    }
                    if let Some(at) = &d.edited {
                        ui.label(meta(format!("Edited {at}"), &p));
                    }
                    if d.recovered {
                        ui.label(meta("Recovered from an old export", &p));
                    }
                    if let Some(to) = d.reply_to {
                        // A link only when the message it answers is here to
                        // jump to; otherwise just the number.
                        let here = self.viewer.index.contains_key(&to);
                        if action(ui, caps(&format!("Reply to {to}"), p.fg), here).clicked() {
                            jump = Some(to);
                        }
                    }
                    if let Some(a) = &d.action {
                        ui.label(meta(format!("[{}]", a.replace('_', " ")), &p));
                    }
                    ui.add_space(space::TIGHT);
                    if !d.text.is_empty() {
                        ui.add(egui::Label::new(text(&d.text, &p)).wrap());
                    }

                    if !d.versions.is_empty() {
                        ui.add_space(space::TIGHT);
                        rule(ui, &p);
                        ui.label(eyebrow("Earlier versions", &p));
                        for v in &d.versions {
                            let edited = v
                                .edited
                                .as_ref()
                                .map(|e| format!(", edited {e}"))
                                .unwrap_or_default();
                            ui.label(meta(
                                format!("Version {} — replaced {}{edited}", v.version, v.replaced),
                                &p,
                            ));
                            let body = if v.text.is_empty() {
                                "(no text)"
                            } else {
                                &v.text
                            };
                            ui.add(egui::Label::new(text(body, &p)).wrap());
                        }
                    }

                    if !d.media.is_empty() {
                        ui.add_space(space::TIGHT);
                        rule(ui, &p);
                        ui.label(eyebrow("Media", &p));
                        for m in &d.media {
                            ui.horizontal(|ui| {
                                let stored = m.size.is_some();
                                if button(ui, "Open", stored, false, &p).clicked() {
                                    open = Some(m.clone());
                                }
                                let size = m
                                    .size
                                    .map(|s| crate::list::human_bytes(s.max(0) as u64))
                                    .unwrap_or_else(|| "not stored".into());
                                ui.label(figure(size, &p));
                                ui.add(egui::Label::new(meta(&m.name, &p)).truncate());
                            });
                        }
                    }
                });
                ui.add_space(space::STEP);
            });
        if let Some(id) = jump {
            self.viewer_select(id, true);
        }
        if let Some(m) = open {
            self.viewer_open_media(m);
        }
    }

    // -- acting --------------------------------------------------------------

    /// Ask for the databases in the destination.
    pub(super) fn viewer_list_databases(&mut self) {
        self.viewer.listing = true;
        self.viewer.listed_for = Some(self.settings.output_dir.clone());
        let tx = self.bridge.sender();
        let dir = self.settings.output_dir.clone();
        self.bridge
            .spawn(async move { crate::actions::viewer::databases(dir, tx).await });
    }

    /// Open one chat's database, and select `then` once it has loaded.
    pub(super) fn viewer_open(&mut self, db: Database, then: Option<i64>) {
        let path = db.path.clone();
        let chat_id = db.chat_id;
        let v = &mut self.viewer;
        if !v.is_open(&path, chat_id) {
            // Another chat's hits, topics and pictures mean nothing in this
            // one — a picture is keyed by message id, and ids repeat.
            v.set_hits(Vec::new());
            v.topic = None;
            v.topics.clear();
            v.topic_names.clear();
            v.pictures.clear();
            v.pic_used.clear();
        }
        v.open = Some(db);
        v.loading = true;
        v.rows.clear();
        v.index.clear();
        v.shown.clear();
        v.items.clear();
        v.item_of_shown.clear();
        v.heights.clear();
        v.offsets.clear();
        v.counts = (0, 0, 0);
        v.selected = None;
        v.detail = None;
        v.scroll_to = None;
        v.select_after_load = then;
        v.showing_found = false;
        let tx = self.bridge.sender();
        self.bridge
            .spawn(async move { crate::actions::viewer::load(path, chat_id, tx).await });
    }

    /// Select a message: highlight it, ask for it in full, and — when `scroll`
    /// — bring it into view. A click on a row passes `false`: it is already
    /// where the user is looking, and moving it would move the list under the
    /// pointer.
    ///
    /// A message the filter is hiding switches the filter to All rather than
    /// selecting something nobody can see; one in another topic switches to
    /// its topic.
    pub(super) fn viewer_select(&mut self, id: i64, scroll: bool) {
        let Some(&at) = self.viewer.index.get(&id) else {
            return;
        };
        let row_topic = self.viewer.rows[at].topic_id;
        let hidden_by_filter = !self.viewer.filter.keeps(&self.viewer.rows[at]);
        let hidden_by_topic = self.viewer.topic.is_some_and(|t| t != row_topic);
        if hidden_by_filter {
            self.viewer.filter = Filter::All;
        }
        if hidden_by_topic {
            self.viewer.topic = Some(row_topic);
        }
        if hidden_by_filter || hidden_by_topic {
            self.viewer.rebuild_shown();
        }
        if scroll {
            self.viewer.scroll_to = self.viewer.shown.binary_search(&at).ok();
        }
        if self.viewer.selected == Some(id) && self.viewer.detail.is_some() {
            return;
        }
        self.viewer.selected = Some(id);
        self.viewer.detail = None;
        if let Some(i) = self.viewer.hits.iter().position(|h| *h == id) {
            self.viewer.hit_at = Some(i);
        }
        let Some(db) = self.viewer.open.clone() else {
            return;
        };
        let tx = self.bridge.sender();
        self.bridge.spawn(async move {
            crate::actions::viewer::detail(db.path, db.chat_id, id, tx).await
        });
    }

    pub(super) fn viewer_set_filter(&mut self, filter: Filter) {
        self.viewer.filter = filter;
        self.viewer.rebuild_shown();
        // Keep the selection in view if it survived the filter.
        if let Some(at) = self
            .viewer
            .selected
            .and_then(|id| self.viewer.index.get(&id))
        {
            self.viewer.scroll_to = self.viewer.shown.binary_search(at).ok();
        }
    }

    /// Search this chat, or every database.
    pub(super) fn viewer_search(&mut self) {
        let needle = self.viewer.query.trim().to_string();
        if needle.is_empty() {
            return;
        }
        let scope = if self.viewer.everywhere {
            None
        } else {
            match &self.viewer.open {
                Some(db) => Some((db.path.clone(), db.chat_id, db.title.clone())),
                None => return,
            }
        };
        self.viewer.asked = Some((needle.clone(), scope.as_ref().map(|s| s.1)));
        self.status = "Searching…".into();
        let tx = self.bridge.sender();
        let dir = self.settings.output_dir.clone();
        self.bridge
            .spawn(async move { crate::actions::viewer::search(dir, scope, needle, tx).await });
    }

    /// Prev (-1) or Next (+1) through this chat's hits, wrapping.
    pub(super) fn viewer_step_hit(&mut self, step: isize) {
        let n = self.viewer.hits.len() as isize;
        if n == 0 {
            return;
        }
        let at = match self.viewer.hit_at {
            Some(i) => (i as isize + step).rem_euclid(n),
            None if step < 0 => n - 1,
            None => 0,
        } as usize;
        self.viewer.hit_at = Some(at);
        let id = self.viewer.hits[at];
        self.viewer_select(id, true);
    }

    /// A hit in the every-database list, clicked: open its chat at it, with
    /// that chat's hits loaded for Prev / Next.
    pub(super) fn viewer_open_hit(&mut self, chat: usize, hit: usize) {
        let Some(found) = &self.viewer.found else {
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
        self.viewer.showing_found = false;
        if self.viewer.is_open(&db.path, db.chat_id) && !self.viewer.loading {
            self.viewer.set_hits(ids);
            self.viewer.hit_at = Some(hit);
            self.viewer_select(id, true);
        } else {
            self.viewer_open(db, Some(id));
            self.viewer.set_hits(ids);
            self.viewer.hit_at = Some(hit);
        }
    }

    fn viewer_open_media(&mut self, media: Media) {
        let Some(db) = self.viewer.open.clone() else {
            return;
        };
        self.status = format!("Opening {}…", media.name);
        let tx = self.bridge.sender();
        self.bridge
            .spawn(async move { crate::actions::viewer::open_media(db.path, media, tx).await });
    }

    // -- applying what came back ---------------------------------------------

    pub(super) fn viewer_databases(
        &mut self,
        output_dir: String,
        found: Result<Vec<Database>, String>,
    ) {
        if output_dir != self.settings.output_dir {
            return;
        }
        self.viewer.listing = false;
        match found {
            Ok(dbs) => {
                self.viewer.databases = dbs;
                // **Open the first, when nothing is open yet.** A view whose
                // first screen is an empty list under a picker is a question
                // the user has to answer before they can see anything at all.
                if self.viewer.open.is_none() {
                    let wanted = super::shot::database().and_then(|want| {
                        self.viewer
                            .databases
                            .iter()
                            .find(|d| d.title.to_lowercase().contains(&want))
                            .cloned()
                    });
                    if let Some(first) = wanted.or_else(|| self.viewer.databases.first().cloned()) {
                        self.viewer_open(first, None);
                    }
                }
            }
            Err(e) => {
                self.viewer.databases.clear();
                self.status = format!("Failed: {e}");
                self.journal.warn(e);
                self.log_copied = false;
            }
        }
    }

    pub(super) fn viewer_loaded(
        &mut self,
        path: std::path::PathBuf,
        chat_id: i64,
        loaded: Result<Loaded, String>,
    ) {
        if !self.viewer.is_open(&path, chat_id) {
            return;
        }
        self.viewer.loading = false;
        let Loaded { rows, topics } = match loaded {
            Ok(loaded) => loaded,
            Err(e) => {
                self.status = format!("Failed: {e}");
                self.journal.warn(e);
                self.log_copied = false;
                return;
            }
        };
        let v = &mut self.viewer;
        v.index = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        v.counts = rows.iter().fold((0, 0, 0), |(d, e, r), row| {
            (
                d + usize::from(row.deleted.is_some()),
                e + usize::from(row.edited()),
                r + usize::from(row.recovered),
            )
        });
        v.rows = rows;
        v.topic_names = topics.iter().map(|t| (t.id, t.title.clone())).collect();
        // A topic picked in this chat before a Refresh stays picked; one from
        // another chat, or no longer here, does not.
        if v.topic.is_some_and(|t| !v.topic_names.contains_key(&t)) {
            v.topic = None;
        }
        v.topics = topics;
        v.rebuild_shown();
        if v.select_after_load.is_none() {
            if let Some(want) = super::shot::find() {
                v.select_after_load = v
                    .rows
                    .iter()
                    .find(|r| r.media.as_deref().is_some_and(|m| m.contains(&want)))
                    .map(|r| r.id);
            }
        }
        if let Some(id) = v.select_after_load.take() {
            self.viewer_select(id, true);
        }
    }

    pub(super) fn viewer_detail(
        &mut self,
        chat_id: i64,
        id: i64,
        detail: Result<Option<Box<Detail>>, String>,
    ) {
        let current = self.viewer.selected == Some(id)
            && self
                .viewer
                .open
                .as_ref()
                .is_some_and(|d| d.chat_id == chat_id);
        if !current {
            return;
        }
        match detail {
            Ok(Some(d)) => self.viewer.detail = Some(d),
            Ok(None) => self.status = format!("Message {id} is no longer in the database"),
            Err(e) => {
                self.status = format!("Failed: {e}");
                self.journal.warn(e);
                self.log_copied = false;
            }
        }
    }

    pub(super) fn viewer_hits(
        &mut self,
        needle: String,
        scope: Option<i64>,
        found: Result<Box<Found>, String>,
    ) {
        if self.viewer.asked.as_ref() != Some(&(needle.clone(), scope)) {
            return;
        }
        self.viewer.asked = None;
        let found = match found {
            Ok(found) => found,
            Err(e) => {
                self.status = format!("Failed: {e}");
                self.journal.warn(e);
                self.log_copied = false;
                return;
            }
        };
        // The count, never the text: the status line is read over shoulders.
        let total = found.total();
        match scope {
            None => {
                self.status = format!("{total} hit(s) in {} chat(s)", found.chats.len());
                self.viewer.found_rows = found
                    .chats
                    .iter()
                    .enumerate()
                    .flat_map(|(c, chat)| (0..chat.hits.len()).map(move |h| (c, h)))
                    .collect();
                self.viewer.found = Some(found);
                self.viewer.showing_found = true;
            }
            Some(_) => {
                let ids: Vec<i64> = found
                    .chats
                    .first()
                    .map(|c| c.hits.iter().map(|h| h.id).collect())
                    .unwrap_or_default();
                self.status = if ids.is_empty() {
                    "Nothing matches in this chat".into()
                } else {
                    format!("{} hit(s) in this chat", ids.len())
                };
                self.viewer.set_hits(ids);
                self.viewer.showing_found = false;
                self.viewer_step_hit(1);
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
