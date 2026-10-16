//! The chat list: the filter, the sort, the rows and the selection controls.
//!
//! **The rows are a cache**, rebuilt by `Shell::rebuild_rows` when something
//! changes what they would be. `show_rows` is virtualised, so it asks only for
//! the range on screen — re-filtering and re-sorting the whole account inside
//! that callback would be work done sixty times a second for an answer that
//! changed once.

use super::*;
use crate::list::{Presence, SortMode};
use chrono::{DateTime, Local};
use eframe::egui::{Align, Layout, Sense, Ui};
use tgx_ui::components::{
    action, badge, button, caps, count_text, disclosure, field, figure, forum_dot, meta, row, rule,
    selection_label, text, tick_box, tick_mark, Tone, GUTTER,
};
use tgx_ui::tokens::space;

/// Every row is this tall, heading and chat alike.
///
/// The list is laid out from one row height, which is what makes several
/// thousand chats cost the same as nine. The price is that a category heading
/// cannot be shorter than a chat row; it is set in tracked micro-type instead,
/// which distinguishes it by weight rather than by size.
///
/// The Analysis view's rows are this tall too, and read it from here.
pub(super) const ROW_HEIGHT: f32 = 46.0;

/// How wide the filter box is.
///
/// **Fixed, not the panel's width.** Stretched across a window it advertises
/// room for a sentence, and what goes in it is a word — the field was 1,170
/// points long to hold `kolab`.
///
/// 260 rather than the 380 it was: the row gained the Show menu, and at the
/// window's 900pt floor four controls at the old width ran off the edge.
const FILTER_W: f32 = 260.0;

impl Shell {
    /// **Head, foot, then the list — as panels, not as measured heights.**
    ///
    /// The first pass reserved `62.0` off the bottom by hand before laying the
    /// list out, because that is what the flex column it came from did
    /// implicitly. egui already has the mechanism: a `TopBottomPanel` measures
    /// itself against its own contents and the central area gets exactly what is
    /// left. So the magic number goes, and the footer is correct at any height
    /// and any type size rather than at the one it was measured on.
    pub(super) fn chat_panel(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let bare = egui::Frame::NONE.fill(p.bg);

        egui::TopBottomPanel::top("chats-head")
            .frame(bare)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                // **No `CHATS` heading here.** The view bar above already says
                // which of the three this is, and repeating it put the same word
                // twice within fifty points, with a rule under each. A view does
                // not need to introduce itself.
                ui.add_space(space::STEP);
                self.filter_row(ui);
                ui.add_space(space::STEP);
                rule(ui, &p);
            });

        egui::TopBottomPanel::bottom("chats-foot")
            .frame(bare)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                let live = self.selection_actions_enabled();
                let (total, any_uncounted) = self.selection_total();
                // The foot keeps its rule: below it is a summary of the list and
                // what to do with it, which is a different kind of thing from
                // the list itself.
                rule(ui, &p);
                ui.add_space(space::STEP);
                self.selection_row(ui, live);
                ui.add_space(space::TIGHT);
                row(ui, |ui| {
                    ui.label(figure(
                        selection_label(self.selected.len(), total, any_uncounted),
                        &p,
                    ));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.saved_selections(ui);
                    });
                });
                ui.add_space(space::STEP);
            });

        egui::CentralPanel::default()
            .frame(bare)
            .show_inside(ui, |ui| self.list_body(ui));
    }

    /// Filter, sort and grouping — **one row, gathered at the left**.
    ///
    /// These were two rows, and the second pushed GROUP BY TYPE to the far edge
    /// with `right_to_left`. That is the correct shape for a status bar, where
    /// one end is fixed and the other gives; it is the wrong one for three
    /// controls that belong together, and at a window's width it left them
    /// nine hundred points apart with nothing in between.
    ///
    /// The field takes a fixed width rather than the panel's: a filter box
    /// stretched across a wide window advertises room for a sentence, and what
    /// goes in it is a word.
    ///
    /// The sort is a menu rather than seven chips: seven labels do not fit, and
    /// cycling through seven with one click means six clicks to undo a mistake.
    /// **It is a `ComboBox`, and that removed a field from the shell.** GPUI
    /// needed a whole catcher apparatus to dismiss a menu — an absolutely
    /// positioned 16000px hitbox with `occlude`, deferred below the menu — so
    /// the swap carried over a hand-driven `Popup` and a `sort_open: bool` to go
    /// with it. Openness is the widget's business, not the window's.
    fn filter_row(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let before = self.view.sort;
        let mut chosen = before;

        row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            let typed_before = self.search.clone();
            let filter = ui.add(
                field(&mut self.search, &p)
                    .hint_text("Filter chats…  Ctrl+F")
                    .desired_width(FILTER_W),
            );
            if std::mem::take(&mut self.focus_filter) {
                filter.request_focus();
            }
            // Read from the field, never typed into a mirror of it: a second
            // copy of the same string is how the empty state ends up quoting
            // something other than what is in the box.
            if self.search != typed_before {
                let typed = self.search.clone();
                self.set_filter(typed);
            }

            ui.label(meta("Sort", &p));
            egui::ComboBox::from_id_salt("sort")
                .selected_text(text(before.label(), &p))
                .width(180.0)
                .show_ui(ui, |ui| {
                    for mode in SortMode::ALL {
                        // The chosen mode is not marked in the accent: the
                        // accent means *this run*, and a sort order is not one.
                        // `selectable_value` already fills the selected item.
                        ui.selectable_value(&mut chosen, mode, text(mode.label(), &p));
                    }
                });

            // **Which chats, by what the destination already holds.** A menu
            // rather than a third tick box: three answers to one question.
            let presence_before = self.view.presence;
            let mut presence = presence_before;
            egui::ComboBox::from_id_salt("presence")
                .selected_text(text(presence.label(), &p))
                .width(160.0)
                .show_ui(ui, |ui| {
                    for option in Presence::ALL {
                        ui.selectable_value(&mut presence, option, text(option.label(), &p));
                    }
                });
            if presence != presence_before {
                self.view.presence = presence;
                self.rebuild_rows();
            }

            // Box and label are one control, as in the settings panel. This was
            // the chat row's bug the other way up: the box worked and the words
            // beside it were a plain label, so clicking the thing you read did
            // nothing.
            let hit = tick_box(ui, self.view.grouped, true, &p).union(action(
                ui,
                text("Group by type", &p),
                true,
            ));
            if hit.clicked() {
                self.view.grouped = !self.view.grouped;
                self.rebuild_rows();
                self.commit_settings();
            }

            // **Reload and Count sit beside the list they act on.** Reload was
            // step 02 on the top bar, on every view, which made a list action
            // look like part of the export sequence; Count was in the foot
            // among the selection verbs, though it selects nothing.
            let busy = self.exporting || self.counting;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // The label states the action; the price is the tooltip — one
                // request per chat, minutes on a large account.
                // Off while Update databases reads the folder, too: its answer
                // starts an export. Stop counting is never off.
                let countable =
                    !self.chats.is_empty() && !self.exporting && (self.counting || !self.scanning);
                let count = if self.counting {
                    "Stop counting"
                } else {
                    "Count"
                };
                if button(ui, count, countable, false, &p)
                    .on_hover_text(
                        "Count the messages in every chat, so the list can sort by size. \
                         One request per chat.",
                    )
                    .clicked()
                {
                    self.start_count();
                }
                if button(ui, "Reload", self.signed_in && !busy, false, &p)
                    .on_hover_text("Reload chats from Telegram")
                    .clicked()
                {
                    self.start_refresh();
                }
            });
        });

        if chosen != before {
            self.view.sort = chosen;
            self.rebuild_rows();
            // Sorting by size with no counts fetched yet is not an error, but it
            // is a list that will not move — say so rather than leaving it
            // looking broken.
            if matches!(chosen, SortMode::Largest | SortMode::Smallest)
                && self.chats.iter().all(|c| c.message_count.is_none())
            {
                self.status = "No message counts yet — press Count".into();
            }
            self.commit_settings();
        }
    }

    fn list_body(&mut self, ui: &mut Ui) {
        let p = self.palette;
        if let Some(empty) = self.list_state().empty_state(&self.view.filter) {
            empty.show(ui, &p, true);
            return;
        }

        // **The clock is read once a frame, here, and carried down.** Every
        // caption says how long ago the chat last moved, and reading the clock
        // inside the row would be one syscall per visible row per frame — for
        // an answer that cannot change between two rows of the same list.
        let now = Local::now();
        let count = self.rows.len();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, count, |ui, range| {
                for i in range {
                    // Indexed rather than iterated: the row is read out of the
                    // cache and the body below needs `&mut self` to act on a
                    // click, so the borrow cannot be held across it.
                    let Some(row) = self.rows.get(i) else {
                        continue;
                    };
                    match row {
                        PaintedRow::Heading {
                            category,
                            total,
                            folded,
                        } => {
                            let (category, total, folded) = (*category, *total, *folded);
                            self.heading_row(ui, category, total, folded);
                        }
                        PaintedRow::Chat(chat) => {
                            let chat = chat.clone();
                            self.chat_row(ui, &chat, now);
                        }
                    }
                }
            });
    }

    /// A category heading.
    ///
    /// Folding is a way of looking at the list, not a second filter: a folded
    /// category's chats still count as visible, so tidying the view cannot
    /// silently change what All selects. See `crate::list`.
    fn heading_row(&mut self, ui: &mut Ui, category: Category, total: usize, folded: bool) {
        let p = self.palette;
        let (rect, response) = cell(ui, Sense::click());
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::ZERO, p.surface);
        inside(ui, rect, GUTTER, |ui| {
            disclosure(ui, folded, &p);
            ui.label(caps(category.label(), p.fg));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(figure(total.to_string(), &p));
            });
        });

        // **The click inverts the chevron it was painted beside**, rather than
        // re-deriving the state from `view.folded`. A heading whose painted
        // state and stored state ever part company — a search re-opening a
        // category is where that happened — would otherwise take one click to
        // undo a fold nobody could see and a second to do what was asked.
        if response.clicked() {
            if folded {
                self.view.folded.remove(&category);
            } else {
                self.view.folded.insert(category);
            }
            self.rebuild_rows();
            self.commit_settings();
        }
    }

    /// One chat. `now` is passed in rather than read here — see [`Self::list_body`].
    fn chat_row(&mut self, ui: &mut Ui, chat: &tgx_tg::client::ChatInfo, now: DateTime<Local>) {
        let p = self.palette;
        let ticked = self.selected.contains(&chat.id);
        let (rect, response) = cell(ui, Sense::click());
        // **A ticked row is a filled row, and a hovered one too.** The whole row
        // is the hit target, so a 12px box in the corner was the only thing
        // saying which rows were chosen — over four hundred of them, on the one
        // decision that governs what the export does.
        if ticked || response.hovered() {
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::ZERO, p.surface);
        }
        let indent = if self.view.grouped { 30.0 } else { GUTTER };
        inside(ui, rect, indent, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // **Painted, not clickable.** The row is the control; a box that
            // sensed clicks would sit on top of it and take them, and the 12px
            // square that looks most like the switch would be the one place
            // clicking did nothing.
            tick_mark(ui, ticked, &p);
            ui.add_space(12.0);
            // A forum is marked by a painted dot, never by a suffix on the
            // stored title — presentation in the string is what the filter then
            // searches.
            if chat.is_forum {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(6.0, 6.0), Sense::hover());
                ui.painter()
                    .rect_filled(dot, egui::CornerRadius::ZERO, forum_dot(&p));
                ui.add_space(8.0);
            }

            // The count is the row's one number. Laid out from the right so a
            // long title cannot push it off the panel.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(figure(count_text(chat.message_count), &p));
                ui.add_space(space::STEP);
                // **Already in the destination**, so a list of six hundred
                // says which of them a run would be updating rather than
                // starting. Muted, beside the count: it is a fact about the
                // folder, not a state of the row, and the fill already means
                // ticked.
                let disk = self.view.disk.get(&chat.id).copied().unwrap_or_default();
                if let Some(mark) = on_disk_mark(disk.database, disk.folder) {
                    badge(ui, mark, Tone::Quiet, &p);
                    ui.add_space(space::STEP);
                }
                // **A converted group is marked in ink**, on both halves: the
                // pair usually share a title and hold different messages, and
                // exporting one of them looks complete. Not the accent, which
                // is this run's.
                if let Some(m) = &chat.migration {
                    badge(ui, list::migration_mark(m), Tone::Ink, &p);
                    ui.add_space(space::STEP);
                }
                // Title over caption, and the two are one block: no space
                // between them beyond the leading, because a gap there would
                // read as two rows rather than one chat with a subtitle.
                ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(text(&chat.title, &p)).truncate());
                    // The caption is the kind and the last activity. **The date
                    // is half its point**: the default sort is Recent activity,
                    // and without it the list is ordered on a unix second that
                    // appears nowhere on screen.
                    //
                    // **What the destination holds goes on the same line**:
                    // how big it is and when the database last caught up.
                    // Under the title rather than beside the count, because
                    // beside it the row would push the title off a 900pt
                    // window to say something that is not about the chat.
                    let mut line = list::caption(chat, now);
                    if disk.bytes > 0 {
                        line.push_str(&format!("    {}", list::human_bytes(disk.bytes)));
                    }
                    if let Some(at) = disk.synced {
                        line.push_str(&format!("  synced {}", list::human_when(at, now)));
                    }
                    ui.add(egui::Label::new(meta(line, &p)).truncate());
                });
            });
        });

        // Clicking anywhere on a row ticks it. Ticks are held by chat id, so
        // they survive re-sorting, regrouping and filtering.
        if response.clicked() && !self.selected.remove(&chat.id) {
            self.selected.insert(chat.id);
        }
        // **Right-click: this chat alone**, without touching the ticks — a
        // selection built over six hundred rows is not the price of
        // exporting one more.
        let can_export = self.signed_in && !self.counting;
        let mut export_one = false;
        response.context_menu(|ui| {
            let verb = if self.exporting {
                "Add this chat to the queue"
            } else {
                "Export this chat"
            };
            if ui
                .add_enabled(can_export, egui::Button::new(verb))
                .clicked()
            {
                export_one = true;
                ui.close();
            }
            let label = if ticked { "Untick" } else { "Tick" };
            if ui.button(label).clicked() {
                if !self.selected.remove(&chat.id) {
                    self.selected.insert(chat.id);
                }
                ui.close();
            }
            if ui.button("Copy title").clicked() {
                ui.ctx().copy_text(chat.title.clone());
                ui.close();
            }
        });
        if export_one {
            self.export_these(vec![chat.clone()]);
        }
    }

    /// **Saved selections: a name, Save, Recall, Forget.** Laid out right to
    /// left by the caller, so the first control added is the furthest right.
    ///
    /// One name field serves all three: Save stores the ticks under it
    /// (replacing a set of the same name), Recall fills it with the set it
    /// recalled, and Forget removes the set it names. A separate field per
    /// action would be three places to type the same word.
    fn saved_selections(&mut self, ui: &mut Ui) {
        let p = self.palette;
        ui.spacing_mut().item_spacing.x = space::TIGHT;
        let name = self.selection_name.trim().to_string();
        let known = self
            .settings
            .saved_selections
            .iter()
            .any(|s| s.name == name);

        if action(ui, text("Forget", &p), known).clicked() {
            self.forget_selection(&name);
        }
        let mut recalled = None;
        let names: Vec<String> = self
            .settings
            .saved_selections
            .iter()
            .map(|s| s.name.clone())
            .collect();
        ui.add_enabled_ui(!names.is_empty(), |ui| {
            egui::ComboBox::from_id_salt("saved-selections")
                .selected_text(text("Recall", &p))
                .width(140.0)
                .show_ui(ui, |ui| {
                    for n in &names {
                        if ui.selectable_label(false, text(n, &p)).clicked() {
                            recalled = Some(n.clone());
                        }
                    }
                });
        });
        if let Some(n) = recalled {
            self.recall_selection(&n);
        }
        let can_save = !name.is_empty() && !self.selected.is_empty();
        if action(ui, text("Save", &p), can_save).clicked() {
            self.save_selection(&name);
        }
        ui.add(
            field(&mut self.selection_name, &p)
                .hint_text("Selection name")
                .desired_width(150.0),
        );
    }

    /// Store the ticks under `name`, replacing a set already called that.
    pub(super) fn save_selection(&mut self, name: &str) {
        let mut chats: Vec<i64> = self.selected.iter().copied().collect();
        chats.sort_unstable();
        let n = chats.len();
        let saved = &mut self.settings.saved_selections;
        match saved.iter_mut().find(|s| s.name == name) {
            Some(existing) => existing.chats = chats,
            None => saved.push(tgx_tg::config::SavedSelection {
                name: name.to_string(),
                chats,
            }),
        }
        self.commit_settings();
        self.status = format!("Saved {n} chats as \"{name}\"");
    }

    /// Replace the ticks with a saved set. **Replaces, like Only forums**:
    /// a recalled set added to whatever was ticked is a set nobody saved.
    pub(super) fn recall_selection(&mut self, name: &str) {
        let Some(set) = self
            .settings
            .saved_selections
            .iter()
            .find(|s| s.name == name)
        else {
            return;
        };
        let live: std::collections::HashSet<i64> = self.chats.iter().map(|c| c.id).collect();
        let ids: Vec<i64> = set.chats.clone();
        self.selected = ids.iter().copied().filter(|id| live.contains(id)).collect();
        let gone = ids.len() - self.selected.len();
        self.selection_name = name.to_string();
        // An id the account no longer has is skipped and *said*: a set of 12
        // recalled as 11 ticks is otherwise a question with no answer.
        self.status = if gone == 0 {
            format!("Recalled \"{name}\": {} chats", self.selected.len())
        } else {
            format!(
                "Recalled \"{name}\": {} chats, {gone} no longer in the list",
                self.selected.len()
            )
        };
    }

    pub(super) fn forget_selection(&mut self, name: &str) {
        self.settings.saved_selections.retain(|s| s.name != name);
        self.commit_settings();
        self.status = format!("Forgot \"{name}\"");
    }

    /// All / None / Invert / Only forums, and the Count button.
    ///
    /// Nothing offers to do what it cannot: these act on the *visible* rows, so
    /// over an empty list every one is a no-op. A button that is enabled and
    /// does nothing teaches that the interface is unreliable.
    fn selection_row(&mut self, ui: &mut Ui, live: bool) {
        let p = self.palette;
        row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = space::STEP;
            for (i, label) in ["All", "None", "Invert", "Only forums"].iter().enumerate() {
                // **Flat, unlike the nav bar's.** Four boxes here would put more
                // border under the list than there is around the buttons that
                // matter, and these four only refine a selection the row above
                // already shows. The ink is unconditional: `action` hands the
                // disabled state to `add_enabled`, which greys the label and
                // refuses the click in the same call — the first pass chose a
                // colour here and a `Sense` beside it, and the two could
                // disagree.
                if action(ui, text(*label, &p), live).clicked() {
                    // Asked once: `visible()` sorts the whole account, and
                    // every one of these four needs the same answer.
                    let visible: Vec<(i64, bool)> =
                        self.visible().iter().map(|c| (c.id, c.is_forum)).collect();
                    match i {
                        0 => self.selected.extend(visible.iter().map(|(id, _)| *id)),
                        1 => {
                            for (id, _) in &visible {
                                self.selected.remove(id);
                            }
                        }
                        2 => {
                            for (id, _) in &visible {
                                if !self.selected.remove(id) {
                                    self.selected.insert(*id);
                                }
                            }
                        }
                        // **"Only forums" replaces, it does not add.** The
                        // label promises an exclusive selection, and adding to
                        // one meant clicking All and then this left every chat
                        // ticked with the footer still counting the lot.
                        // Assigning the set outright would do it, but the
                        // replacement is scoped to the visible rows like the
                        // other three, so a tick on a chat the filter is hiding
                        // is left where it is rather than silently dropped.
                        3 => {
                            for (id, is_forum) in &visible {
                                if *is_forum {
                                    self.selected.insert(*id);
                                } else {
                                    self.selected.remove(id);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            // **Update databases sits here, not on the top bar.** It is a way
            // of choosing chats — the ones the destination already holds — and
            // it needs this list, because a database records a chat's id and
            // the list is where the rest of what a run needs comes from. Off
            // during a run: that run has its format already, and this one may
            // have to change it. (Count went up to the filter row: it changes
            // what the list shows, not which chats are chosen.)
            let updatable = self.signed_in
                && self.loaded
                && !self.exporting
                && !self.counting
                && !self.scanning;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, "Update databases", updatable, false, &p)
                    .on_hover_text("Sync every chat that already has a database here")
                    .clicked()
                {
                    self.start_update();
                }
            });
        });
    }
}

/// What a row says about the destination: a database, a folder, or both.
pub(super) fn on_disk_mark(database: bool, folder: bool) -> Option<&'static str> {
    match (database, folder) {
        (true, true) => Some("Database + Folder"),
        (true, false) => Some("Database"),
        (false, true) => Some("Folder"),
        (false, false) => None,
    }
}

/// One list row: full width, exactly [`ROW_HEIGHT`], allocated and sensed.
///
/// The height is fixed because `ScrollArea::show_rows` is told the same figure
/// and skips straight to the visible range on it — a row that measured itself
/// would put the list's scroll position and its contents out of step.
fn cell(ui: &mut Ui, sense: Sense) -> (egui::Rect, egui::Response) {
    ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), sense)
}

/// Lay a row's contents out inside the rect it was given, **vertically centred**.
///
/// `left` is the row's own indent; the right gutter is the panel's. The first
/// pass reached for `Ui::horizontal_centered` here, which allocates a strip one
/// `interact_size.y` tall — 30 points against this row's 46 — and centres within
/// *that*, leaving every row sitting eight points high in its own box. A layout
/// given the row rect centres in the row.
fn inside<R>(ui: &mut Ui, rect: egui::Rect, left: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + left, rect.top()),
        egui::pos2(rect.right() - GUTTER, rect.bottom()),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            // A label is selectable text by default, and selectable text takes
            // the click: the row's title would be the one place it missed.
            ui.style_mut().interaction.selectable_labels = false;
            add(ui)
        },
    )
    .inner
}
