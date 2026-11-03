//! The Viewer, in this window: [`tgx_viewer::Viewer`] behind Analysis, with a
//! way back and Export as folder.
//!
//! The screen itself is `tgx-viewer`'s, shared with TelegramViewer.exe. What
//! is this window's is here: where the work runs (the bridge), where the
//! status line and the warnings go (this window's own), which folder it
//! reads (the destination), the way back to Analysis or Summaries, and
//! Export as folder, which needs the exporter and so cannot be in the
//! Viewer alone.

use super::*;
use crate::bridge::Bridge;
use eframe::egui::Ui;
use tgx_viewer::{Asked, Head, Host, Job, Reply};

/// The Viewer, and where its way back goes.
#[derive(Default)]
pub(super) struct ViewerState {
    pub(super) inner: tgx_viewer::Viewer,
    /// Where the way back goes: Analysis, unless the Viewer was opened
    /// from a summary's citation.
    pub(super) back: Option<View>,
}

/// So the shell and its tests read the Viewer's state as `self.viewer.rows`,
/// as they did when it was this window's own.
impl std::ops::Deref for ViewerState {
    type Target = tgx_viewer::Viewer;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for ViewerState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// [`Host`] for this window. **Every reply goes back through the bridge's
/// `Events`**, which wakes the window on each send — the rule in
/// `shell/mod.rs`'s header, kept.
struct ShellHost<'a> {
    bridge: &'a Bridge,
    status: &'a mut String,
    journal: &'a mut Journal,
    log_copied: &'a mut bool,
}

impl Host for ShellHost<'_> {
    fn run(&mut self, job: Job) {
        let tx = self.bridge.sender();
        self.bridge.spawn(async move {
            tgx_viewer::jobs::run(job, move |reply| {
                let _ = tx.send(Event::Viewer(reply));
            })
            .await
        });
    }

    fn status(&mut self, line: String) {
        *self.status = line;
    }

    fn warn(&mut self, line: String) {
        self.journal.warn(line);
        *self.log_copied = false;
    }
}

/// What the empty Viewer says: chats are opened from Analysis now, so the
/// way out of an empty Viewer is back there — not a picker this screen does
/// not have.
const NONE_OPEN: &str = "Open one with Read on its row in Analysis.";

impl Shell {
    /// The Viewer and a host for it, with its folder brought up to the
    /// destination first — the destination is free text and can change under
    /// the view.
    fn viewer_parts(&mut self) -> (&mut tgx_viewer::Viewer, ShellHost<'_>) {
        if self.viewer.folder != self.settings.output_dir {
            self.viewer.folder = self.settings.output_dir.clone();
        }
        (
            &mut self.viewer.inner,
            ShellHost {
                bridge: &self.bridge,
                status: &mut self.status,
                journal: &mut self.journal,
                log_copied: &mut self.log_copied,
            },
        )
    }

    pub(super) fn viewer_panel(&mut self, ui: &mut Ui) {
        let p = self.palette;
        // Ctrl+F, pressed with the Viewer on screen.
        if std::mem::take(&mut self.focus_filter) {
            self.viewer.focus_search = true;
        }
        let back_to = self.viewer.back.unwrap_or(View::Analysis);
        let head = Head {
            back: Some(back_to.label()),
            unpack: true,
            none_open: NONE_OPEN,
        };
        let (v, mut host) = self.viewer_parts();
        let asked = v.panel(ui, &p, &mut host, head);
        let Asked { back, unpack } = asked;
        if back {
            self.show(back_to);
        }
        if unpack {
            self.viewer_unpack();
        }
    }

    /// Why the list is empty, when it is.
    #[cfg(test)]
    pub(super) fn viewer_empty(&self) -> Option<tgx_ui::components::EmptyState> {
        self.viewer.empty(NONE_OPEN)
    }

    /// Export as folder: ask where, then write the open database out as the
    /// folders a Classic run would have made — deleted messages included.
    fn viewer_unpack(&mut self) {
        let Some(db) = self.viewer.open.clone() else {
            return;
        };
        let Some(out) = rfd::FileDialog::new()
            .set_title("Export as folder into…")
            .set_directory(&self.settings.output_dir)
            .pick_folder()
        else {
            return;
        };
        self.viewer.unpacking = true;
        self.status = format!("Writing {} out as a folder export…", db.title);
        let tx = self.bridge.sender();
        let settings = self.settings.clone();
        self.bridge
            .spawn(async move { crate::actions::unpack(db.path, out, settings, tx).await });
    }

    /// Export as folder finished. Logged, so the folder it went to can be
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

    /// The Analysis view's search box: search every database for `needle` and
    /// show the results in the reader, with no chat needing to be open first.
    ///
    /// `dir` is the Analysis folder, which may not be the destination: the
    /// results have to be of the exports the user was looking at.
    pub(super) fn search_everywhere(&mut self, needle: String, dir: String) {
        self.viewer.query = needle;
        self.viewer.everywhere = true;
        let (v, mut host) = self.viewer_parts();
        v.search_in(dir, &mut host);
        // Entered from Analysis, so the way back goes there — as `read`.
        self.viewer.back = None;
        self.show(View::Viewer);
    }

    // -- the Viewer's own verbs, with this window's host ---------------------

    pub(super) fn viewer_apply(&mut self, reply: Reply) {
        let (v, mut host) = self.viewer_parts();
        v.apply(reply, &mut host);
    }

    pub(super) fn viewer_open(&mut self, db: tgx_viewer::data::Database, then: Option<i64>) {
        let (v, mut host) = self.viewer_parts();
        v.open_db(db, then, &mut host);
    }

    pub(super) fn viewer_select(&mut self, id: i64, scroll: bool) {
        let (v, mut host) = self.viewer_parts();
        v.select(id, scroll, &mut host);
    }

    #[cfg(test)]
    pub(super) fn viewer_list_databases(&mut self) {
        let (v, mut host) = self.viewer_parts();
        v.list_databases(&mut host);
    }

    #[cfg(test)]
    pub(super) fn viewer_set_filter(&mut self, filter: tgx_viewer::Filter) {
        self.viewer.set_filter(filter);
    }

    #[cfg(test)]
    pub(super) fn viewer_set_topic(&mut self, topic: Option<i64>) {
        self.viewer.set_topic(topic);
    }

    #[cfg(test)]
    pub(super) fn viewer_set_sender(&mut self, sender: Option<String>) {
        self.viewer.set_sender(sender);
    }

    #[cfg(test)]
    pub(super) fn viewer_go_to_date(&mut self) {
        let (v, mut host) = self.viewer_parts();
        v.go_to_date(&mut host);
    }

    #[cfg(test)]
    pub(super) fn viewer_search(&mut self) {
        let (v, mut host) = self.viewer_parts();
        v.search(&mut host);
    }

    #[cfg(test)]
    pub(super) fn viewer_step_hit(&mut self, step: isize) {
        let (v, mut host) = self.viewer_parts();
        v.step_hit(step, &mut host);
    }

    #[cfg(test)]
    pub(super) fn viewer_open_hit(&mut self, chat: usize, hit: usize) {
        let (v, mut host) = self.viewer_parts();
        v.open_hit(chat, hit, &mut host);
    }
}
