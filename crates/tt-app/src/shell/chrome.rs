//! The frame around the panels: the rail, the top bar and the status bar.
//!
//! The panels themselves are the sibling modules -- chats, queue, settings,
//! signin, viewer, analysis, summaries.

use super::*;
use eframe::egui::{Layout, Ui};
use tgx_ui::components::{
    action, button, edge_rule, figure, meta, nav_item, row, rule, text, title,
};
use tgx_ui::tokens::{space, window, Palette};

/// A view's name on the top bar: the reading face, medium, in ink — a
/// heading in sentence case rather than one more line of tracked capitals.
pub(super) fn heading(label: &str, p: &Palette) -> egui::RichText {
    egui::RichText::new(label)
        .font(tgx_ui::fonts::medium(window::READING))
        .color(p.fg)
}

/// What Chats' primary says, given the ticks and the run.
///
/// The number is in the label because it is the decision: "Export" over
/// four hundred ticked chats and over one are different buttons to press.
pub(super) fn export_label(ticked: usize, exporting: bool, paused: bool) -> String {
    let chats = |n: usize| if n == 1 { "chat" } else { "chats" };
    if exporting {
        if ticked == 0 {
            "Add to queue".into()
        } else {
            format!("Add {ticked} to queue")
        }
    } else if paused {
        "Resume".into()
    } else if ticked == 0 {
        "Export".into()
    } else {
        format!("Export {ticked} {}", chats(ticked))
    }
}

/// How wide the rail is: room for `ANALYSIS  12` in tracked capitals and the
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
        let current = self.body.on_rail();
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
        // Edge to edge: the hover fill and the bar are the rail's, not a
        // gutter's, so the entries are not put through `row`.
        let width = ui.available_width() - 1.0;
        for view in View::ALL {
            let count = match view {
                View::Chats if !self.chats.is_empty() => Some(self.chats.len()),
                View::Queue if !self.queue.is_empty() => Some(self.queue.len()),
                View::Analysis if !self.analysis.state.entries.is_empty() => {
                    Some(self.analysis.state.entries.len())
                }
                _ => None,
            };
            let hit = ui
                .allocate_ui(egui::vec2(width, 0.0), |ui| {
                    nav_item(ui, view.label(), count, view == current, &p)
                })
                .inner;
            if hit.clicked() {
                chosen = Some(view);
            }
        }

        // The account, at the foot: who the exports are of, and the way in
        // when nobody is.
        ui.with_layout(Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(space::STEP);
            let busy = self.exporting || self.counting;
            match self.account.clone() {
                Some(name) => {
                    // Bottom-up, so this is drawn under the name. Quiet, like
                    // every other link here; the second press is the one that
                    // acts, and the label says so between them.
                    let label = if self.signing_out {
                        "Signing out…"
                    } else if self.sign_out_armed {
                        "Press again to sign out"
                    } else {
                        "Sign out"
                    };
                    let enabled = !busy && !self.signing_out;
                    if row(ui, |ui| action(ui, meta(label, &p), enabled))
                        .on_hover_text(
                            "Ends this session at Telegram and deletes it from this \
                             computer. Signing in again needs a new code.",
                        )
                        .clicked()
                    {
                        self.sign_out();
                    }
                    row(ui, |ui| ui.add(egui::Label::new(text(name, &p)).truncate()));
                    row(ui, |ui| ui.label(meta("Signed in as", &p)));
                }
                None => {
                    let enabled = !busy && self.login.is_none();
                    if row(ui, |ui| button(ui, "Sign in", enabled, false, &p)).clicked() {
                        self.start_sign_in();
                    }
                    ui.add_space(space::TIGHT);
                    row(ui, |ui| ui.label(meta("Not signed in", &p)));
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
    /// **One primary action per view, top right** (TELEGRAMTOOLS.md §4.2).
    /// Chats has Export, Activity has Stop, Analysis has Make report;
    /// Settings and the reader have none.
    ///
    /// **The numbered steps are gone.** `01 Sign in / 02 Refresh / 03 Start
    /// export` told a first-time user there was an order, but it put an
    /// account action, a list action and the run on one strip, on every view,
    /// beside a rail that already showed the account. Sign in lives at the
    /// rail's foot — and is Chats' primary while nobody is signed in, because
    /// then it is the only thing Chats can do. Reload went beside the list it
    /// reloads.
    ///
    /// **The appearance chip moved into Settings.** It was a sixth kind of mark
    /// on a bar that was already carrying too many, and it is a stored setting
    /// with a home — see `settings::appearance_section`.
    pub(super) fn top_bar(&mut self, ui: &mut Ui, ctx: &Context) {
        let p = self.palette;
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
                ui.label(heading(self.body.label(), &p));
                ui.add_space(space::BREAK);
                match self.body {
                    View::Analysis => self.analysis.bar(ui, ctx, &p),
                    View::Chats => self.chats_actions(ui),
                    View::Queue => self.activity_actions(ui),
                    View::Summaries => self.summaries_actions(ui, ctx),
                    View::Settings | View::Viewer => {}
                }
            });
        });
        edge_rule(ui, &p);
    }

    /// Chats: Stop while something runs, then the one red.
    fn chats_actions(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let busy = self.exporting || self.counting;
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if !self.signed_in {
                // Disabled while the dialog is up: a second click would spawn a
                // second `Session::connect` behind the one already running.
                let enabled = !busy && self.login.is_none();
                if button(ui, "Sign in", enabled, true, &p).clicked() {
                    self.start_sign_in();
                }
                return;
            }
            // **Export becomes Add to queue while a run is going**, because that
            // is what pressing it does now: the ticked chats join the run in
            // flight. Same slot, so the control does not move under the pointer
            // — only its promise changes, and it changes to the true one.
            //
            // **And Resume after a Stop**, which carries on the paused rows —
            // so it needs no ticks of its own; any that are set join the end.
            let paused = !self.exporting && self.queue.is_paused();
            let label = export_label(self.selected.len(), self.exporting, paused);
            let enabled = (paused || !self.selected.is_empty()) && !self.counting;
            // **The one red in the window.** This is the button the
            // application exists for. See `components::button`.
            if button(ui, &label, enabled, true, &p)
                .on_hover_text("Ctrl+E")
                .clicked()
            {
                if paused {
                    let picked = self.ticked_chats();
                    self.resume_queue(picked);
                } else {
                    self.start_export();
                }
            }
            if busy && button(ui, "Stop", true, false, &p).clicked() {
                self.stop();
            }
        });
    }

    /// Activity: Stop is the primary, Open folder the tool beside it.
    ///
    /// **Stop stops everything this view shows** — the export and the report
    /// run both, since both are on it. The report run stops after the report
    /// in hand, as Analysis's own Stop does.
    fn activity_actions(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let exporting = self.exporting || self.counting;
        let reporting = self.analysis.state.running && !self.analysis.state.stopping;
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if button(ui, "Stop", exporting || reporting, true, &p).clicked() {
                if exporting {
                    self.stop();
                }
                if reporting {
                    self.analysis.stop();
                }
            }
            // Reported, not discarded. A silent spawn failure is what let
            // `explorer.exe` be looked for in System32 — where it is not — for
            // as long as nobody happened to watch the button do nothing.
            if button(ui, "Open folder", true, false, &p).clicked() {
                if let Err(e) = crate::actions::open_output_folder(&self.settings.output_dir) {
                    self.journal.warn(e);
                    self.log_copied = false;
                }
            }
        });
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
        // System is resolved on the next frame, by `follow_system_theme`,
        // which has the context to ask Windows with.
        if name != "system" {
            self.palette = Palette::named(&self.settings.theme);
            self.theme_stale = true;
        }
        self.commit_settings();
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        let p = self.palette;
        // Counting reports in chats and an export in messages, and the export
        // owns the line whenever it is running. **The analyser speaks here
        // too** — while Analysis is on screen, or while a report run goes
        // and nothing of the exporter's does. It had a status line of its own
        // stacked above this one.
        let reporting = !self.exporting
            && self.count_progress.is_none()
            && (self.body == View::Analysis || self.analysis.fill().is_some());
        // The Summaries view speaks for itself while it is on screen and the
        // exporter is quiet.
        let summarising = self.body == View::Summaries && !self.exporting;
        let (status, right) = if summarising {
            self.summaries_status()
        } else if reporting {
            self.analysis.status()
        } else if self.exporting {
            let right = self
                .queue
                .fraction()
                .map(|f| format!("{:.0}%", f * 100.0))
                .unwrap_or_default();
            (self.status.clone(), right)
        } else {
            let right = match self.count_progress {
                Some((done, total)) => format!("{done} of {total}"),
                None => String::new(),
            };
            (self.status.clone(), right)
        };

        // A report run's bar takes the rule's place, at the same height
        // either way, so the line under it does not jump when one starts.
        match self.analysis.fill().filter(|_| reporting && !summarising) {
            Some(fraction) => tgx_ui::components::progress_bar(ui, Some(fraction), &p),
            None => edge_rule(ui, &p),
        }
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
                ui.label(figure(&right, &p));
                ui.with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                    // **In the mono, and as written.** It was uppercased, which
                    // is capitals carrying data — and this line carries chat
                    // titles and raw RPC errors, whose case is part of what
                    // they say. The mono keeps a line that changes several
                    // times a second from changing width under every character.
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&status)
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
