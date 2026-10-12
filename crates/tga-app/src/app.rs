//! The window: a library of exports, and a report for each.
//!
//! **A worker event repaints the window; the window never polls for one.** The
//! job hands back a `std::sync::mpsc::Receiver`, which can only be polled — so
//! the worker also holds a clone of the egui context and calls
//! `request_repaint()` on every message. Without that, nothing appears until
//! some unrelated input causes a frame, which the user experiences as having to
//! move the mouse to make the app work.
//!
//! Everything this window *decides* is in [`crate::state`]; what is here is
//! where things sit and what they look like. **What they look like is the
//! exporter's**: the bar, the rows, the footer and the status line are laid out
//! the way `TelegramExporter`'s chat list lays out its own, from the same
//! `tgx-ui` pieces, so moving from one window to the other changes what the
//! rows are and nothing about how to read them.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::SystemTime;

use eframe::egui::{self, Align, Layout, Sense, Ui};
use tgx_ui::components::{
    action, caps, edge_rule, eyebrow, field, figure, help, progress_bar, row, rule, text, tick_box,
    tick_mark, title, uppercase, EmptyState, NavCell, GUTTER,
};
use tgx_ui::tokens::{metrics, space, window};

use crate::job::{self, Counted, Request};
use crate::open;
use crate::refresh::Source;
use crate::state::{Mark, Pick, Show, Sort, State};
use crate::theme::{self, PALETTE};

pub const TITLE: &str = "Telegram Export Analyser";

/// Every row is this tall — the exporter's chat row, so a title over a caption
/// sits the same in both windows.
const ROW_HEIGHT: f32 = 46.0;

/// How wide the filter box is. **Fixed**, for the exporter's reason: a field
/// stretched across the window advertises room for a sentence, and what goes
/// in it is a word.
const FILTER_W: f32 = 260.0;

/// The count column: wide enough for `1,486,313`.
const COUNT_W: f32 = 80.0;

/// The mark column: wide enough for `UP TO DATE`, the longest of the six.
const MARK_W: f32 = 96.0;

pub struct Shell {
    state: State,
    /// The run in progress. `None` between runs.
    run: Option<job::Run>,
    /// Titles and counts arriving for the rows. `None` once the counter has
    /// finished and hung up.
    counts: Option<Receiver<Counted>>,
    /// For `shot`: frames drawn, and the last one with work in flight.
    frame_no: u32,
    busy_until: u32,
}

impl Shell {
    pub fn new(ctx: &egui::Context) -> Self {
        theme::install(ctx);
        let mut shell = Shell {
            state: State::default(),
            run: None,
            counts: None,
            frame_no: 0,
            busy_until: 0,
        };
        if let Some(folder) = crate::shot::folder() {
            shell.set_path(Path::new(&folder), ctx);
        }
        shell
    }

    /// Re-read the folder in the field, and send whatever is new off to be
    /// counted.
    fn rescan(&mut self, ctx: &egui::Context) {
        let todo = self.state.rescan();
        self.count(todo, ctx);
    }

    fn set_path(&mut self, path: &Path, ctx: &egui::Context) {
        let todo = self.state.set_path(path);
        self.count(todo, ctx);
    }

    fn count(&mut self, todo: Vec<Source>, ctx: &egui::Context) {
        if !todo.is_empty() {
            // Replacing the receiver drops the last one: a count still running
            // for a folder no longer on screen sends into nothing and stops.
            self.counts = Some(job::count(todo, self.state.generation, ctx.clone()));
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        let Some(targets) = self.state.start() else {
            return;
        };
        let request = Request {
            targets,
            write_digest: self.state.write_digest,
            embed_fonts: self.state.embed_fonts,
            handles_only: self.state.handles_only,
        };
        self.run = Some(job::spawn(request, ctx.clone()));
    }

    fn stop(&mut self) {
        if let Some(run) = &self.run {
            run.stop();
            self.state.stopping = true;
            self.state.status = "Stopping after this export".into();
        }
    }

    /// Apply everything the workers have said since the last frame.
    ///
    /// In a batch: a run emits one message per topic it reads, and a ten-topic
    /// export would otherwise be ten frames nobody sees.
    fn drain(&mut self) {
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

    /// A folder — or a database — dropped anywhere on the window.
    ///
    /// The window is the target rather than the field, which is a small one.
    /// The first directory wins, or the first file that is a database: those
    /// are the two things an export can be.
    fn dropped(&mut self, ctx: &egui::Context) {
        if self.state.running {
            return;
        }
        let dirs: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .filter(|p| p.is_dir() || tga_db::find(p).is_some())
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

    /// **One bar: what this is on the left, what to do on the right.** The
    /// exporter's arrangement, with the path field in the middle where its
    /// view tabs sit — the one thing this window is ever pointed at.
    fn top_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let p = PALETTE;
        let running = self.state.running;
        let ticked = self.state.selected.len();
        let analyse = match (running, ticked) {
            (true, _) => "Analysing".to_string(),
            (false, 0) => "Analyse".to_string(),
            (false, n) => format!("Analyse {n}"),
        };
        let root = self.state.root.clone().filter(|r| r.is_dir());

        // The row is given the bar's height, not `interact_size`'s, so the
        // buttons centre in the bar and the rule lands at its foot. See the
        // exporter's `chrome::top_bar`, which learned it first.
        let body = egui::vec2(ui.available_width(), ui.available_height() - 1.0);
        ui.allocate_ui_with_layout(body, Layout::left_to_right(Align::Center), |ui| {
            row(ui, |ui| {
                ui.label(title(TITLE, &p));
                ui.add_space(space::BREAK);
                // Right to left from the far end, so what is listed first lands
                // furthest right and the field takes whatever is left.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // **The one red.** Everything else on the bar leads to it.
                    if NavCell::tool(analyse)
                        .enabled(self.state.ready())
                        .primary(true)
                        .show(ui, &p)
                        .clicked()
                    {
                        self.start(ctx);
                    }
                    if NavCell::tool("Stop")
                        .enabled(running && !self.state.stopping)
                        .show(ui, &p)
                        .clicked()
                    {
                        self.stop();
                    }
                    ui.add_space(space::STEP);
                    if NavCell::tool("Open folder")
                        .enabled(root.is_some())
                        .show(ui, &p)
                        .clicked()
                    {
                        if let Some(dir) = &root {
                            if let Err(e) = open::reveal(dir) {
                                self.state.status = e;
                            }
                        }
                    }
                    if NavCell::tool("Choose")
                        .enabled(!running)
                        .show(ui, &p)
                        .clicked()
                    {
                        self.browse(ctx);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let width = ui.available_width();
                        // Pasting a path out of Explorer is how this window is
                        // actually used, so the field is a field and not a
                        // label beside a Choose button.
                        let edited = ui
                            .add_enabled(
                                !running,
                                field(&mut self.state.path, &p)
                                    .hint_text(r"C:\...\telegram export")
                                    .desired_width(width),
                            )
                            .changed();
                        if edited {
                            self.rescan(ctx);
                        }
                    });
                });
            });
        });
        edge_rule(ui, &p);
    }

    /// Filter, show and sort — one row, gathered at the left.
    fn filter_row(&mut self, ui: &mut Ui) {
        let p = PALETTE;
        row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            ui.add(
                field(&mut self.state.filter, &p)
                    .hint_text("Filter exports…")
                    .desired_width(FILTER_W),
            );
            ui.label(eyebrow("Show", &p));
            egui::ComboBox::from_id_salt("show")
                .selected_text(text(self.state.show.label(), &p))
                .width(170.0)
                .show_ui(ui, |ui| {
                    for option in Show::ALL {
                        ui.selectable_value(&mut self.state.show, option, text(option.label(), &p));
                    }
                });
            ui.label(eyebrow("Sort", &p));
            egui::ComboBox::from_id_salt("sort")
                .selected_text(text(self.state.sort.label(), &p))
                .width(190.0)
                .show_ui(ui, |ui| {
                    for option in Sort::ALL {
                        ui.selectable_value(&mut self.state.sort, option, text(option.label(), &p));
                    }
                });
        });
    }

    fn list(&mut self, ui: &mut Ui) {
        let p = PALETTE;
        let visible = self.state.visible();
        if visible.is_empty() {
            EmptyState::new(
                format!(
                    "Nothing matches \u{201c}{}\u{201d}",
                    self.state.filter.trim()
                ),
                Some("Clear the filter, or show every export.".into()),
            )
            .show(ui, &p, true);
            return;
        }
        // Read once a frame and carried down, rather than one clock read per
        // row for an answer that cannot change between two of them.
        let now = SystemTime::now();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, visible.len(), |ui, range| {
                for i in range {
                    self.export_row(ui, visible[i], now);
                }
            });
    }

    /// One export.
    fn export_row(&mut self, ui: &mut Ui, index: usize, now: SystemTime) {
        let p = PALETTE;
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

        let mut open = None;
        inside(ui, rect, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Painted, not clickable: the row is the control.
            tick_mark(ui, ticked, &p);
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
                        ui.label(figure(count, &p));
                    },
                );
                ui.add_space(space::STEP);

                // **The accent is this run**, as in the exporter: the row being
                // read now and nothing else. A row that needs a report is ink,
                // one that does not is muted.
                let mark = entry.mark();
                let ink = match mark {
                    Mark::Running => p.accent,
                    Mark::Failed | Mark::NoReport | Mark::Changed => p.fg,
                    Mark::Queued | Mark::Current => p.muted,
                };
                ui.allocate_ui_with_layout(
                    egui::vec2(MARK_W, ROW_HEIGHT),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_min_size(egui::vec2(MARK_W, ROW_HEIGHT));
                        ui.label(caps(mark.label(), ink));
                    },
                );
                ui.add_space(space::STEP);

                // Open sits on top of the row and takes its own click, so
                // opening a report does not also tick the row.
                if let Some(report) = entry.report() {
                    if action(ui, caps("Open", p.fg), true).clicked() {
                        open = Some(report.to_path_buf());
                    }
                }
                ui.add_space(space::STEP);

                // Title over caption, one block.
                ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(text(&entry.title, &p)).truncate());
                    ui.add(egui::Label::new(caps(&entry.caption(now), p.muted)).truncate());
                });
            });
        });

        if let Some(report) = open {
            self.open_report(&report);
        } else if response.clicked() {
            self.state.toggle(&path);
        }
    }

    /// The selection verbs, the run's options, and what is ticked.
    fn foot(&mut self, ui: &mut Ui) {
        let p = PALETTE;
        let running = self.state.running;
        row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            for (label, pick) in [
                ("All", Pick::All),
                ("None", Pick::None),
                ("Needs a report", Pick::Behind),
            ] {
                // Flat, as the exporter's are: these refine a selection the
                // rows already show, and boxes here would outweigh the bar.
                if action(ui, caps(label, p.fg), true).clicked() {
                    self.state.select(pick);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Right to left, so each option is its mark, then its label,
                // then its box — read left to right as box, label, mark.
                // **Box and label are one control**, as in the exporter: the
                // words you read are the thing you click.
                help(
                    ui,
                    "fonts",
                    "Geist is written into every report, so it looks right on a \
                     machine that does not have it. Off, each report is about \
                     300 KB smaller.",
                    &p,
                );
                let fonts = action(ui, eyebrow("Embed fonts", &p), !running).union(tick_box(
                    ui,
                    self.state.embed_fonts,
                    !running,
                    &p,
                ));
                if fonts.clicked() {
                    self.state.embed_fonts = !self.state.embed_fonts;
                }
                ui.add_space(space::STEP);
                help(
                    ui,
                    "digest",
                    "Also writes analysis/digest.jsonl and EVENTS.md beside each \
                     export: every message as one line, with each attachment's \
                     text and date, which is what a model needs to map events \
                     onto the timeline.",
                    &p,
                );
                let digest = action(ui, eyebrow("Write the digest", &p), !running).union(tick_box(
                    ui,
                    self.state.write_digest,
                    !running,
                    &p,
                ));
                if digest.clicked() {
                    self.state.write_digest = !self.state.write_digest;
                }
                ui.add_space(space::STEP);
                help(
                    ui,
                    "handles",
                    "Shows everyone with an @handle by it instead of by name. \
                     For your contacts an export holds the name you saved them \
                     as, which nobody else should see. Somebody with no handle \
                     keeps the name in the export.",
                    &p,
                );
                let handles = action(ui, eyebrow("Handles only", &p), !running).union(tick_box(
                    ui,
                    self.state.handles_only,
                    !running,
                    &p,
                ));
                if handles.clicked() {
                    self.state.handles_only = !self.state.handles_only;
                }
            });
        });
        ui.add_space(space::TIGHT);
        row(ui, |ui| {
            ui.label(eyebrow(&self.state.selection_label(), &p));
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let p = PALETTE;
        // The bar takes the rule's place while a run goes, at the same height
        // either way, so the status line does not jump when one starts.
        match self.state.fill() {
            Some(fraction) => progress_bar(ui, Some(fraction), &p),
            None => {
                edge_rule(ui, &p);
                ui.add_space(2.0);
            }
        }
        ui.add_space(space::TIGHT);
        let right = if self.state.running {
            format!("{} of {}", self.state.done, self.state.of)
        } else {
            String::new()
        };
        // The figure is the fixed part, so it is allocated first from the
        // right and the status is laid out forwards in what remains — the
        // exporter's status bar, and the nesting is load-bearing there too.
        row(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(eyebrow(&right, &p));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(uppercase(&self.state.status))
                                .font(tgx_ui::fonts::mono(window::LABEL))
                                .color(p.muted),
                        )
                        .truncate(),
                    );
                });
            });
        });
        ui.add_space(space::TIGHT);
    }
}

impl eframe::App for Shell {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        PALETTE.bg.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        self.dropped(ctx);

        let p = PALETTE;
        let bare = egui::Frame::NONE.fill(p.bg);

        egui::TopBottomPanel::top("bar")
            .frame(bare)
            .exact_height(metrics::NAV_HEIGHT)
            .show_separator_line(false)
            .show(ctx, |ui| self.top_bar(ui, ctx));

        egui::TopBottomPanel::bottom("status")
            .frame(bare)
            .show_separator_line(false)
            .show(ctx, |ui| self.status_bar(ui));

        egui::CentralPanel::default().frame(bare).show(ctx, |ui| {
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
                        "Nothing to analyse here",
                        Some(
                            "No result.json under it, and no .sqlite in it or in the folders \
                             directly inside it."
                                .into(),
                        ),
                    )
                };
                empty.show(ui, &p, true);
                return;
            }
            // Head and foot as panels, the list in what is left: a panel
            // measures itself, so nothing here is a height measured by hand.
            egui::TopBottomPanel::top("head")
                .frame(bare)
                .show_separator_line(false)
                .show_inside(ui, |ui| {
                    ui.add_space(space::STEP);
                    self.filter_row(ui);
                    ui.add_space(space::STEP);
                    rule(ui, &p);
                });
            egui::TopBottomPanel::bottom("foot")
                .frame(bare)
                .show_separator_line(false)
                .show_inside(ui, |ui| {
                    rule(ui, &p);
                    ui.add_space(space::STEP);
                    self.foot(ui);
                    ui.add_space(space::STEP);
                });
            egui::CentralPanel::default()
                .frame(bare)
                .show_inside(ui, |ui| self.list(ui));
        });

        // After everything has been drawn, so the frame egui hands back is the
        // finished one — and only once the counts are in, or the picture is of
        // a list with its figures still blank.
        self.frame_no += 1;
        if let Some(to) = crate::shot::path() {
            if self.counts.is_some() || self.run.is_some() {
                self.busy_until = self.frame_no;
            }
            crate::shot::tick(ctx, self.frame_no - self.busy_until, &to);
        }
    }
}

/// Lay a row's contents out inside the rect it was given, **vertically
/// centred**, between the panel's gutters. A layout given the row rect centres
/// in the row; `horizontal_centered` centres in a 30-point strip and leaves
/// every row sitting high in its own box — the exporter found that one.
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
