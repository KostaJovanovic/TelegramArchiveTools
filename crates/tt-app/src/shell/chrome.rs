//! The frame around the panels: the rail, the top bar and the status bar.
//!
//! The panels themselves are the sibling modules -- chats, queue, settings,
//! signin, viewer, library.

use super::*;
use eframe::egui::{Layout, Ui};
use tgx_ui::components::{action, caps, edge_rule, eyebrow, row, rule, title, NavCell};
use tgx_ui::tokens::{space, window};

/// How wide the rail is: room for `LIBRARY  12` in tracked capitals and the
/// account's name under it, and not a point more — every point here comes out
/// of the chat list's title column.
pub(crate) const RAIL_WIDTH: f32 = 168.0;

impl Shell {
    /// **Where you are, down the left; who you are, at its foot.**
    ///
    /// The views used to be tabs on the top bar, beside the action buttons,
    /// and two kinds of control on one line is what the complaint about that
    /// bar was: tabs choose what the body shows and change nothing, buttons
    /// act. With five views and a second tool in the window, the tabs would
    /// have crowded the actions off the bar. A rail gives each its own edge.
    ///
    /// **The selected one is ink with a rule beside it; the others are
    /// muted.** The tab strip's underline, turned on its side. Each cell
    /// carries its own count, because the reason to look at a view you are
    /// not in is usually to find out whether it has anything in it.
    pub(super) fn rail(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let current = self.body;
        let mut chosen = None;

        // The rail's own right edge, the full height — the hairline that says
        // where the views end and the view begins.
        let edge = ui.max_rect();
        ui.painter().vline(
            edge.right() - 0.5,
            edge.y_range(),
            egui::Stroke::new(1.0_f32, p.hairline),
        );

        ui.add_space(space::BREAK);
        row(ui, |ui| ui.label(title(crate::TITLE, &p)));
        ui.add_space(space::BREAK);
        for view in View::ALL {
            let selected = view == current;
            let ink = if selected { p.fg } else { p.muted };
            let count = match view {
                View::Chats if !self.chats.is_empty() => Some(self.chats.len()),
                View::Queue if !self.queue.is_empty() => Some(self.queue.len()),
                View::Library if !self.library.state.entries.is_empty() => {
                    Some(self.library.state.entries.len())
                }
                _ => None,
            };
            let mut label = view.label().to_string();
            if let Some(n) = count {
                label.push_str(&format!("  {n}"));
            }
            let hit = row(ui, |ui| action(ui, caps(&label, ink), true));
            if selected {
                ui.painter().vline(
                    edge.left() + space::TIGHT,
                    hit.rect.y_range(),
                    egui::Stroke::new(2.0_f32, p.fg),
                );
            }
            if hit.clicked() {
                chosen = Some(view);
            }
            ui.add_space(space::TIGHT);
        }

        // The account, at the foot: who the exports are of, and the way in
        // when nobody is.
        ui.with_layout(Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(space::STEP);
            let busy = self.exporting || self.counting;
            match self.account.clone() {
                Some(name) => {
                    row(ui, |ui| {
                        ui.add(egui::Label::new(tgx_ui::components::text(name, &p)).truncate())
                    });
                    row(ui, |ui| ui.label(eyebrow("Signed in", &p)));
                }
                None => {
                    let enabled = !busy && self.login.is_none();
                    if row(ui, |ui| {
                        NavCell::tool("Sign in").enabled(enabled).show(ui, &p)
                    })
                    .clicked()
                    {
                        self.start_sign_in();
                    }
                    row(ui, |ui| ui.label(eyebrow("Not signed in", &p)));
                }
            }
            ui.add_space(space::TIGHT);
            rule(ui, &p);
        });

        if let Some(view) = chosen {
            self.show(view);
        }
    }

    /// **One bar: what this view is on the left, what to do on the right.**
    ///
    /// It was two stacked strips — five buttons in the first, three tab labels
    /// in the second — and the complaint they earned was that the top of the
    /// window read as two menus with no way to tell which was which. The tabs
    /// have gone to the rail; what is left here is the view's name and its
    /// actions, so the bar changes with the view.
    ///
    /// **The export sequence keeps its numbers.** `01`–`03` across Sign in,
    /// Refresh and Start export is the one thing on this bar that tells a
    /// first-time user there is an order. Stop and Open folder are tools:
    /// unnumbered, and last. The Library has its own actions, and none of the
    /// export's.
    ///
    /// **The appearance chip moved into Settings.** It was a sixth kind of mark
    /// on a bar that was already carrying too many, and it is a stored setting
    /// with a home — see `settings::appearance_section`.
    pub(super) fn top_bar(&mut self, ui: &mut Ui, ctx: &Context) {
        let p = self.palette;
        if self.body == View::Library {
            let body = egui::vec2(ui.available_width(), ui.available_height() - 1.0);
            ui.allocate_ui_with_layout(body, Layout::left_to_right(egui::Align::Center), |ui| {
                row(ui, |ui| {
                    ui.label(caps(View::Library.label(), p.fg));
                    ui.add_space(space::BREAK);
                    self.library.bar(ui, ctx, &p);
                });
            });
            edge_rule(ui, &p);
            return;
        }
        let busy = self.exporting || self.counting;

        // **Start export becomes Add to queue while a run is going**, because
        // that is what pressing it does now: the ticked chats join the run in
        // flight rather than replacing it. Same slot, same number, so the
        // control does not move under the pointer — only its promise changes,
        // and it changes to the true one.
        //
        // **And Resume queue after a Stop**, which carries on the paused rows
        // — so it needs no ticks of its own; any that are set join the end.
        let paused = !self.exporting && self.queue.is_paused();
        let start = if self.exporting {
            "Add to queue"
        } else if paused {
            "Resume queue"
        } else {
            "Start export"
        };
        let steps = [
            // Disabled while the dialog is up: a second click would spawn a
            // second `Session::connect` behind the one already running, and
            // `open_login` would raise the existing dialog while the extra
            // connection carried on in the background.
            NavCell::step(1, "Sign in").enabled(!busy && self.login.is_none()),
            NavCell::step(2, "Refresh").enabled(self.signed_in && !busy),
            // **The one red in the window.** This is the button the application
            // exists for; everything else on the bar leads to it or cleans up
            // after it. See `components::button`.
            NavCell::step(3, start)
                .enabled(self.signed_in && (paused || !self.selected.is_empty()) && !self.counting)
                .primary(true),
        ];
        let tools = [
            NavCell::tool("Stop").enabled(busy),
            NavCell::tool("Open folder"),
        ];

        // **The row is given the bar's height, not `interact_size`'s.** The
        // panel is `NAV_HEIGHT` tall, but `Ui::horizontal` allocates a strip one
        // `interact_size.y` high and centres within *that* — so the buttons sat
        // in the top 30 points with their edges against the window, the rule
        // below them landed at 33, and the remaining 27 points of the bar were
        // empty. Laying out inside a rect of the full height puts them in the
        // middle of it and the rule at the bottom, where a rule under a bar goes.
        let body = egui::vec2(ui.available_width(), ui.available_height() - 1.0);
        ui.allocate_ui_with_layout(body, Layout::left_to_right(egui::Align::Center), |ui| {
            row(ui, |ui| {
                ui.label(caps(self.body.label(), p.fg));

                // Laid out right-to-left from the far end, so the tools finish
                // the line and the sequence keeps the middle. Pushed in reverse
                // for that reason: what is listed first lands furthest right.
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    for (i, cell) in tools.iter().enumerate().rev() {
                        if cell.show(ui, &p).clicked() {
                            match i {
                                0 => self.stop(),
                                // Reported, not discarded. A silent spawn
                                // failure is what let `explorer.exe` be looked
                                // for in System32 — where it is not — for as
                                // long as nobody happened to watch the button do
                                // nothing.
                                1 => {
                                    if let Err(e) = crate::actions::open_output_folder(
                                        &self.settings.output_dir,
                                    ) {
                                        self.journal.warn(e);
                                        self.log_copied = false;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    ui.add_space(space::STEP);
                    for (i, cell) in steps.iter().enumerate().rev() {
                        if cell.show(ui, &p).clicked() {
                            match i {
                                0 => self.start_sign_in(),
                                1 => self.start_refresh(),
                                2 if paused => {
                                    let picked = self.ticked_chats();
                                    self.resume_queue(picked);
                                }
                                2 => self.start_export(),
                                _ => {}
                            }
                        }
                    }
                });
            });
        });
        edge_rule(ui, &p);
    }

    /// Swap the appearance.
    ///
    /// **A token swap, not a rebuild.** Every colour in this window comes from
    /// the palette, so re-reading it repaints the lot. Under GPUI there was a
    /// second call here for the borrowed components, which painted from
    /// `gpui-component`'s own theme global rather than from ours; nothing is
    /// borrowed now, so the style is reinstalled on the next frame from the one
    /// palette and there is no second vocabulary to keep in step.
    pub(super) fn set_theme(&mut self, name: &str) {
        if self.settings.theme == name {
            return;
        }
        self.settings.theme = name.into();
        self.palette = Palette::named(&self.settings.theme);
        self.theme_stale = true;
        self.commit_settings();
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        let p = self.palette;
        // Counting reports in chats and an export in messages, and the export
        // owns the line whenever it is running.
        let right = if self.exporting {
            self.queue
                .fraction()
                .map(|f| format!("{:.0}%", f * 100.0))
                .unwrap_or_default()
        } else {
            match self.count_progress {
                Some((done, total)) => format!("{done} of {total}"),
                None => String::new(),
            }
        };

        edge_rule(ui, &p);
        ui.add_space(space::TIGHT);
        // **The nested left-to-right is load-bearing, and taking it out was a
        // regression.** The figure is the fixed part, so it is allocated first
        // from the right; the status has to be laid out *forwards* in what
        // remains, or a truncating label in a right-to-left layout fills the
        // panel and draws its text against the far edge — which is what put the
        // status line on the right-hand side of the window with nothing at all
        // on the left.
        row(ui, |ui| {
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                // The right-hand figure is ours and short, so it can be tracked.
                ui.label(eyebrow(&right, &p));
                ui.with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                    // **In the mono, not letterspaced.** `eyebrow` goes through
                    // `components::tracked`, whose own doc says not to put a
                    // string of unknown length through it, and this line carries
                    // chat titles and raw RPC errors. The mono also keeps a line
                    // that changes several times a second from changing width
                    // under every character.
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(tgx_ui::components::uppercase(&self.status))
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
