//! Keyboard shortcuts.
//!
//! | keys            | does                                              |
//! |-----------------|---------------------------------------------------|
//! | Ctrl+1 … Ctrl+5 | Chats, Activity, Analysis, Summaries, Settings    |
//! | Ctrl+F          | the view's filter or search box                   |
//! | Ctrl+A          | tick every chat shown (Chats, outside a text box) |
//! | Ctrl+E          | Export — the Chats view's primary                 |
//! | F5              | reload the chat list, or re-read Analysis         |
//! | Esc             | clear the chat filter                             |
//! | Ctrl +, −, 0    | zoom — egui's own, kept between launches          |
//!
//! **Nothing while the sign-in dialog is up**: it holds every key, and Esc
//! there means Cancel. And Ctrl+A in a text box is the text box's.

use super::*;
use eframe::egui::{Key, KeyboardShortcut, Modifiers};

/// What a key press asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shortcut {
    Go(View),
    Find,
    TickAll,
    Export,
    Reload,
    ClearFilter,
}

impl Shell {
    /// Read this frame's shortcuts and act on them. Before the panels, so a
    /// focus asked for here is taken when the field is drawn.
    pub(super) fn shortcuts(&mut self, ctx: &Context) {
        if self.login.is_some() {
            return;
        }
        let typing = ctx.wants_keyboard_input();
        let cmd = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let mut asked = Vec::new();
        ctx.input_mut(|i| {
            for (n, view) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5]
                .into_iter()
                .zip(View::ALL)
            {
                if i.consume_shortcut(&cmd(n)) {
                    asked.push(Shortcut::Go(view));
                }
            }
            if i.consume_shortcut(&cmd(Key::F)) {
                asked.push(Shortcut::Find);
            }
            if i.consume_shortcut(&cmd(Key::E)) {
                asked.push(Shortcut::Export);
            }
            if !typing && i.consume_shortcut(&cmd(Key::A)) {
                asked.push(Shortcut::TickAll);
            }
            if i.consume_key(Modifiers::NONE, Key::F5) {
                asked.push(Shortcut::Reload);
            }
            if !self.search.is_empty()
                && self.body == View::Chats
                && i.consume_key(Modifiers::NONE, Key::Escape)
            {
                asked.push(Shortcut::ClearFilter);
            }
        });
        for s in asked {
            self.shortcut(s, ctx);
        }
    }

    /// Do what `s` asks, if this view and this moment allow it — the same
    /// conditions the button it stands for is enabled on.
    pub(super) fn shortcut(&mut self, s: Shortcut, ctx: &Context) {
        match s {
            Shortcut::Go(view) => self.show(view),
            Shortcut::Find => match self.body {
                View::Chats | View::Viewer => self.focus_filter = true,
                View::Analysis => self.analysis.focus_filter = true,
                View::Summaries => self.summaries.focus_filter = true,
                View::Queue | View::Settings => {}
            },
            Shortcut::TickAll if self.body == View::Chats => {
                let ids: Vec<i64> = self.visible().iter().map(|c| c.id).collect();
                self.selected.extend(ids);
            }
            Shortcut::Export if self.body == View::Chats && self.signed_in => {
                let paused = !self.exporting && self.queue.is_paused();
                if paused {
                    let picked = self.ticked_chats();
                    self.resume_queue(picked);
                } else if !self.selected.is_empty() && !self.counting {
                    self.start_export();
                }
            }
            Shortcut::Reload => match self.body {
                View::Chats if self.signed_in && !self.exporting && !self.counting => {
                    self.start_refresh()
                }
                View::Analysis | View::Summaries => self.analysis.reload(ctx),
                _ => {}
            },
            Shortcut::ClearFilter => {
                self.search.clear();
                self.set_filter(String::new());
            }
            _ => {}
        }
    }
}
