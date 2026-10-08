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

use super::*;
use eframe::egui::{Align, Layout, Sense, Ui};
use std::collections::{HashMap, HashSet};
use tgx_tg::search::Found;
use tgx_tg::viewer::{Database, Detail, Media, Row};
use tgx_ui::components::{
    action, block, button, caps, eyebrow, field, figure, meta, row, rule, segmented, text,
    thousands, tick_box, EmptyState, GUTTER,
};
use tgx_ui::tokens::space;

/// Every message row is this tall. One line: date, sender, text, markers.
///
/// Shorter than the chat list's 46, because a row here is one line rather than
/// a title over a caption, and the thing being read is the run of messages —
/// more of them on screen is the whole benefit.
const ROW_HEIGHT: f32 = 30.0;

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
}

impl ViewerState {
    fn rebuild_shown(&mut self) {
        let filter = self.filter;
        self.shown = (0..self.rows.len())
            .filter(|&i| filter.keeps(&self.rows[i]))
            .collect();
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

        if self.viewer.open.is_some() && !self.viewer.showing_found {
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

    /// The database picker, Refresh, and the filter.
    fn viewer_picker_row(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let mut chosen = None;
        let mut refresh = false;
        let mut filter = None;
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
                    .width(240.0)
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
            // Re-reads the folder *and* the chat on screen: the reason to
            // press it is usually that a sync has just written to both.
            if button(ui, "Refresh", !v.listing, false, &p).clicked() {
                refresh = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let labels: Vec<&str> = Filter::ALL.iter().map(|f| f.label()).collect();
                let at = Filter::ALL.iter().position(|f| *f == v.filter).unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, at, &p) {
                    filter = Some(Filter::ALL[i]);
                }
            });
        });
        if let Some(db) = chosen {
            self.viewer_open(db, None);
        }
        if refresh {
            self.viewer_list_databases();
            if let Some(db) = self.viewer.open.clone() {
                let keep = self.viewer.selected;
                self.viewer_open(db, keep);
            }
        }
        if let Some(f) = filter {
            self.viewer_set_filter(f);
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
        });
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

    fn viewer_rows(&mut self, ui: &mut Ui) {
        let p = self.palette;
        if let Some(empty) = self.viewer_empty() {
            empty.show(ui, &p, true);
            return;
        }
        let pitch = ROW_HEIGHT + ui.spacing().item_spacing.y;
        let mut area = egui::ScrollArea::vertical()
            .id_salt("viewer-rows")
            .auto_shrink([false, false]);
        // **A jump lands the row mid-list, not at an edge**, so the messages
        // around a hit — what it was a reply to, what answered it — are on
        // screen with it.
        if let Some(i) = self.viewer.scroll_to.take() {
            let offset = i as f32 * pitch - ui.available_height() / 2.0 + pitch / 2.0;
            area = area.vertical_scroll_offset(offset.max(0.0));
        }
        let mut clicked = None;
        let v = &self.viewer;
        area.show_rows(ui, ROW_HEIGHT, v.shown.len(), |ui, range| {
            for i in range {
                let Some(r) = v.shown.get(i).and_then(|&r| v.rows.get(r)) else {
                    continue;
                };
                let selected = v.selected == Some(r.id);
                if message_row(ui, r, selected, v.hit_set.contains(&r.id), &p).clicked() {
                    clicked = Some(r.id);
                }
            }
        });
        if let Some(id) = clicked {
            self.viewer_select(id, false);
        }
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
            // Another chat's hits mean nothing in this one.
            v.set_hits(Vec::new());
        }
        v.open = Some(db);
        v.loading = true;
        v.rows.clear();
        v.index.clear();
        v.shown.clear();
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
    /// selecting something nobody can see.
    pub(super) fn viewer_select(&mut self, id: i64, scroll: bool) {
        let Some(&at) = self.viewer.index.get(&id) else {
            return;
        };
        if !self.viewer.filter.keeps(&self.viewer.rows[at]) {
            self.viewer.filter = Filter::All;
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
                    if let Some(first) = self.viewer.databases.first().cloned() {
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
        rows: Result<Vec<Row>, String>,
    ) {
        if !self.viewer.is_open(&path, chat_id) {
            return;
        }
        self.viewer.loading = false;
        let rows = match rows {
            Ok(rows) => rows,
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
        v.rebuild_shown();
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

/// One message as a list row: date, sender, media and first line, markers.
fn message_row(
    ui: &mut Ui,
    r: &Row,
    selected: bool,
    hit: bool,
    p: &tgx_ui::tokens::Palette,
) -> egui::Response {
    let (rect, response) = cell(ui);
    if selected || response.hovered() {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::ZERO, p.surface);
    }
    // A search hit is marked in the gutter, where it does not compete with
    // the text for the eye — the rows between hits are what give a hit its
    // meaning, and they should not be drowned out.
    if hit {
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height()));
        ui.painter()
            .rect_filled(bar, egui::CornerRadius::ZERO, p.accent);
    }
    inside(ui, rect, |ui| {
        ui.label(figure(&r.date, p));
        let from_ink = if r.service { p.muted } else { p.fg };
        fixed(ui, FROM_W, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&r.from)
                        .color(from_ink)
                        .font(tgx_ui::fonts::medium(tgx_ui::tokens::window::LABEL)),
                )
                .truncate(),
            );
        });
        // Markers first, from the right, so a long first line is what gives
        // way. The nested left-to-right is the status bar's: a truncating
        // label inside a right-to-left layout would draw against the far edge.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(at) = &r.deleted {
                let day = at.get(..10).unwrap_or(at);
                ui.label(caps(&format!("deleted {day}"), p.accent));
            }
            if r.edited() {
                ui.label(caps(&format!("edited x{}", r.version - 1), p.muted));
            }
            if r.recovered {
                ui.label(caps("recovered", p.fg));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let line = match &r.media {
                    Some(m) if r.preview.is_empty() => m.clone(),
                    Some(m) => format!("{m}  {}", r.preview),
                    None => r.preview.clone(),
                };
                let body = if r.service {
                    meta(line, p)
                } else {
                    text(line, p)
                };
                ui.add(egui::Label::new(body).truncate());
            });
        });
    });
    response
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
