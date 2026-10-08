//! What a control does when it is pressed: sign in, refresh, count, export,
//! stop, and the settings write-back.
//!
//! **One writer per fact.** A chat's message count has three sources -- the
//! Count button, the total an export looks up, and the number an export wrote
//! -- and one setter. A finished export once left the row showing one number
//! and sorting on another.

use super::*;

impl Shell {
    // -- settings ----------------------------------------------------------

    /// Read the fields back, persist, and write the stored values back in.
    pub(super) fn commit_settings(&mut self) {
        let dir_before = self.settings.output_dir.clone();
        self.form.collect(&mut self.settings);
        // The marks in the list describe the destination, so a new one needs
        // reading. Only on a change: this runs on every checkbox.
        if self.settings.output_dir != dir_before {
            self.scan_destination();
        }
        self.settings.sort_mode = self.view.sort.key().into();
        self.settings.group_by_type = self.view.grouped;
        // Stored in the fixed category order rather than in iteration order, so
        // two runs that folded the same categories write the same file and a
        // diff of settings.json means something changed.
        self.settings.folded_categories = Category::ALL
            .into_iter()
            .filter(|c| self.view.folded.contains(c))
            .map(|c| c.key().to_string())
            .collect();
        // **Never from a test.** A test binary under `target\` resolves its data
        // folder to the workspace root, so a test that pressed a button would
        // overwrite the developer's own settings.json — credentials included —
        // with `Settings::default()`.
        if !cfg!(test) {
            if let Err(e) = self.settings.save() {
                self.journal.warn(format!("could not save settings: {e}"));
            }
        }
        // **Written back at once**, not queued for the next frame. Under GPUI
        // this had to wait for the first moment a `&mut Window` was in hand, so
        // the shell carried a `needs_field_sync` flag and the render read it.
        // The form is plain data now, so a clamped entry is visible on the same
        // frame that clamped it.
        self.form.sync(&self.settings);
    }

    /// A checkbox changed. Same path as a text field, minus the parsing.
    pub(super) fn toggle_setting(&mut self, f: impl FnOnce(&mut Settings)) {
        f(&mut self.settings);
        self.commit_settings();
    }

    // -- actions -----------------------------------------------------------

    /// Open the sign-in dialog, or **raise the existing one**.
    ///
    /// Making a second dialog is what put two modals on top of each other,
    /// which the user experienced as the app freezing the moment it logged
    /// them in.
    pub(super) fn open_login(&mut self) {
        if self.login.is_some() {
            return;
        }
        let stage = if self.settings.api_id == 0 || self.settings.api_hash.is_empty() {
            Stage::Credentials
        } else {
            Stage::Phone
        };
        let api_id = if self.settings.api_id == 0 {
            String::new()
        } else {
            self.settings.api_id.to_string()
        };
        let hash = self.settings.api_hash.clone();
        let phone = self.settings.phone.clone();
        self.login = Some(LoginDialog::new(stage, &api_id, &hash, &phone));
    }

    pub(super) fn start_sign_in(&mut self) {
        // Probe the session on disk, and open the dialog without waiting for
        // the answer. A signed-in account answers `SignedIn`, which closes the
        // dialog again — so the cost of already being signed in is that the
        // dialog appears for as long as the connect takes, and the cost of
        // waiting instead would be a button that does nothing visible while a
        // network round trip happens. The first is the better trade, but it is
        // a trade: this does *not* skip the dialog for a good session.
        let tx = self.bridge.sender();
        let settings = self.settings.clone();
        self.bridge
            .spawn(async move { crate::actions::sign_in(settings, tx).await });
        if !self.signed_in {
            self.open_login();
        }
    }

    pub(super) fn start_refresh(&mut self) {
        let tx = self.bridge.sender();
        let settings = self.settings.clone();
        self.bridge
            .spawn(async move { crate::actions::refresh_chats(settings, tx).await });
    }

    /// Count every chat, or stop a count already running.
    ///
    /// The button has to be able to undo itself: a count can sit in a
    /// two-minute rate-limit wait, and with the button merely disabled that is
    /// indistinguishable from a hang.
    pub(super) fn start_count(&mut self) {
        if self.counting {
            self.cancel.cancel();
            self.status = "Stopping the count…".into();
            return;
        }
        if self.chats.is_empty() {
            return;
        }
        self.cancel.reset();
        self.counting = true;
        let ids: Vec<i64> = self.chats.iter().map(|c| c.id).collect();
        self.count_progress = Some((0, ids.len()));
        let tx = self.bridge.sender();
        let settings = self.settings.clone();
        let cancel = self.cancel.clone();
        self.bridge
            .spawn(async move { crate::actions::count_chats(settings, ids, cancel, tx).await });
    }

    /// The per-run state a new export resets.
    ///
    /// Split out of [`Self::start_export`] because that needs a `Window` and
    /// this does not, so the reset can be tested — which matters most for
    /// `failure`, whose whole failure mode is being *left set*.
    pub(super) fn begin_run(&mut self) {
        // An export is the longer job and it claims the progress bar here.
        // While `exporting` is set, no other handler may write to it.
        self.cancel.reset();
        self.exporting = true;
        // **Cleared on the way in, not on the way out.** `failure` is appended
        // to the run's summary, so a cause left over from an earlier run — a
        // sign-in that could not reach Telegram, a destination that was
        // unwritable last time — was reported as the reason *this* run ended.
        self.failure = None;
        self.count_progress = None;
    }

    /// Start a run, **or add to the one already going**.
    ///
    /// One button, two questions, and which is being asked is decided here and
    /// nowhere else. The bar's third cell says which — "Start export" idle,
    /// "Add to queue" mid-run — so the branch below is not a hidden mode.
    ///
    /// **A finished run is cleared before the next one begins.** Not doing so
    /// left an hour-old row sitting above the run that was actually happening,
    /// with its own counts, its own folder and a state that read as current.
    /// Both lists go: [`Queue::start`] replaces the table, and
    /// [`Pending::reset`] replaces the worker's work list.
    pub(super) fn start_export(&mut self) {
        // Whatever is in the fields is what the run should use, so the fields
        // are read here rather than trusted to have been committed already.
        self.commit_settings();
        if !self.settings.writes_anything() {
            self.journal
                .warn("Nothing to write: pick HTML or JSON under Classic, or choose Database.");
            self.status = "No output format selected".into();
            return;
        }
        let picked = self.ticked_chats();
        self.run_chats(picked);
    }

    /// The chats ticked in the list, in list order.
    pub(super) fn ticked_chats(&self) -> Vec<ChatInfo> {
        self.chats
            .iter()
            .filter(|c| self.selected.contains(&c.id))
            .cloned()
            .collect()
    }

    /// Start a run of exactly these chats, or add them to the one going.
    ///
    /// Split out of [`Self::start_export`] so Update databases can queue the
    /// chats it found **without rewriting the ticks**: a selection the user
    /// built up over six hundred rows is theirs, and a button that quietly
    /// replaced it would be undoing work it was never asked about.
    fn run_chats(&mut self, picked: Vec<ChatInfo>) {
        if picked.is_empty() {
            return;
        }

        if self.exporting {
            self.add_to_run(picked);
            return;
        }
        // A stopped run is carried on, not replaced: its rows are still wanted.
        if self.queue.is_paused() {
            self.resume_queue(picked);
            return;
        }

        // An export is the longer job and it claims the progress bar here.
        // While `exporting` is set, no other handler may write to it.
        self.begin_run();
        // **And it claims the window.** Pressing Start used to leave the user on
        // the chat list, where the only sign anything had happened was a
        // sentence in the status bar; everything that actually reports the run
        // was in a 400pt column on the right. A run that has just been asked for
        // is what the window should be showing. `suggest`, not `show`, so
        // looking at Settings mid-export is not undone on the next event.
        self.body_pinned = false;
        self.suggest(View::Queue);
        self.queue
            .start(picked.iter().map(|c| (c.id, c.title.clone())));
        self.pending.reset(picked.clone());
        self.status = format!("Exporting {} chats…", picked.len());
        self.journal.push(format!(
            "Starting {} export(s) into {}",
            picked.len(),
            self.settings.output_dir
        ));
        self.spawn_export();
    }

    /// Find out which chats the destination already holds, for the list's
    /// marks. No connection, and nothing written — see
    /// [`crate::actions::scan_destination`].
    pub(super) fn scan_destination(&mut self) {
        let tx = self.bridge.sender();
        let settings = self.settings.clone();
        self.bridge
            .spawn(async move { crate::actions::scan_destination(settings, tx).await });
    }

    /// **Update databases: sync every chat that already has one here.**
    ///
    /// The chats come from inside the files in the destination, not from the
    /// ticks, so pointing Destination at a folder of databases and pressing
    /// this is the whole of keeping them current. See [`Self::update_found`]
    /// for what happens with the answer.
    pub(super) fn start_update(&mut self) {
        if self.scanning {
            return;
        }
        // The destination as typed, not as last committed — the same rule as
        // Start export.
        self.commit_settings();
        self.scanning = true;
        let tx = self.bridge.sender();
        let settings = self.settings.clone();
        self.bridge
            .spawn(async move { crate::actions::find_archives(settings, tx).await });
    }

    /// Check folder: look through the destination and log what is wrong.
    pub(super) fn check_folder(&mut self) {
        self.commit_settings();
        self.checking = true;
        let tx = self.bridge.sender();
        let settings = self.settings.clone();
        self.bridge
            .spawn(async move { crate::actions::check_folder(settings, tx).await });
    }

    /// Check folder came back. Problems are warnings, so Copy warnings takes
    /// them; notes are ordinary lines. The log is in the Queue view, which is
    /// shown, since the button is on another.
    pub(super) fn folder_checked(&mut self, found: Result<Vec<String>, String>) {
        self.checking = false;
        self.log_copied = false;
        let dir = self.settings.output_dir.clone();
        match found {
            Err(e) => self.journal.warn(e),
            Ok(lines) if lines.is_empty() => {
                self.journal.push(format!("Checked {dir}: nothing wrong"));
            }
            Ok(lines) => {
                self.journal
                    .push(format!("Checked {dir}: {} finding(s)", lines.len()));
                for line in lines {
                    if line.starts_with("PROBLEM") {
                        self.journal.warn(line);
                    } else {
                        self.journal.push(format!("  {line}"));
                    }
                }
                self.journal.push(
                    "  Fixes are made from the command line: tgx doctor --fix, with no export \
                     running",
                );
            }
        }
        self.show(View::Queue);
    }

    /// Find duplicate media: read the destination off the window's thread and
    /// log what is stored more than once.
    pub(super) fn find_dupes(&mut self) {
        if self.deduping {
            return;
        }
        self.commit_settings();
        self.deduping = true;
        let tx = self.bridge.sender();
        let settings = self.settings.clone();
        self.bridge
            .spawn(async move { crate::actions::find_dupes(settings, tx).await });
    }

    /// Find duplicate media came back. **Nothing here is a warning**: a second
    /// copy is space, not damage, and a warning would put it in Copy warnings
    /// beside an INCOMPLETE export. The log is in the Queue view, which is
    /// shown, since the button is on another.
    pub(super) fn dupes_found(&mut self, found: Result<Vec<String>, String>) {
        self.deduping = false;
        self.log_copied = false;
        let dir = self.settings.output_dir.clone();
        match found {
            Err(e) => self.journal.warn(e),
            Ok(lines) => {
                self.status = format!("Looked for duplicate media in {dir}");
                self.journal
                    .push(format!("Duplicate media in {dir}: nothing is changed"));
                for line in lines {
                    self.journal.push(format!("  {line}"));
                }
            }
        }
        self.show(View::Queue);
    }

    /// Restart the scheduled sync's clock from `now` — on launch, when the
    /// interval is changed, and when a run ends.
    pub(super) fn arm_schedule(&mut self, now: std::time::Instant) {
        self.next_sync = super::schedule_from(&self.settings, now);
    }

    /// Fire the scheduled sync if it is due, and say how long until it next
    /// needs a frame. Run once a frame; `now` is passed in so a test can stand
    /// in for the clock.
    ///
    /// **It yields to everything.** Due while a run, a scan or the sign-in is
    /// still going, it looks again in a minute rather than queuing behind —
    /// Update databases while a Classic run is going would be refused anyway,
    /// and stacking a sync behind a sync is what the clock restarting at the
    /// end of each run exists to prevent.
    pub(super) fn tick_schedule(&mut self, now: std::time::Instant) -> Option<std::time::Duration> {
        let due = self.next_sync?;
        if now < due {
            return Some(due - now);
        }
        const RETRY: std::time::Duration = std::time::Duration::from_secs(60);
        if self.exporting || self.scanning || self.counting || !self.loaded {
            self.next_sync = Some(now + RETRY);
            return Some(RETRY);
        }
        if !self.settings.export_db {
            // Switched to Classic since it was armed: a Classic run is not
            // something to start unasked.
            self.next_sync = None;
            return None;
        }
        self.journal.push("Scheduled sync");
        self.start_update();
        self.arm_schedule(now);
        self.next_sync.map(|at| at - now)
    }

    /// The scan came back. Queue what it found, in Database mode.
    ///
    /// **The mode is switched, and said so.** A database can only be synced by
    /// a Database run, and a Classic run of the same chats would write a fresh
    /// folder of each beside the databases the user asked to update — so the
    /// button sets the mode rather than refusing, and the log records it.
    ///
    /// A chat with a database but missing from the list — left, deleted, or
    /// on another account — is named rather than skipped in silence, because
    /// "updated 11 databases" in a folder of 12 is otherwise a question with no
    /// answer on screen.
    pub(super) fn update_found(&mut self, found: Result<Box<tgx_tg::engine::Archived>, String>) {
        self.scanning = false;
        let found = match found {
            Ok(found) => found,
            Err(e) => {
                self.status = format!("Failed: {e}");
                self.journal.warn(e);
                self.log_copied = false;
                return;
            }
        };
        for (path, why) in &found.unreadable {
            self.journal
                .warn(format!("skipped {}: {why}", path.display()));
        }
        let mut picked = Vec::new();
        for (id, title) in &found.chats {
            match self.chats.iter().find(|c| c.id == *id) {
                Some(chat) => picked.push(chat.clone()),
                None => self.journal.warn(format!(
                    "{title} has a database here but is not in this account's chat list"
                )),
            }
        }
        self.log_copied = false;
        if picked.is_empty() {
            self.status = format!("No databases to update in {}", self.settings.output_dir);
            self.journal.push(self.status.clone());
            return;
        }
        // A run started while the folder was being read is already a Classic
        // or a Database run with its own copy of the settings, and adding to
        // a Classic one would export these chats as folders.
        if self.exporting && !self.settings.export_db {
            self.status = "A Classic run is going — update the databases after it ends".into();
            self.journal.warn(self.status.clone());
            return;
        }
        if !self.settings.export_db {
            self.toggle_setting(|s| s.export_db = true);
            self.journal
                .push("Switched to Database mode to update the databases");
        }
        self.journal.push(format!(
            "Updating {} of {} databases found",
            picked.len(),
            found.chats.len()
        ));
        self.run_chats(picked);
    }

    /// Put more chats into a run that is already going.
    ///
    /// **Both lists, and only the ones the queue actually took.**
    /// [`Queue::append`] refuses a chat that is already waiting or in flight
    /// and re-queues one whose row has finished; pushing the whole selection
    /// into `pending` regardless would export the chats in flight a second
    /// time, with one row between them.
    fn add_to_run(&mut self, picked: Vec<ChatInfo>) {
        let queued = self
            .queue
            .append(picked.iter().map(|c| (c.id, c.title.clone())));
        if queued.is_empty() {
            self.status = "Already queued".into();
            return;
        }
        self.pending
            .extend(picked.into_iter().filter(|c| queued.contains(&c.id)));
        let n = queued.len();
        let chats = if n == 1 { "chat" } else { "chats" };
        self.status = format!("Added {n} {chats} to the queue");
        self.journal
            .push(format!("Added {n} {chats} to the run in progress"));
        self.log_copied = false;
    }

    /// **Carry on a stopped run**, with any newly ticked chats joining the end.
    ///
    /// The work list is rebuilt from the rows, so it cannot disagree with
    /// them. A chat that has left the chat list since cannot be reached and
    /// says so on its row rather than sitting at Queued.
    pub(super) fn resume_queue(&mut self, extra: Vec<ChatInfo>) {
        if self.exporting {
            return;
        }
        self.commit_settings();
        if !self.settings.writes_anything() {
            self.journal
                .warn("Nothing to write: pick HTML or JSON under Classic, or choose Database.");
            self.status = "No output format selected".into();
            return;
        }
        self.queue
            .append(extra.iter().map(|c| (c.id, c.title.clone())));
        let mut work = Vec::new();
        for id in self.queue.resume() {
            match self.chats.iter().find(|c| c.id == id) {
                Some(chat) => work.push(chat.clone()),
                None => self.queue.failed(id, "Not in the chat list".into()),
            }
        }
        if work.is_empty() {
            self.status = self.queue.summary();
            return;
        }
        self.begin_run();
        self.body_pinned = false;
        self.suggest(View::Queue);
        let n = work.len();
        self.pending.reset(work);
        let chats = if n == 1 { "chat" } else { "chats" };
        self.status = format!("Exporting {n} {chats}…");
        self.journal.push(format!(
            "Resuming the queue: {n} {chats} into {}",
            self.settings.output_dir
        ));
        self.log_copied = false;
        self.spawn_export();
    }

    /// **Clear: empty the queue**, rows and work list both. Only between runs —
    /// mid-run, × on a row and Stop are the controls.
    pub(super) fn clear_queue(&mut self) {
        if self.exporting || self.queue.is_empty() {
            return;
        }
        self.queue.clear();
        self.pending.clear();
        self.status = "Queue cleared".into();
        self.journal.push("Queue cleared");
        self.log_copied = false;
    }

    /// Hand the work list to a worker.
    ///
    /// Split out because [`Shell::resume_if_more_queued`] needs the same three
    /// lines: a run whose worker has finished while chats were being added is
    /// restarted rather than left with rows nothing will ever reach.
    pub(super) fn spawn_export(&mut self) {
        let tx = self.bridge.sender();
        let settings = self.settings.clone();
        let cancel = self.cancel.clone();
        let pending = self.pending.clone();
        self.bridge
            .spawn(async move { crate::actions::export(settings, pending, cancel, tx).await });
    }

    /// Ask the run to stop, and **wait for it to say it has**.
    ///
    /// It does not clear `exporting` here. That was the whole of the old Stop:
    /// it set a flag the worker never read, cleared the progress, wrote
    /// "Stopped", and the export ran to completion writing files — then fired
    /// its own `Finished` over the top of the message. The run is over when the
    /// worker says so, and until then the interface says "Stopping".
    pub(super) fn stop(&mut self) {
        self.cancel.cancel();
        if self.exporting {
            self.status = "Stopping…".into();
        }
        if self.counting {
            self.status = "Stopping the count…".into();
        }
    }
}
