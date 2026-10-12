//! What a run is doing: the queue table, the progress bar and the log.
//!
//! **This is the panel; `crate::queue` is the model it paints.** Same word,
//! two layers: `Queue` and `Job` hold the state and are testable without a
//! window, and everything here is layout and clicks.

use super::*;
use crate::queue::JobState;
use eframe::egui::{Align, Layout, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use tga_run::state::{Mark, Outcome};
use tgx_ui::components::{
    action, badge, block, dismiss, eyebrow, figure, meta, progress_bar, row, rule, text, thousands,
    title, EmptyState, Tone,
};
use tgx_ui::tokens::{space, window};

/// How loud an export row's state is: the accent for the one exporting,
/// ink for one that wants a decision, quiet for the rest.
fn job_tone(state: &JobState) -> Tone {
    match state {
        JobState::Exporting => Tone::Accent,
        JobState::Paused | JobState::Failed(_) => Tone::Ink,
        JobState::Queued | JobState::Done => Tone::Quiet,
    }
}

/// The same for a Library row: shared with the Library, which paints the
/// same marks.
pub(super) fn mark_tone(mark: Mark) -> Tone {
    match mark {
        Mark::Running => Tone::Accent,
        Mark::Failed | Mark::NoReport | Mark::Changed => Tone::Ink,
        Mark::Queued | Mark::Current => Tone::Quiet,
    }
}

/// Widths for the queue's count columns.
///
/// **The Chat column is the stretch section and the only one that says which row
/// is which**, so it must not be the first thing a narrow window throws away.
/// The counts are fixed and Chat takes the rest, with a floor under it.
///
/// **The floor is enforced rather than described.** An earlier pass computed the
/// Chat width itself, as `available - 68 - 58 - 38 - 62 - 16 - 4*8`, written out
/// twice — once for the header and once per row — under a comment asking the
/// next reader to re-check the sum by hand after any edit.
/// `Column::remainder().at_least(..)` is the same statement made to the layout
/// instead of to the reader, and it cannot fall out of step with the header
/// above it.
///
/// The counts are wider than they were, because there is now room: this table
/// had a whole window's width taken off it by a side panel, and `MESSAGES` at 58
/// points cannot hold `6,643` under its own header.
const STATUS_W: f32 = 96.0;
const COUNT_W: f32 = 88.0;
const TOPICS_W: f32 = 64.0;
const MEDIA_W: f32 = 88.0;
/// The floor under the Chat column.
///
/// **240, not 72.** Seventy-two was the width at which a name is still
/// technically present in a 400pt side column; at a window's width the column is
/// six hundred points and the floor only matters on a very narrow window, where
/// what it should protect is a title long enough to recognise rather than the
/// first two characters of one.
const CHAT_MIN_W: f32 = 240.0;
/// The × column. One control wide, and the table's own gap either side of it.
const REMOVE_W: f32 = tgx_ui::tokens::window::READING;
const COLUMN_GAP: f32 = space::TIGHT;

/// One queue row, and the header above it.
///
/// **26, not 20.** A row was 20 points tall carrying 13-point text, which is
/// tighter than the text's own leading — the type filled the row edge to edge
/// with nothing around it, and the row was also the click target that opens the
/// folder an export wrote. `READING` at the design's body leading is 21, so 26
/// gives it the same breathing room every other row in the window has.
/// Lines kept before each warning by Copy warnings, beside the chat's own
/// heading line. Three is the settings, the count and the step that went wrong
/// on a typical chat, without turning the copy back into the whole log.
const WARNING_CONTEXT: usize = 3;
const QUEUE_ROW_H: f32 = 26.0;
const HEADER_H: f32 = 20.0;

impl Shell {
    /// **Bar and log off the bottom, queue in what is left — as panels.**
    ///
    /// The first pass did this arithmetically: `46.0` reserved for the bar, 42%
    /// of the height for the log, and the queue given the remainder with a
    /// `.max(60.0)` in case the sums went negative. Three numbers to keep in
    /// step with three panels' actual contents. The bar measures itself now, and
    /// the log's share is a *default* the user can drag, which is the right
    /// answer to a split whose better position depends on whether an export is
    /// running.
    pub(super) fn queue_view(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let bare = egui::Frame::NONE.fill(p.bg);

        // **The log gets a third of the window rather than 160 points.** It is
        // the only account of what a run actually did — every rate-limit wait,
        // every file that did not arrive, the read total against what Telegram
        // promised — and in a side column it showed six lines of a two-thousand
        // line ring.
        egui::TopBottomPanel::bottom("log")
            .frame(bare)
            .default_height(ui.available_height() * 0.38)
            .height_range(120.0..=640.0)
            .resizable(true)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                rule(ui, &p);
                self.log_panel(ui);
            });

        egui::TopBottomPanel::bottom("progress")
            .frame(bare)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                rule(ui, &p);
                self.progress_row(ui);
            });

        egui::CentralPanel::default()
            .frame(bare)
            .show_inside(ui, |ui| self.queue_panel(ui));
    }

    fn queue_panel(&mut self, ui: &mut Ui) {
        let p = self.palette;
        self.report_runs(ui);
        ui.add_space(space::STEP);
        row(ui, |ui| {
            ui.label(title("Exports", &p));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = space::STEP;
                // **Emptying the queue is the user's decision**, and this is
                // the one place it is made — a Stop leaves the rows paused, and
                // a run that ends leaves them as its report.
                if !self.queue.is_empty() && !self.exporting {
                    if action(ui, text("Clear", &p), true).clicked() {
                        self.clear_queue();
                    }
                    if self.queue.is_paused()
                        && action(ui, text("Resume", &p), self.signed_in).clicked()
                    {
                        let picked = self.ticked_chats();
                        self.resume_queue(picked);
                    }
                }
                if !self.queue.is_empty() {
                    ui.label(figure(format!("{} chats", self.queue.len()), &p));
                }
            });
        });
        ui.add_space(space::TIGHT);
        rule(ui, &p);

        if self.queue.is_empty() {
            EmptyState::new(
                "Nothing queued",
                Some("Tick the chats you want under Chats, then press Export.".into()),
            )
            // A short panel drops the hint and keeps the headline.
            .show(ui, &p, false);
            return;
        }

        // Cloned once, because a click below needs `&mut self` and the borrow
        // cannot be held across it.
        let jobs: Vec<crate::queue::Job> = self.queue.jobs().to_vec();
        let mut opened = None;
        let mut removed = None;
        ui.add_space(space::TIGHT);
        block(ui, |ui| {
            // The table takes its column gap from the ui's own spacing.
            ui.spacing_mut().item_spacing.x = COLUMN_GAP;
            TableBuilder::new(ui)
                .id_salt("queue-rows")
                .vscroll(true)
                .auto_shrink([false, false])
                // A finished row opens what it wrote. The export's own folder is
                // the thing the user came for, and the alternative is retyping a
                // path they watched scroll past in the log.
                .sense(Sense::click())
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::remainder().at_least(CHAT_MIN_W).clip(true))
                .column(Column::exact(STATUS_W).clip(true))
                .column(Column::exact(COUNT_W))
                .column(Column::exact(TOPICS_W))
                .column(Column::exact(MEDIA_W))
                .column(Column::exact(REMOVE_W))
                .header(HEADER_H, |mut header| {
                    // Column headers are one word each, so they keep the
                    // section label's capitals: a column of counts needs its
                    // label to read as a label. The last one is blank: a
                    // heading over a column of × would name the control.
                    for label in ["Chat", "Status", "Messages", "Topics", "Media", ""] {
                        header.col(|ui| {
                            ui.label(eyebrow(label, &p));
                        });
                    }
                })
                .body(|body| {
                    body.rows(QUEUE_ROW_H, jobs.len(), |mut r| {
                        let job = &jobs[r.index()];
                        let running = job.state == crate::queue::JobState::Exporting;
                        r.col(|ui| {
                            ui.add(egui::Label::new(text(&job.title, &p)).truncate());
                        });
                        // **The one place the accent appears in this table**,
                        // and it means the same thing it means on the nav bar:
                        // *this run*. The state of a row that is not running is
                        // metadata like the counts beside it.
                        //
                        // A failure is its own error, which can be long, so it
                        // is text in ink and not a badge — a badge is a word.
                        r.col(|ui| match &job.state {
                            JobState::Failed(why) => {
                                ui.add(egui::Label::new(text(why, &p)).truncate())
                                    .on_hover_text(why);
                            }
                            state => {
                                badge(ui, state.label(), job_tone(state), &p);
                            }
                        });
                        // Counts that tick upward while the run goes, so Geist's
                        // proportional figures would move the column sideways on
                        // every update — `1` is 384 units against `0`'s 663.
                        // `figure` is the mono, which is tabular by
                        // construction.
                        for n in [
                            thousands(job.messages as i64),
                            job.topics_text(),
                            job.media_text(),
                        ] {
                            r.col(|ui| {
                                ui.label(figure(n, &p));
                            });
                        }
                        // **Take this one out of the run.** Offered on every
                        // row but the one exporting — see `Queue::remove` for
                        // why that one is Stop's business.
                        //
                        // **It sits inside a click-sensing row and that is the
                        // point.** egui gives a click to the topmost widget
                        // that wants one, so the × takes it and the row does
                        // not open a folder underneath. This is the same
                        // mechanism that broke the chat list's tick box, used
                        // deliberately and in the direction that is wanted:
                        // there the box had to stay out of the row's way, here
                        // it has to get in it.
                        let live = !running;
                        r.col(|ui| {
                            if dismiss(ui, live, &p).clicked() {
                                removed = Some(job.chat_id);
                            }
                        });
                        if job.root.is_some() && r.response().clicked() {
                            opened = Some(job.chat_id);
                        }
                    });
                });
        });

        if let Some(chat_id) = opened {
            if let Some(root) = self.queue.root_of(chat_id).cloned() {
                if let Err(e) = crate::actions::open_folder(&root) {
                    self.apply(crate::bridge::Event::Warn(e));
                }
            }
        }
        if let Some(chat_id) = removed {
            self.remove_from_queue(chat_id);
        }
    }

    /// **The report runs, above the exports.** Activity is everything this
    /// window is doing, and a report run used to be visible only from the
    /// Library — so "Report after export" made reports nobody could watch
    /// being made without leaving the export they were watching.
    ///
    /// Only what this session touched: a row the Library has never been
    /// asked to report on is not activity. Absent entirely when there is none.
    fn report_runs(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let state = &self.library.state;
        let rows: Vec<(String, Mark, String)> = state
            .entries
            .iter()
            .filter(|e| e.outcome != Outcome::Idle)
            .map(|e| {
                let why = match &e.outcome {
                    Outcome::Failed(why) => why.clone(),
                    _ => String::new(),
                };
                (e.title.clone(), e.mark(), why)
            })
            .collect();
        if rows.is_empty() && !state.running {
            return;
        }
        let (fill, status, running) = (state.fill(), state.status.clone(), state.running);
        ui.add_space(space::STEP);
        row(ui, |ui| {
            ui.label(title("Reports", &p));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(figure(format!("{} chats", rows.len()), &p));
            });
        });
        ui.add_space(space::TIGHT);
        rule(ui, &p);
        ui.add_space(space::TIGHT);
        block(ui, |ui| {
            for (name, mark, why) in &rows {
                ui.horizontal(|ui| {
                    ui.set_min_height(QUEUE_ROW_H);
                    badge(ui, mark.label(), mark_tone(*mark), &p);
                    ui.add_space(space::TIGHT);
                    ui.add(egui::Label::new(text(name, &p)).truncate());
                    if !why.is_empty() {
                        ui.add(egui::Label::new(meta(why, &p)).truncate());
                    }
                });
            }
            if running {
                ui.add_space(space::TIGHT);
                progress_bar(ui, fill, &p);
                ui.add_space(space::TIGHT);
                ui.label(meta(&status, &p));
            } else if !status.is_empty() {
                ui.label(meta(&status, &p));
            }
        });
        ui.add_space(space::STEP);
        rule(ui, &p);
    }

    /// Drop a chat from the run — **from all three, in this order**.
    ///
    /// `pending` is what the worker pops from and `queue` is what the table
    /// paints, and a removal that reached only one of them is worse than none:
    /// take it out of the table alone and the export writes the chat anyway
    /// with no row reporting it, take it out of the work list alone and a row
    /// sits at Queued for a chat that is never coming. `pending` first, so the
    /// worker cannot pick it up in the window between the two.
    ///
    /// **And the third is the tick in the chat list**, which is the one this
    /// shipped without. The tick is what `start_export` reads, so a × that took
    /// the row out and left the tick set meant the chat came straight back on
    /// the next Start — the removal undone by the button most likely to be
    /// pressed after it. `selected` is not a separate wish from the queue; it
    /// is the same one held where the chats are, and × says *not this chat*
    /// whichever panel it is said in.
    ///
    /// Removing the chat that is exporting is refused by `Queue::remove`, and
    /// the × is not drawn on that row either — two statements of one rule,
    /// because the second is a painting decision and the first has to hold
    /// whatever the panel does.
    pub(super) fn remove_from_queue(&mut self, chat_id: i64) {
        let title = self.queue.title_of(chat_id).unwrap_or_default().to_string();
        self.pending.remove(chat_id);
        if !self.queue.remove(chat_id) {
            // Refused, so the chat is in flight — and it was not in `pending`
            // either, because the worker popped it before it began. Nothing to
            // undo, and the tick stays: the chat is being exported, so saying
            // it was deselected would be the interface disagreeing with what
            // the run is doing.
            return;
        }
        self.selected.remove(&chat_id);
        self.journal
            .push(format!("{title}: taken out of the queue"));
        self.log_copied = false;
    }

    /// **One bar, two claimants.** The export claims it; while it is claimed
    /// every counting handler keeps its hands off, or a count finishing
    /// mid-export paints "Counted 12 of 12 chats" over "6,000 of 6,643".
    fn progress_row(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let (fraction, caption) = if self.exporting {
            (self.queue.fraction(), self.queue.running_caption())
        } else if let Some((done, total)) = self.count_progress {
            (
                Some(done as f32 / total.max(1) as f32),
                format!("Counting {done} of {total} chats"),
            )
        } else if !self.queue.is_empty() {
            // A finished run keeps its bar and its summary. Resetting to Idle
            // the moment the last chat lands throws away the one place that
            // says how the run ended, on the frame the user looks at it.
            (self.queue.fraction(), self.queue.summary())
        } else {
            // Not indeterminate: nothing is running, and a marker sitting at
            // the left of an idle bar reads as a job that has stalled.
            (Some(0.0), "Idle".to_string())
        };
        // A block, not a row inside an `allocate_ui` sized by hand: the bar and
        // its caption are stacked, and inside the gutter the available width
        // already is the bar's width.
        ui.add_space(space::STEP);
        block(ui, |ui| {
            progress_bar(ui, fraction, &p);
            ui.add_space(space::TIGHT);
            ui.label(meta(&caption, &p));
        });
        ui.add_space(space::STEP);
    }

    fn log_panel(&mut self, ui: &mut Ui) {
        let p = self.palette;
        ui.add_space(space::STEP);
        row(ui, |ui| {
            ui.label(title("Log", &p));
            // Two things the header has to say. A warning is marked so it
            // cannot be lost among the ordinary lines, and a thousand-line
            // transcript with one red line in it is exactly where it gets lost
            // — so the count is up here. And what the cap has dropped is stated
            // rather than silently presenting a truncated run as the whole run.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = space::STEP;
                // **The one way out of this panel.** An export's own account of
                // what it did used to stop at the screen: it could be read,
                // photographed, and handed on no other way. egui can select the
                // lines themselves now, but a transcript is wanted all at once.
                if !self.journal.is_empty() {
                    let copied = self.log_copied && !self.copied_warnings;
                    // Ink while it is an offer, muted once it has been taken —
                    // the same weight as the metadata beside it. Enabled either
                    // way: "Copied" is a report, not a control that has been
                    // used up, and the transcript can be taken again once it has
                    // grown.
                    let label = if copied { "Copied" } else { "Copy all" };
                    let ink = if copied { p.muted } else { p.fg };
                    if action(ui, text(label, &p).color(ink), true).clicked() {
                        ui.ctx().copy_text(self.journal.to_text());
                        self.log_copied = true;
                        self.copied_warnings = false;
                    }
                }
                // **Just the warnings, each with the lines that explain it.**
                // A twenty-chat run is a thousand lines, and the five that
                // matter were being handed on buried in the other 995 — or
                // copied one at a time, which loses which chat each was about.
                if self.journal.warnings() > 0 {
                    let copied = self.log_copied && self.copied_warnings;
                    let label = if copied { "Copied" } else { "Copy warnings" };
                    let ink = if copied { p.muted } else { p.fg };
                    if action(ui, text(label, &p).color(ink), true).clicked() {
                        ui.ctx()
                            .copy_text(self.journal.warnings_text(WARNING_CONTEXT));
                        self.log_copied = true;
                        self.copied_warnings = true;
                    }
                }
                if self.journal.dropped() > 0 {
                    ui.label(meta(
                        format!("{} earlier lines dropped", self.journal.dropped()),
                        &p,
                    ));
                }
                // **The accent, and it is still "this run".** A warning is
                // something the run produced and has not been read yet, which is
                // the same category as the progress bar beside it.
                if self.journal.warnings() > 0 {
                    let n = self.journal.warnings();
                    let word = if n == 1 { "warning" } else { "warnings" };
                    badge(ui, &format!("{n} {word}"), Tone::Accent, &p);
                }
            });
        });
        ui.add_space(space::TIGHT);

        if self.journal.is_empty() {
            EmptyState::new("Nothing logged yet", None).show(ui, &p, false);
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("log-lines")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add_space(4.0);
                // One gutter around the whole transcript rather than one
                // `add_space` per line: the lines wrap, and a space allocated
                // inside a wrapping row is a space the second line does not get,
                // so long lines used to start flush against the panel edge.
                block(ui, |ui| {
                    for line in self.journal.lines() {
                        // **A warning is ink, not the accent.** In a transcript
                        // set entirely in `muted`, the text colour already *is*
                        // the loud one — and the accent is spoken for: it means
                        // *this run*, on the Start button, the progress fill and
                        // the row that is exporting. Spending it on a fourth
                        // thing is how it stopped meaning anything. Marked as a
                        // warning by whoever wrote the line, never sniffed out
                        // of its text.
                        let ink = if line.warning { p.fg } else { p.muted };
                        ui.label(
                            egui::RichText::new(&line.text)
                                .font(tgx_ui::fonts::sans(window::LABEL))
                                .color(ink),
                        );
                    }
                });
                ui.add_space(space::TIGHT);
            });
    }
}
