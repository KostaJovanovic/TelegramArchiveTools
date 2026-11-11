//! People: one cell per account, most messages first, with a bar for how
//! much each wrote; and Organisations, the same for the organisations a tag
//! in people's names names.
//!
//! Two views fill it, from one saved list per folder (`people.json`, see
//! `tga_run::people`). Analysis reads the exports it lists; Chats asks
//! Telegram about the ticked chats, members only or every message. Either way
//! a person in several chats is one cell: matched by Telegram id, never by
//! name. This file only draws; its owner starts the counting.

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Align, Layout, Sense, Ui};
use tga_run::people::{ChatRow, List, ORGS};
use tgx_ui::components::{
    action, badge, button, field, figure, meta, rule, segmented, text, EmptyState, Tone, GUTTER,
};
use tgx_ui::tokens::{space, Palette};

use super::chats::ROW_HEIGHT;

/// A cell is never narrower than this; the grid takes as many as fit.
const CELL_W: f32 = 280.0;
const CELL_H: f32 = 44.0;
const BAR_H: f32 = 3.0;

const TABS: [&str; 3] = ["People", "Organisations", "Chats"];
const PEOPLE: usize = 0;
const ORGANISATIONS: usize = 1;

#[derive(Default)]
pub struct People {
    /// The screen is showing in place of its view's list.
    pub open: bool,
    filter: String,
    list: List,
    /// Which of [`TABS`].
    tab: usize,
    /// A chat whose Remove was pressed once: the second press removes it.
    confirm: Option<String>,
    /// What the counting is doing now; `None` when idle.
    pub working: Option<String>,
    /// A list has arrived since the screen was last cleared.
    pub counted: bool,
}

/// What the owner is asked to do.
pub enum Asked {
    /// One of the head's buttons, by label.
    Action(&'static str),
    /// Take this chat out of the saved list (its key in `people.json`).
    Remove(String),
}

/// Who a chat's gone is decided by. `telegram` is the chat list as loaded,
/// when there is one: a chat counted from Telegram that is no longer in it
/// was left or deleted.
pub struct Known<'a> {
    pub telegram: Option<&'a HashSet<i64>>,
}

fn gone(chat: &ChatRow, known: &Known) -> bool {
    chat.file_gone
        || (!chat.from_export
            && known
                .telegram
                .zip(chat.chat_id())
                .is_some_and(|(ids, id)| !ids.contains(&id)))
}

/// One cell, whichever tab.
struct Cell<'a> {
    id: &'a str,
    name: &'a str,
    figure: String,
    measure: usize,
    hover: String,
}

impl People {
    pub fn set(&mut self, list: List) {
        self.list = list;
        self.working = None;
        self.counted = true;
    }

    /// The screen. `actions` are the buttons at the far right: label, whether
    /// it can be pressed, its tooltip. `known` says which chats are gone.
    /// Removing is offered only while no count runs.
    pub fn panel(
        &mut self,
        ui: &mut Ui,
        actions: &[(&'static str, bool, &str)],
        known: &Known,
        p: &Palette,
    ) -> Option<Asked> {
        let bare = egui::Frame::NONE.fill(p.bg);
        let mut pressed = None;
        egui::TopBottomPanel::top("people-head")
            .frame(bare)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.add_space(space::STEP);
                tgx_ui::components::row(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = space::STEP;
                    if action(ui, text("\u{2190} Back", p), true).clicked() {
                        self.open = false;
                    }
                    ui.label(text("People", p).strong());
                    let count = match &self.working {
                        Some(doing) => doing.clone(),
                        None => format!(
                            "{} in {}",
                            plural(self.list.people.len(), "person", "people"),
                            plural(self.list.chats.len(), "chat", "chats"),
                        ),
                    };
                    ui.label(figure(count, p));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        for (label, enabled, hover) in actions.iter().rev() {
                            if button(ui, label, *enabled, false, p)
                                .on_hover_text(*hover)
                                .clicked()
                            {
                                pressed = Some(Asked::Action(label));
                            }
                        }
                        ui.add(
                            field(&mut self.filter, p)
                                .hint_text("Filter")
                                .desired_width(200.0),
                        );
                        if let Some(tab) = segmented(ui, &TABS, self.tab, p) {
                            self.tab = tab;
                        }
                    });
                });
                if !self.list.notes.is_empty() {
                    tgx_ui::components::row(ui, |ui| {
                        ui.add(egui::Label::new(meta(self.list.notes.join("; "), p)).wrap())
                    });
                }
                ui.add_space(space::STEP);
                rule(ui, p);
            });
        egui::CentralPanel::default()
            .frame(bare)
            .show_inside(ui, |ui| match self.tab {
                PEOPLE => self.people_grid(ui, p),
                ORGANISATIONS => self.org_grid(ui, p),
                _ => {
                    let idle = self.working.is_none();
                    if let Some(id) = self.chat_list(ui, known, idle, p) {
                        pressed = Some(Asked::Remove(id));
                    }
                }
            });
        pressed
    }

    /// Every saved chat: what it holds, how and when it was counted, whether
    /// it is gone, and Remove — pressed twice, because a count of every
    /// message can have taken minutes.
    fn chat_list(&mut self, ui: &mut Ui, known: &Known, idle: bool, p: &Palette) -> Option<String> {
        let needle = self.filter.trim().to_lowercase();
        let rows: Vec<ChatRow> = self
            .list
            .chats
            .iter()
            .filter(|c| needle.is_empty() || c.title.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        if rows.is_empty() {
            let empty = if self.list.chats.is_empty() {
                "No chats counted here yet"
            } else {
                "Nothing matches the filter"
            };
            EmptyState::new(empty, None).show(ui, p, true);
            return None;
        }
        let now = SystemTime::now();
        let mut removed = None;
        egui::ScrollArea::vertical()
            .id_salt("people-chats")
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                for chat in &rows[range] {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        Sense::hover(),
                    );
                    if response.hovered() {
                        ui.painter()
                            .rect_filled(rect, egui::CornerRadius::ZERO, p.surface);
                    }
                    let inner = rect.shrink2(egui::vec2(GUTTER, 0.0));
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(inner)
                            .layout(Layout::right_to_left(Align::Center)),
                        |ui| {
                            ui.style_mut().interaction.selectable_labels = false;
                            ui.spacing_mut().item_spacing.x = space::STEP;
                            if self.confirm.as_deref() == Some(chat.id.as_str()) {
                                if action(ui, text("Cancel", p), true).clicked() {
                                    self.confirm = None;
                                }
                                if action(ui, text("Really remove", p), idle).clicked() {
                                    removed = Some(chat.id.clone());
                                    self.confirm = None;
                                }
                            } else if action(ui, meta("Remove", p), idle)
                                .on_hover_text(
                                    "Take this chat and its counts out of the saved list. \
                                     An export still in the folder is read again on the \
                                     next update.",
                                )
                                .clicked()
                            {
                                self.confirm = Some(chat.id.clone());
                            }
                            if gone(chat, known) {
                                badge(ui, "Gone", Tone::Ink, p).on_hover_text(if chat.file_gone {
                                    "The export it was read from is no longer on disk"
                                } else {
                                    "No longer in your Telegram chat list: left or deleted"
                                });
                            }
                            ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.add(egui::Label::new(text(&chat.title, p).strong()).truncate());
                                ui.add(egui::Label::new(meta(caption(chat, now), p)).truncate());
                            });
                        },
                    );
                }
            });
        removed
    }

    fn people_grid(&mut self, ui: &mut Ui, p: &Palette) {
        let needle = self.filter.trim().to_lowercase();
        let messages = self.list.messages;
        let cells: Vec<Cell> = self
            .list
            .people
            .iter()
            .filter(|person| {
                needle.is_empty()
                    || person.name.to_lowercase().contains(&needle)
                    || person.org.to_lowercase() == needle
            })
            .map(|person| {
                let msgs = plural(person.messages, "message", "messages");
                let chats = plural(person.chats, "chat", "chats");
                let org = if person.org.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", person.org)
                };
                Cell {
                    id: &person.key,
                    name: if person.name.is_empty() {
                        &person.key
                    } else {
                        &person.name
                    },
                    hover: if messages {
                        format!("{} · {msgs} in {chats}{org}", person.key)
                    } else {
                        format!("{} · in {chats}{org}", person.key)
                    },
                    figure: if messages { msgs } else { chats },
                    measure: if messages {
                        person.messages
                    } else {
                        person.chats
                    },
                }
            })
            .collect();
        // Out of the largest anywhere, not the largest shown, so a filter
        // does not stretch everybody's bar.
        let most = self
            .list
            .people
            .iter()
            .map(|x| if messages { x.messages } else { x.chats })
            .max();
        let empty = if self.working.is_some() {
            "Counting people…"
        } else if !self.counted {
            "Not counted yet"
        } else if self.list.people.is_empty() {
            "Nobody counted here yet"
        } else {
            "Nobody matches the filter"
        };
        grid(ui, &cells, most, empty, None, p);
    }

    fn org_grid(&mut self, ui: &mut Ui, p: &Palette) {
        if self.list.orgs.is_empty() {
            EmptyState::new(
                "No organisations",
                Some(format!(
                    "Put {ORGS} beside people.json, in the export folder: ReVision's own \
                     file works as it is (id, name, kind, parent, city, country, short, \
                     other_names; \"|\" between names). Somebody belongs to one when a \
                     tag in their Telegram name names it, as in \"Ana ETF\". Press Update \
                     list after adding it."
                )),
            )
            .show(ui, p, true);
            return;
        }
        let needle = self.filter.trim().to_lowercase();
        let cells: Vec<Cell> = self
            .list
            .orgs
            .iter()
            .filter(|o| {
                needle.is_empty()
                    || o.label.to_lowercase().contains(&needle)
                    || o.name.to_lowercase().contains(&needle)
            })
            .map(|o| {
                let people = plural(o.people, "person", "people");
                Cell {
                    id: &o.label,
                    name: &o.label,
                    hover: format!(
                        "{} · {people} · {} — click for the people",
                        o.name,
                        plural(o.messages, "message", "messages")
                    ),
                    figure: if self.list.messages {
                        plural(o.messages, "message", "messages")
                    } else {
                        people
                    },
                    measure: if self.list.messages {
                        o.messages
                    } else {
                        o.people
                    },
                }
            })
            .collect();
        let most = self
            .list
            .orgs
            .iter()
            .map(|o| {
                if self.list.messages {
                    o.messages
                } else {
                    o.people
                }
            })
            .max();
        let mut clicked = None;
        grid(
            ui,
            &cells,
            most,
            "Nothing matches the filter",
            Some(&mut clicked),
            p,
        );
        // An organisation's people: the People tab, filtered to it.
        if let Some(label) = clicked {
            self.filter = label;
            self.tab = PEOPLE;
        }
    }
}

/// Cells in as many columns as fit. `clicked` makes them clickable and
/// receives the id of the one clicked.
fn grid(
    ui: &mut Ui,
    cells: &[Cell],
    most: Option<usize>,
    empty: &str,
    mut clicked: Option<&mut Option<String>>,
    p: &Palette,
) {
    if cells.is_empty() {
        EmptyState::new(empty, None).show(ui, p, true);
        return;
    }
    let most = most.unwrap_or(1).max(1) as f32;
    let width = ui.available_width() - 2.0 * GUTTER;
    let columns = ((width / CELL_W).floor() as usize).max(1);
    let cell_w = width / columns as f32;
    let rows = cells.len().div_ceil(columns);
    let sense = if clicked.is_some() {
        Sense::click()
    } else {
        Sense::hover()
    };
    egui::ScrollArea::vertical()
        .id_salt("people-grid")
        .auto_shrink([false, false])
        .show_rows(ui, CELL_H, rows, |ui, range| {
            for r in range {
                let (line, _) = ui
                    .allocate_exact_size(egui::vec2(ui.available_width(), CELL_H), Sense::hover());
                for c in 0..columns {
                    let Some(cell) = cells.get(r * columns + c) else {
                        break;
                    };
                    let left = line.left() + GUTTER + c as f32 * cell_w;
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(left, line.top()),
                        egui::vec2(cell_w - space::STEP * 2.0, CELL_H),
                    );
                    if draw(ui, rect, cell, most, sense, p).clicked() {
                        if let Some(out) = clicked.as_deref_mut() {
                            *out = Some(cell.id.to_string());
                        }
                    }
                }
            }
        });
}

/// One cell: name and figure on a line, the bar under them.
fn draw(
    ui: &mut Ui,
    rect: egui::Rect,
    cell: &Cell,
    most: f32,
    sense: Sense,
    p: &Palette,
) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(cell.id), sense);
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(4), p.surface);
    }
    let text_rect =
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.bottom() - 10.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(Layout::right_to_left(Align::Center)),
        |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            ui.label(meta(&cell.figure, p));
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(text(cell.name, p).strong()).truncate());
            });
        },
    );
    let track = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.bottom() - 8.0),
        egui::vec2(rect.width(), BAR_H),
    );
    let painter = ui.painter();
    painter.rect_filled(track, egui::CornerRadius::ZERO, p.rule);
    let mut fill = track;
    fill.set_width(track.width() * cell.measure as f32 / most);
    painter.rect_filled(fill, egui::CornerRadius::ZERO, p.muted);
    response.on_hover_text(&cell.hover)
}

/// "1,204 people · 38,511 messages · from an export · counted yesterday"
fn caption(chat: &ChatRow, now: SystemTime) -> String {
    let what = match chat.messages {
        Some(n) => format!(
            "{} · {}",
            plural(chat.people, "person", "people"),
            plural(n, "message", "messages")
        ),
        None => format!(
            "{} · members only",
            plural(chat.people, "member", "members")
        ),
    };
    let from = if chat.from_export {
        "from an export"
    } else {
        "from Telegram"
    };
    let when = UNIX_EPOCH + Duration::from_secs(chat.updated.max(0) as u64);
    format!(
        "{what} · {from} · counted {}",
        tga_run::state::human_when(when, now)
    )
}

fn plural(n: usize, one: &str, many: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{} {many}", tgx_ui::thousands(n as i64)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, from_export: bool, file_gone: bool) -> ChatRow {
        ChatRow {
            id: id.into(),
            title: "t".into(),
            people: 1,
            messages: None,
            updated: 0,
            from_export,
            file_gone,
        }
    }

    #[test]
    fn a_chat_is_gone_only_on_evidence() {
        let ids: HashSet<i64> = [1].into();
        let loaded = Known {
            telegram: Some(&ids),
        };
        let unloaded = Known { telegram: None };
        // Left on Telegram: gone, but only once the chat list says so.
        assert!(gone(&chat("2", false, false), &loaded));
        assert!(!gone(&chat("2", false, false), &unloaded));
        assert!(!gone(&chat("1", false, false), &loaded));
        // An export is judged by its file, not by the Telegram list.
        assert!(!gone(&chat("2", true, false), &loaded));
        assert!(gone(&chat("1", true, true), &unloaded));
        // A key that is no chat id is never gone from Telegram.
        assert!(!gone(&chat("export:x", false, false), &loaded));
    }
}
