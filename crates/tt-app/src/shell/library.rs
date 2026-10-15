//! The Library: every export under a folder, and a report for each.
//!
//! **This was the analyser's whole window**, `TelegramAnalyser.exe`. It is a
//! view of TelegramTools now, and the folder it opens on is the one the
//! exporter writes into — the exports this window made are the ones it reports
//! on. Pointing it somewhere else is still a paste or a Choose away.
//!
//! **A worker event repaints the window; the window never polls for one.** The
//! job hands back a `std::sync::mpsc::Receiver`, which can only be polled — so
//! the worker also holds a [`Wake`] and calls it on every message. It is built
//! here from the egui context, because `tga-run` has no toolkit in it. Without
//! it nothing appears until some unrelated input causes a frame, which the
//! user experiences as having to move the mouse to make the app work.
//!
//! Everything this view *decides* is in [`tga_run::state`]; what is here is
//! where things sit and what they look like.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::SystemTime;

use eframe::egui::{self, Align, Layout, Sense, Ui};
use tga_run::job::{self, Counted, Request, Wake};
use tga_run::open;
use tga_run::refresh::Source;
use tga_run::state::{Mark, Pick, Show, Sort, State};
use tgx_ui::components::{
    action, badge, button, field, figure, meta, row, rule, text, tick_mark, EmptyState, GUTTER,
};
use tgx_ui::tokens::{space, Palette};

// Every row is this tall — **the chat list's own constant**, not a copy of its
// value, so a title over a caption sits the same in both views and changing
// one cannot leave the other behind.
use super::chats::ROW_HEIGHT;

/// How wide the filter box is. **Fixed**, for the chat list's reason: a field
/// stretched across the window advertises room for a sentence, and what goes
/// in it is a word.
const FILTER_W: f32 = 260.0;

/// The count column: wide enough for `1,486,313`.
const COUNT_W: f32 = 80.0;

/// The mark column: wide enough for `UP TO DATE`, the longest of the six.
const MARK_W: f32 = 96.0;

#[derive(Default)]
pub struct Library {
    pub(super) state: State,
    /// The run in progress. `None` between runs.
    run: Option<job::Run>,
    /// Titles and counts arriving for the rows. `None` once the counter has
    /// finished and hung up.
    counts: Option<Receiver<Counted>>,
    /// The destination the Library last opened on, by itself. `None` until the
    /// view is first shown, so a launch that never opens the Library never
    /// scans a drive for it.
    followed: Option<String>,
    /// A row's Read, pressed this frame: the database and the chat in it. The
    /// shell takes it and opens the Viewer — this view has no Viewer to open.
    pub(super) read: Option<Read>,
    /// Exports the exporter has finished, waiting for the analyser: the
    /// database file or the export folder of each. See [`Library::report_later`].
    waiting: Vec<PathBuf>,
    /// The search box's buffer: text to find in every database's messages.
    /// Not the filter beside it, which matches export titles.
    search: String,
    /// A search, pressed this frame, and the folder it is of. The shell takes
    /// it and shows the results in the reader, as it takes a row's Read.
    pub(super) search_all: Option<(String, String)>,
    /// Ctrl+F was pressed: the filter box takes the focus when next drawn.
    pub(super) focus_filter: bool,
}

/// What a click on one of a row's own actions asked for.
enum RowAction {
    Open(PathBuf),
    Report(PathBuf),
    Read(Read),
    Reveal(PathBuf),
}

/// What a row's Read hands the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub path: PathBuf,
    pub chat_id: i64,
    pub title: String,
}

/// What the Viewer opens for a Read — a row's, or a summary's citation.
impl From<Read> for tgx_tg::viewer::Database {
    fn from(read: Read) -> Self {
        tgx_tg::viewer::Database {
            chat_id: read.chat_id,
            title: read.title,
            path: read.path,
        }
    }
}

/// What a worker calls after every send: a repaint of this window.
fn wake(ctx: &egui::Context) -> Wake {
    let ctx = ctx.clone();
    Arc::new(move || ctx.request_repaint())
}

impl Library {
    /// Point the view at the destination: the first time it is shown, and
    /// again whenever the destination changes in Settings — **unless the
    /// folder was pointed somewhere else by hand**, which is a choice the next
    /// edit to Settings must not undo.
    ///
    /// A photograph never opens the real destination, because the rows are
    /// chat titles and a shot is something that gets pasted into an issue; it
    /// lists `TGX_SHOT_LIBRARY` or nothing.
    pub(super) fn follow(&mut self, output_dir: &str, ctx: &egui::Context) {
        // Not mid-run: the rows are the run's. The change is picked up on the
        // first frame after it finishes.
        if self.followed.as_deref() == Some(output_dir) || self.state.running {
            return;
        }
        let first = self.followed.is_none();
        let ours = self
            .followed
            .as_deref()
            .is_some_and(|old| Path::new(old) == self.state.folder().unwrap_or_default());
        self.followed = Some(output_dir.to_string());
        if !(first || ours) {
            return;
        }
        let folder = if super::shot::path().is_some() {
            if !first {
                return;
            }
            super::shot::library()
        } else {
            Some(output_dir.to_string()).filter(|d| !d.trim().is_empty())
        };
        if let Some(folder) = folder {
            self.set_path(Path::new(&folder), ctx);
        }
    }

    /// Whether a worker still has something to say — for the screenshot,
    /// which must not photograph a list whose figures are still blank.
    pub(super) fn busy(&self) -> bool {
        self.counts.is_some() || self.run.is_some()
    }

    /// Re-read the folder in the field, and send whatever is new off to be
    /// counted.
    fn rescan(&mut self, ctx: &egui::Context) {
        let todo = self.state.rescan();
        self.count(todo, ctx);
    }

    /// F5: read the folder again. Not mid-run — the rows are the run's.
    pub(super) fn reload(&mut self, ctx: &egui::Context) {
        if !self.state.running {
            self.rescan(ctx);
        }
    }

    fn set_path(&mut self, path: &Path, ctx: &egui::Context) {
        let todo = self.state.set_path(path);
        self.count(todo, ctx);
    }

    fn count(&mut self, todo: Vec<Source>, ctx: &egui::Context) {
        if !todo.is_empty() {
            // Replacing the receiver drops the last one: a count still running
            // for a folder no longer on screen sends into nothing and stops.
            self.counts = Some(job::count(todo, self.state.generation, wake(ctx)));
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        let targets = self.state.start();
        self.spawn(targets, ctx);
    }

    /// One row's Make report: that row alone, whatever is ticked.
    fn start_one(&mut self, path: &Path, ctx: &egui::Context) {
        let targets = self.state.start_one(path);
        self.spawn(targets, ctx);
    }

    /// What every report contains — Settings › Reports. Held on the state
    /// because that is what `spawn` builds the request from, and what the
    /// analyser's own tests drive.
    pub(super) fn options(&mut self, digest: bool, fonts: bool, handles: bool) {
        self.state.write_digest = digest;
        self.state.embed_fonts = fonts;
        self.state.handles_only = handles;
    }

    /// "Report after export": a chat the exporter has just finished, to be
    /// reported on as soon as the analyser is free. See
    /// [`Library::report_waiting`].
    pub(super) fn report_later(&mut self, export: PathBuf) {
        if !self.waiting.contains(&export) {
            self.waiting.push(export);
        }
    }

    #[cfg(test)]
    pub(super) fn waiting(&self) -> &[PathBuf] {
        &self.waiting
    }

    /// Start a run on whatever exports finished while the analyser was busy —
    /// all of them, one run. Called every frame; does nothing mid-run, so the
    /// chats that finish during one are picked up by the next.
    pub(super) fn report_waiting(&mut self, ctx: &egui::Context) {
        if self.state.running || self.waiting.is_empty() {
            return;
        }
        let targets: Vec<_> = std::mem::take(&mut self.waiting)
            .iter()
            .filter_map(|p| tga_run::refresh::target_for(p))
            .collect();
        let targets = self.state.start_these(targets);
        self.spawn(targets, ctx);
    }

    fn spawn(&mut self, targets: Option<Vec<tga_run::refresh::Target>>, ctx: &egui::Context) {
        let Some(targets) = targets else {
            return;
        };
        let request = Request {
            targets,
            write_digest: self.state.write_digest,
            embed_fonts: self.state.embed_fonts,
            handles_only: self.state.handles_only,
        };
        self.run = Some(job::spawn(request, wake(ctx)));
    }

    /// Stop after the report in hand. Also what quitting does — see
    /// `Shell::on_exit`.
    pub(super) fn stop(&mut self) {
        if let Some(run) = &self.run {
            run.stop();
            self.state.stopping = true;
            self.state.status = "Stopping after this export".into();
        }
    }

    /// Apply everything the workers have said since the last frame.
    ///
    /// Every frame, whichever view is showing: a run carries on while someone
    /// looks at the chat list, and its rows must be right when they come back.
    /// In a batch, because a run emits one message per topic it reads, and a
    /// ten-topic export would otherwise be ten frames nobody sees.
    pub(super) fn drain(&mut self, ctx: &egui::Context) {
        self.drain_counts();
        self.drain_run();
        // What the rescan ending a run found, once no count is running —
        // starting one drops the last (`State::take_uncounted`).
        if self.counts.is_none() {
            let todo = self.state.take_uncounted();
            self.count(todo, ctx);
        }
    }

    fn drain_counts(&mut self) {
        if let Some(rx) = &self.counts {
            let mut batch = Vec::new();
            let finished = loop {
                match rx.try_recv() {
                    Ok(counted) => batch.push(counted),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break false,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break true,
                }
            };
            for counted in batch {
                self.state.apply_count(counted);
            }
            if finished {
                self.counts = None;
            }
        }
    }

    fn drain_run(&mut self) {
        let Some(run) = &self.run else {
            return;
        };
        let batch: Vec<job::Progress> = run.events.try_iter().collect();
        for event in batch {
            self.state.apply(event);
        }
        if !self.state.running {
            // The worker is finished with; dropping the receiver is what lets a
            // second run start clean.
            self.run = None;
        }
    }

    fn browse(&mut self, ctx: &egui::Context) {
        if let Some(dir) = rfd::FileDialog::new()
            .set_title("Choose an export, or the folder the exporter syncs into")
            .pick_folder()
        {
            self.set_path(&dir, ctx);
        }
    }

    /// A folder — or a database — dropped anywhere on the window, while this
    /// view is showing.
    ///
    /// The window is the target rather than the field, which is a small one.
    /// The first directory wins, or the first file that is a database: those
    /// are the two things an export can be.
    pub(super) fn dropped(&mut self, ctx: &egui::Context) {
        if self.state.running {
            return;
        }
        let dirs: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .filter(|p| tga_run::can_open(p))
                .collect()
        });
        if let Some(dir) = dirs.into_iter().next() {
            self.set_path(&dir, ctx);
        }
    }

    fn open_report(&mut self, path: &Path) {
        if let Err(e) = open::open(path) {
            // The report may have been deleted, or the drive unplugged. The
            // status line says so rather than a dialog from the shell.
            self.state.status = e;
        }
    }

    /// The view's half of the top bar: the folder, and what to do with it.
    ///
    /// Right to left from the far end, so what is listed first lands furthest
    /// right and the field takes whatever is left.
    pub(super) fn bar(&mut self, ui: &mut Ui, ctx: &egui::Context, p: &Palette) {
        let running = self.state.running;
        let label = report_label(self.state.selected.len(), running);
        let root = self.state.root.clone().filter(|r| r.is_dir());

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // **The one red.** Everything else on the bar leads to it.
            if button(ui, &label, self.state.ready(), true, p).clicked() {
                self.start(ctx);
            }
            if running && button(ui, "Stop", !self.state.stopping, false, p).clicked() {
                self.stop();
            }
            ui.add_space(space::STEP);
            if button(ui, "Open folder", root.is_some(), false, p).clicked() {
                if let Some(dir) = &root {
                    if let Err(e) = open::reveal(dir) {
                        self.state.status = e;
                    }
                }
            }
            if button(ui, "Choose", !running, false, p).clicked() {
                self.browse(ctx);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let width = ui.available_width();
                // Pasting a path out of Explorer is how this view is actually
                // used, so the field is a field and not a label beside a
                // Choose button.
                let edited = ui
                    .add_enabled(
                        !running,
                        field(&mut self.state.path, p)
                            .hint_text(r"C:\...\telegram export")
                            .desired_width(width),
                    )
                    .changed();
                if edited {
                    self.rescan(ctx);
                }
            });
        });
    }

    /// The body: the list between its filter row and its foot.
    pub(super) fn panel(&mut self, ui: &mut Ui, p: &Palette) {
        let bare = egui::Frame::NONE.fill(p.bg);
        if self.state.entries.is_empty() {
            let empty = if self.state.folder().is_none() {
                EmptyState::new(
                    "Choose an export",
                    Some(
                        "A folder of result.json files, one of the exporter's databases, \
                         or the folder it syncs them into. Paste a path above, press \
                         Choose, or drop it on the window."
                            .into(),
                    ),
                )
            } else {
                EmptyState::new(
                    "No exports here",
                    Some(
                        "No result.json under it, and no .sqlite in it or in the folders \
                         directly inside it."
                            .into(),
                    ),
                )
            };
            egui::CentralPanel::default()
                .frame(bare)
                .show_inside(ui, |ui| empty.show(ui, p, true));
            return;
        }
        // Head and foot as panels, the list in what is left: a panel
        // measures itself, so nothing here is a height measured by hand.
        egui::TopBottomPanel::top("library-head")
            .frame(bare)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.add_space(space::STEP);
                self.filter_row(ui, p);
                ui.add_space(space::STEP);
                rule(ui, p);
            });
        egui::TopBottomPanel::bottom("library-foot")
            .frame(bare)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                rule(ui, p);
                ui.add_space(space::STEP);
                self.foot(ui, p);
                ui.add_space(space::STEP);
            });
        egui::CentralPanel::default()
            .frame(bare)
            .show_inside(ui, |ui| self.list(ui, p));
    }

    /// Filter, show and sort — one row, gathered at the left.
    fn filter_row(&mut self, ui: &mut Ui, p: &Palette) {
        row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            let filter = ui.add(
                field(&mut self.state.filter, p)
                    .hint_text("Filter exports…  Ctrl+F")
                    .desired_width(FILTER_W),
            );
            if std::mem::take(&mut self.focus_filter) {
                filter.request_focus();
            }
            ui.label(meta("Show", p));
            egui::ComboBox::from_id_salt("library-show")
                .selected_text(text(self.state.show.label(), p))
                .width(170.0)
                .show_ui(ui, |ui| {
                    for option in Show::ALL {
                        ui.selectable_value(&mut self.state.show, option, text(option.label(), p));
                    }
                });
            ui.label(meta("Sort", p));
            egui::ComboBox::from_id_salt("library-sort")
                .selected_text(text(self.state.sort.label(), p))
                .width(190.0)
                .show_ui(ui, |ui| {
                    for option in Sort::ALL {
                        ui.selectable_value(&mut self.state.sort, option, text(option.label(), p));
                    }
                });
            // **Inside the messages, not the titles**: every database here,
            // earlier versions and deleted messages included. Enter searches —
            // it reads every message, and a scan per keystroke would queue one
            // per letter.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let typed = ui.add(
                    field(&mut self.search, p)
                        .hint_text("Search messages in every database…")
                        .desired_width(FILTER_W),
                );
                let needle = self.search.trim();
                if typed.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !needle.is_empty()
                {
                    if let Some(dir) = self.state.folder() {
                        self.search_all =
                            Some((needle.to_string(), dir.to_string_lossy().into_owned()));
                    }
                }
            });
        });
    }

    fn list(&mut self, ui: &mut Ui, p: &Palette) {
        let visible = self.state.visible();
        if visible.is_empty() {
            EmptyState::new(
                format!(
                    "Nothing matches \u{201c}{}\u{201d}",
                    self.state.filter.trim()
                ),
                Some("Clear the filter, or show every export.".into()),
            )
            .show(ui, p, true);
            return;
        }
        // Read once a frame and carried down, rather than one clock read per
        // row for an answer that cannot change between two of them.
        let now = SystemTime::now();
        egui::ScrollArea::vertical()
            .id_salt("library-list")
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, visible.len(), |ui, range| {
                for i in range {
                    self.export_row(ui, visible[i], now, p);
                }
            });
    }

    /// One export.
    fn export_row(&mut self, ui: &mut Ui, index: usize, now: SystemTime, p: &Palette) {
        let entry = self.state.entries[index].clone();
        let path = entry.target.path().to_path_buf();
        let ticked = self.state.selected.contains(&path);

        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
        // A ticked row is a filled row, and a hovered one too: the whole row is
        // the hit target, and a small box was the only thing saying which rows
        // were chosen.
        if ticked || response.hovered() {
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::ZERO, p.surface);
        }

        let mut act = None;
        inside(ui, rect, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Painted, not clickable: the row is the control.
            tick_mark(ui, ticked, p);
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // The row's one number, laid out first so a long title cannot
                // push it off the panel. Blank, not zero, until counted.
                let count = entry
                    .messages
                    .map(|n| tgx_ui::thousands(n as i64))
                    .unwrap_or_default();
                // Fixed columns, and the figure set flush right in its own:
                // `add_sized` centres, and a column of centred numbers does
                // not line its digits up.
                ui.allocate_ui_with_layout(
                    egui::vec2(COUNT_W, ROW_HEIGHT),
                    Layout::right_to_left(Align::Center),
                    |ui| {
                        // A child ui reports what its contents used, not what
                        // it was offered; this is what makes the column fixed.
                        ui.set_min_size(egui::vec2(COUNT_W, ROW_HEIGHT));
                        ui.label(figure(count, p));
                    },
                );
                ui.add_space(space::STEP);

                // **The accent is this run**, as in Activity: the row being
                // read now and nothing else. A row that needs a report is ink,
                // one that does not is quiet. A badge, not capitals: the
                // state is a word to read, in a box that says it is a state.
                let mark = entry.mark();
                ui.allocate_ui_with_layout(
                    egui::vec2(MARK_W, ROW_HEIGHT),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_min_size(egui::vec2(MARK_W, ROW_HEIGHT));
                        badge(ui, mark.label(), super::queue::mark_tone(mark), p);
                    },
                );
                ui.add_space(space::STEP);

                // **The row's three actions**, right to left so they read
                // Read · report · Show in folder. Each sits on top of the row
                // and takes its own click, so acting on a row does not also
                // tick it. Sentence case: they are verbs, not labels.
                if action(ui, meta("Show in folder", p), true).clicked() {
                    act = Some(RowAction::Reveal(entry.target.home()));
                }
                ui.add_space(space::STEP);
                // Open when there is a current report to open; make one when
                // there is not, or it is out of date. Off while any run goes:
                // one analyser at a time.
                match (mark, entry.report()) {
                    (Mark::Current, Some(report)) => {
                        if action(ui, text("Open report", p), true).clicked() {
                            act = Some(RowAction::Open(report.to_path_buf()));
                        }
                    }
                    _ => {
                        let idle = !self.state.running;
                        if action(ui, text("Make report", p), idle).clicked() {
                            act = Some(RowAction::Report(path.clone()));
                        }
                    }
                }
                ui.add_space(space::STEP);
                // Read is a database's alone — a folder export is Desktop's
                // HTML, which a browser already reads — and needs the chat id,
                // which arrives with the count.
                if let Some(chat_id) = entry.chat_id {
                    if action(ui, text("Read", p), true).clicked() {
                        act = Some(RowAction::Read(Read {
                            path: path.clone(),
                            chat_id,
                            title: entry.title.clone(),
                        }));
                    }
                }
                ui.add_space(space::STEP);

                // Title over caption, one block.
                ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(text(&entry.title, p)).truncate());
                    ui.add(egui::Label::new(meta(entry.caption(now), p)).truncate());
                });
            });
        });

        // The row's actions again, where a right-click looks, and the path.
        response.context_menu(|ui| {
            if let Some(chat_id) = entry.chat_id {
                if ui.button("Read").clicked() {
                    act = Some(RowAction::Read(Read {
                        path: path.clone(),
                        chat_id,
                        title: entry.title.clone(),
                    }));
                    ui.close();
                }
            }
            if let Some(report) = entry.report() {
                if ui.button("Open report").clicked() {
                    act = Some(RowAction::Open(report.to_path_buf()));
                    ui.close();
                }
            }
            if ui
                .add_enabled(!self.state.running, egui::Button::new("Make report"))
                .clicked()
            {
                act = Some(RowAction::Report(path.clone()));
                ui.close();
            }
            if ui.button("Show in folder").clicked() {
                act = Some(RowAction::Reveal(entry.target.home()));
                ui.close();
            }
            if ui.button("Copy path").clicked() {
                ui.ctx().copy_text(path.display().to_string());
                ui.close();
            }
        });

        match act {
            Some(RowAction::Open(report)) => self.open_report(&report),
            Some(RowAction::Report(path)) => self.start_one(&path, &ui.ctx().clone()),
            Some(RowAction::Read(read)) => self.read = Some(read),
            Some(RowAction::Reveal(dir)) => {
                if let Err(e) = open::reveal(&dir) {
                    self.state.status = e;
                }
            }
            None if response.clicked() => self.state.toggle(&path),
            None => {}
        }
    }

    /// The selection verbs and what is ticked — **and nothing else.**
    ///
    /// Embed fonts, Write the digest and Handles only sat here too, held for
    /// the session, so the foot mixed "which exports" with "what a report
    /// contains" and the second was forgotten at every launch. They are
    /// Settings › Reports now.
    fn foot(&mut self, ui: &mut Ui, p: &Palette) {
        row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            for (label, pick) in [
                ("All", Pick::All),
                ("None", Pick::None),
                ("Needs a report", Pick::Behind),
            ] {
                // Flat, as the chat list's are: these refine a selection the
                // rows already show, and boxes here would outweigh the bar.
                if action(ui, text(label, p), true).clicked() {
                    self.state.select(pick);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(figure(self.state.selection_label(), p));
            });
        });
    }

    /// The analyser's line for the window's status bar, while the Library
    /// shows: what it is doing, and how far through the run.
    ///
    /// It had a status line of its own, above the window's — two status
    /// lines, one over the other, the upper one in capitals. One bar, with
    /// whoever is speaking in it.
    pub(super) fn status(&self) -> (String, String) {
        let right = if self.state.running {
            format!("{} of {}", self.state.done, self.state.of)
        } else {
            String::new()
        };
        (self.state.status.clone(), right)
    }

    /// The run's fill, for the bar's rule. `None` between runs.
    pub(super) fn fill(&self) -> Option<f32> {
        self.state.fill()
    }
}

/// What the Library's primary says.
pub(super) fn report_label(ticked: usize, running: bool) -> String {
    match (running, ticked) {
        (true, _) => "Making reports".into(),
        (false, 0) => "Make report".into(),
        (false, 1) => "Make 1 report".into(),
        (false, n) => format!("Make {n} reports"),
    }
}

/// Lay a row's contents out inside the rect it was given, **vertically
/// centred**, between the panel's gutters. A layout given the row rect centres
/// in the row; `horizontal_centered` centres in a 30-point strip and leaves
/// every row sitting high in its own box — the chat list found that one.
fn inside<R>(ui: &mut Ui, rect: egui::Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + GUTTER, rect.top()),
        egui::pos2(rect.right() - GUTTER, rect.bottom()),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
        add,
    )
    .inner
}

#[cfg(test)]
mod tests {
    use eframe::egui::Color32;
    use tgx_ui::tokens::Palette;

    /// The report's own value for a token, as it writes it into the stylesheet.
    fn from_report(name: &str) -> Color32 {
        let text = tga_run::report_colour(name);
        let rgb = u32::from_str_radix(text.trim_start_matches('#'), 16).expect("hex");
        tgx_ui::tokens::hex(rgb)
    }

    #[test]
    fn the_dark_window_and_the_report_name_the_same_colours() {
        // The window's palette comes from `tgx-ui` and the report's from its
        // own stylesheet. If the dark palette moves, the report written here
        // and the window that wrote it stop being one product, and this is
        // where that shows. Dark only: the report has one appearance.
        let p = Palette::dark();
        for (name, ours) in [
            ("bg", p.bg),
            ("fg", p.fg),
            ("muted", p.muted),
            ("hairline", p.hairline),
            ("rule", p.rule),
            ("surface", p.surface),
            ("accent", p.accent),
            ("accent_fg", p.accent_fg),
        ] {
            assert_eq!(
                ours,
                from_report(name),
                "{name} has drifted from the report's stylesheet"
            );
        }
    }

    #[test]
    fn a_launch_that_never_opens_the_library_scans_nothing() {
        let library = super::Library::default();
        assert!(library.followed.is_none());
        assert!(library.state.root.is_none());
        assert!(!library.busy());
    }
}
