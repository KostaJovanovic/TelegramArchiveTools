//! The settings panel: one column down the middle, every setting one box,
//! every explanation behind its own `?`.
//!
//! **Laid out like a game's options screen, on purpose** (2026-10-16, the
//! account holder's ask, after calling the two-column panel bad). A setting is
//! one wide box reading `Name: Value`; a click moves it to the next value and a
//! right-click to the one before. A number, a date or a path is the same box
//! with a field in it. Beside each box is a square `?` holding what the line
//! under every setting used to say, and the longer explanation where there was
//! one. The column scrolls; nothing else does.
//!
//! # What the earlier passes got wrong
//!
//! **Two columns of rows, each with a sentence under it.** Every setting was a
//! name, a sentence and a control at three different x positions, in two
//! columns, so there was no one line for the eye to run down. One column, one
//! box per setting with its state written inside it, is that line.
//!
//! **A `[?]` beside a bare switch made every switch a question.** That was
//! tried and taken back once. The difference now is that the box already says
//! what the setting is set to in words — `Download media: On` — so the `?` is
//! only ever for *why*, never for *what is this*.
//!
//! **Folder and Database replace each other**, they are not dimmed: the boxes
//! under Format are the chosen format's alone.
//!
//! **There is no "Chats at once".** This engine exports one chat at a time, and
//! a control that is enabled and does nothing teaches that the interface is
//! unreliable. An old `settings.json` naming it still loads, because unknown
//! keys are dropped per field.

use super::{list, Shell, THEMES};
use eframe::egui::{self, Align, Layout, Margin, Response, Sense, Stroke, Ui};
use tgx_ui::components::{action, button, eyebrow, field, meta, text, GUTTER};
use tgx_ui::tokens::{space, window, Palette};

/// The seven media categories Desktop offers, with the labels shown for each.
/// Keyed on `tgx_tg::config::MEDIA_KINDS`, which is what `plan.rs`
/// compares against — a label mismatch here would silently switch off a
/// category the settings file names.
const MEDIA_LABELS: [(&str, &str); 7] = [
    ("photos", "Photos"),
    ("video_files", "Videos"),
    ("voice_messages", "Voice messages"),
    ("video_messages", "Video messages"),
    ("stickers", "Stickers"),
    ("animations", "GIFs"),
    ("files", "Files"),
];

/// The extra requests, each with what its `?` says.
///
/// Each costs traffic, each is separately switchable, and each degrades to
/// nothing on failure.
const EXTRAS: [(&str, &str, &str); 6] = [
    (
        "full_reactions",
        "Full reaction lists",
        "Who reacted, not only how many. One request per message with reactions.",
    ),
    (
        "chat_metadata",
        "Chat details",
        "The chat's description, photo and settings. One request per chat.",
    ),
    (
        "invite_links",
        "Invite links",
        "Only served to an account that can administer the chat.",
    ),
    (
        "refresh_polls",
        "Current poll results",
        "Each poll as it stands now, not as it stood when it was sent.",
    ),
    (
        "scheduled_messages",
        "Scheduled messages",
        "Messages queued to send later. Desktop leaves them out.",
    ),
    (
        "member_roster",
        "Member list",
        "Who is in the chat, as participants.json. Capped by Member list cap.",
    ),
];

impl Shell {
    pub(super) fn settings_panel(&mut self, ui: &mut Ui) {
        // No `SETTINGS` heading: the view bar above already says so, and a view
        // does not need to introduce itself.
        egui::ScrollArea::vertical()
            .id_salt("settings-body")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                column(ui, |ui| {
                    self.unreadable_settings(ui);
                    self.destination_section(ui);
                    self.contents_section(ui);
                    self.media_section(ui);
                    self.reports_section(ui);
                    self.summaries_section(ui);
                    self.appearance_section(ui);
                    self.advanced_section(ui);
                    ui.add_space(space::BREAK);
                });
            });
    }

    /// `settings.json` would not read at start, so nothing here is saved over
    /// it — it may still hold the credentials — until this is pressed. The
    /// file is kept as `settings.json.bad` either way.
    fn unreadable_settings(&mut self, ui: &mut Ui) {
        let Some(why) = tgx_tg::config::settings_unreadable() else {
            return;
        };
        let p = self.palette;
        ui.add_space(space::STEP);
        ui.add(
            egui::Label::new(text(
                format!(
                    "Your settings file was not used: {why}. Changes here are not saved until \
                     you choose to replace it."
                ),
                &p,
            ))
            .wrap(),
        );
        let width = ui.available_width();
        if option_box(ui, width, "Replace it with these settings", true, &p).clicked() {
            tgx_tg::config::allow_settings_overwrite();
            self.commit_settings();
            self.status = "Settings saved over the unreadable file".into();
        }
    }

    /// **The path's whole is the tooltip.** A field scrolled to its caret
    /// renders a long path as a drive letter sliced in half; hovering says the
    /// rest.
    fn destination_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        section(ui, "Where exports go", &p);
        let (commit, pressed) = path_option(
            ui,
            "output_dir",
            "Export folder",
            "Each chat gets a folder here, or a database in a folder of its own. \
             Analysis lists what is in it.",
            &mut self.form.output_dir,
            "Where exports go",
            &["Browse"],
            &p,
        );
        // Committing on losing focus rather than on every keystroke: a
        // half-typed path is not a decision.
        if commit {
            self.commit_settings();
        }
        if pressed.is_some() {
            self.browse_for_output_dir();
        }
    }

    /// **Folder or Database — an exclusive choice, and it changes what is on
    /// the page.** The boxes under it are the chosen format's alone; then what
    /// both formats ask Telegram for.
    ///
    /// Split forum topics stays below the format's own boxes because it is the
    /// one setting both formats read — a folder per topic, or the topic
    /// recorded per message.
    fn contents_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let db = self.settings.export_db;
        section(ui, "What an export contains", &p);

        if let Some(i) = cycle(
            ui,
            "export_db",
            "Format",
            &["Folder", "Database"],
            usize::from(db),
            true,
            MODE_HELP,
            &p,
        ) {
            self.toggle_setting(move |s| s.export_db = i == 1);
        }

        if db {
            if self.number(
                ui,
                "reread_window",
                "Messages to re-read",
                "How far back each sync looks again. 0 is new messages only.",
                true,
                |f| &mut f.reread_window,
            ) {
                self.commit_settings();
            }
            if switch(
                ui,
                "reread_all",
                "Check the whole history",
                self.settings.reread_all,
                true,
                REREAD_ALL_HELP,
                &p,
            ) {
                self.toggle_setting(|s| s.reread_all = !s.reread_all);
            }
            if self.number(
                ui,
                "sync_every",
                "Sync every (hours)",
                "Updates every database in the export folder on a timer while the window is \
                 open. 0 is off.",
                true,
                |f| &mut f.sync_every,
            ) {
                self.commit_settings();
                self.arm_schedule(std::time::Instant::now());
            }
            // Here and not under Media: only a database keeps them, so the
            // choice is on screen only when there is one.
            let at = USERPICS
                .iter()
                .position(|k| *k == self.settings.profile_photos)
                .unwrap_or(1);
            if let Some(i) = cycle(
                ui,
                "profile_photos",
                "Profile pictures",
                &["Off", "Current", "History"],
                at,
                true,
                PHOTOS_HELP,
                &p,
            ) {
                self.toggle_setting(move |s| s.profile_photos = USERPICS[i].into());
            }
        } else {
            let rows: [(&str, &str, &str, bool, Flip); 2] = [
                (
                    "export_html",
                    "HTML pages",
                    "Desktop's browsable pages, line for line.",
                    self.settings.export_html,
                    |s| s.export_html = !s.export_html,
                ),
                (
                    "export_json",
                    "result.json",
                    "Every message as data, byte for byte as Desktop writes it.",
                    self.settings.export_json,
                    |s| s.export_json = !s.export_json,
                ),
            ];
            for (key, label, why, on, flip) in rows {
                if switch(ui, key, label, on, true, why, &p) {
                    self.toggle_setting(flip);
                }
            }
            if self.number(
                ui,
                "page_size",
                "Messages per page",
                "Desktop's default is 1,000. Held between 50 and 20,000.",
                true,
                |f| &mut f.page_size,
            ) {
                self.commit_settings();
            }
            self.date_rows(ui);
        }

        if switch(
            ui,
            "split_topics",
            "Split forum topics",
            self.settings.split_topics,
            true,
            "A folder per topic in a forum group — what Telegram Desktop cannot do.",
            &p,
        ) {
            self.toggle_setting(|s| s.split_topics = !s.split_topics);
        }
        for (key, label, why) in EXTRAS {
            let on = extra(&self.settings, key);
            if switch(ui, key, label, on, true, why, &p) {
                self.toggle_setting(move |s| set_extra(s, key, !extra(s, key)));
            }
        }
        if self.number(
            ui,
            "member_limit",
            "Member list cap",
            "Stop the member list after this many. 0 is no cap.",
            self.settings.member_roster,
            |f| &mut f.member_limit,
        ) {
            self.commit_settings();
        }
        // Not among the extras: they are requests, and this costs none — it
        // changes which of two names already in hand gets written.
        if switch(
            ui,
            "own_names",
            "Contacts by @username",
            self.settings.own_names,
            true,
            "Writes a contact's @username, not the name you saved them under, which only \
             you see.",
            &p,
        ) {
            self.toggle_setting(|s| s.own_names = !s.own_names);
        }
    }

    fn media_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let media_on = self.settings.download_media;
        section(ui, "Media", &p);
        if switch(
            ui,
            "download_media",
            "Download media",
            media_on,
            true,
            "Off, the export still records every file's name, size and type, and is far \
             faster.",
            &p,
        ) {
            self.toggle_setting(|s| s.download_media = !s.download_media);
        }

        // **Two to a line.** Seven categories are one question — which kinds —
        // so they share one `?`, on the first line.
        let mut toggled: Option<&'static str> = None;
        for (n, pair) in MEDIA_LABELS.chunks(2).enumerate() {
            let why = (n == 0).then_some(
                "Which kinds of file to download. A kind left off is recorded as Desktop \
                 records one it skipped.",
            );
            let kinds = &self.settings.media_kinds;
            option(ui, "media_kinds", why, &p, |ui, width| {
                let half = (width - ui.spacing().item_spacing.x) / 2.0;
                for (key, label) in pair {
                    let on = kinds.iter().any(|k| k == key);
                    let caption = format!("{label}: {}", on_off(on));
                    if option_box(ui, half, &caption, media_on, &p).clicked() {
                        toggled = Some(*key);
                    }
                }
            });
        }
        if let Some(key) = toggled {
            self.toggle_setting(move |s| {
                if let Some(at) = s.media_kinds.iter().position(|k| k == key) {
                    s.media_kinds.remove(at);
                } else {
                    s.media_kinds.push(key.to_string());
                }
            });
        }

        if self.number(
            ui,
            "size_limit",
            "Size limit, MB",
            "Larger files are recorded, not downloaded. 0 is no limit.",
            media_on,
            |f| &mut f.size_limit,
        ) {
            self.commit_settings();
        }
        if switch(
            ui,
            "link_previews",
            "Link-preview images",
            self.settings.link_previews,
            media_on,
            "Desktop does not save these. On, every later photo's number shifts by one.",
            &p,
        ) {
            self.toggle_setting(|s| s.link_previews = !s.link_previews);
        }
    }

    /// What the analyser writes, and when. The three run options were the
    /// Analysis view's, for the session only; they are kept with the rest now.
    fn reports_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        section(ui, "Analysis", &p);
        let rows: [(&str, &str, &str, bool, Flip); 4] = [
            (
                "report_after_export",
                "Report after each export",
                "Writes each chat's report beside it while the next chat downloads.",
                self.settings.report_after_export,
                |s| s.report_after_export = !s.report_after_export,
            ),
            (
                "report_digest",
                "Write the digest",
                "digest.jsonl and EVENTS.md beside the report, for a model to read.",
                self.settings.report_digest,
                |s| s.report_digest = !s.report_digest,
            ),
            (
                "report_fonts",
                "Embed fonts",
                "Looks right on any machine. Off, each report is about 300 KB smaller.",
                self.settings.report_fonts,
                |s| s.report_fonts = !s.report_fonts,
            ),
            (
                "report_handles",
                "People by @handle",
                "Never by the name this account saved them under.",
                self.settings.report_handles,
                |s| s.report_handles = !s.report_handles,
            ),
        ];
        for (key, label, why, on, flip) in rows {
            if switch(ui, key, label, on, true, why, &p) {
                self.toggle_setting(flip);
            }
        }
    }

    /// The local model the Summaries view talks to. **A port, never an
    /// address**: the model reads other people's conversation, so it is on
    /// this machine.
    fn summaries_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        section(ui, "Summaries", &p);
        let mut commit = false;
        let folder = self.settings.summary_models_dir();
        self.summaries.look_for_models(&folder, ui.ctx());
        // Only a look at *this* folder answers for it; until one arrives the
        // boxes say so rather than "none in the folder".
        let found = self.summaries.found.as_ref().filter(|f| f.folder == folder);
        let looking = found.is_none();
        let (models, found_server) = found
            .map(|f| (f.models.clone(), f.server.clone()))
            .unwrap_or_default();
        let in_folder = |path: &std::path::Path| {
            path.strip_prefix(&folder)
                .unwrap_or(path)
                .display()
                .to_string()
        };

        let (typed, models_button) = path_option(
            ui,
            "summary_models",
            "Models folder",
            "The models and llama-server.exe, each in it or in a folder of its own inside it.",
            &mut self.form.summary_models,
            "Where the models are",
            &["Browse", "Open"],
            &p,
        );
        commit |= typed;

        let chosen = super::summaries::chosen_model(&self.settings, &models);
        let about_model = if looking {
            "Looking in the models folder…".to_string()
        } else if models.is_empty() {
            "None in the models folder yet: see How to add a model, below.".to_string()
        } else {
            format!(
                "{} in the models folder. The prompts are written for Gemma.",
                match models.len() {
                    1 => "One model".to_string(),
                    n => format!("{n} models"),
                }
            )
        };
        let mut picked: Option<String> = None;
        option(
            ui,
            "summary_model",
            Some(about_model.as_str()),
            &p,
            |ui, width| {
                let shown = chosen
                    .as_deref()
                    .map(&in_folder)
                    .unwrap_or_else(|| "None".into());
                let hit = option_box(ui, width, &format!("Model: {shown}"), true, &p);
                egui::Popup::menu(&hit)
                    .id(egui::Id::new("summary-model"))
                    .width(width)
                    .show(|ui| {
                        if models.is_empty() {
                            ui.label(meta("None in the folder", &p));
                        }
                        for m in &models {
                            let name = in_folder(m);
                            if ui
                                .selectable_label(chosen.as_ref() == Some(m), &name)
                                .clicked()
                            {
                                picked = Some(name);
                            }
                        }
                    });
            },
        );

        let about_server = if !self.settings.summary_server.is_empty() {
            "The program that runs the model. Chosen here, so the models folder's is not used."
                .to_string()
        } else if let Some(exe) = &found_server {
            format!(
                "The program that runs the model. Blank uses {}, in the models folder.",
                in_folder(exe)
            )
        } else if looking {
            "The program that runs the model. Looking in the models folder…".to_string()
        } else {
            "The program that runs the model. None in the models folder: see How to add a \
             model, below, or choose one."
                .to_string()
        };
        let (typed, server_button) = path_option(
            ui,
            "summary_server",
            "Model server",
            &about_server,
            &mut self.form.summary_server,
            "Blank: llama-server.exe in the models folder",
            &["Browse"],
            &p,
        );
        commit |= typed;
        commit |= self.number(
            ui,
            "summary_port",
            "Port",
            "On 127.0.0.1, this machine only. A Gemma already serving there is used as it is.",
            true,
            |f| &mut f.summary_port,
        );
        let (typed, _) = path_option(
            ui,
            "summary_options",
            "Server options",
            "For a server started from here. Blank suits Gemma 4 26B on a 16 GB card.",
            &mut self.form.summary_options,
            tga_run::tga_sum::server::DEFAULT_OPTIONS,
            &[],
            &p,
        );
        commit |= typed;
        commit |= self.number(
            ui,
            "summary_idle",
            "Unload after (minutes)",
            "How long after you leave Summaries a server started here is stopped, to free \
             the graphics card.",
            true,
            |f| &mut f.summary_idle,
        );
        let lang = {
            use tga_run::tga_sum::Lang;
            let at = Lang::ALL
                .iter()
                .position(|l| *l == self.summaries.lang)
                .unwrap_or(0);
            cycle(
                ui,
                "summary_lang",
                "Language",
                &["Serbian", "English", "As the chat"],
                at,
                true,
                "What summaries and catch-ups are written in. Serbian is Latin, ekavian.",
                &p,
            )
            .map(|i| Lang::ALL[i])
        };
        let photos = switch(
            ui,
            "summary_photos",
            "Read text in photos",
            self.settings.summary_photos,
            true,
            "PaddleOCR-VL, if it is in the models folder, reads the days' photos first, \
             then makes way for the model.",
            &p,
        );
        self.how_to_add_a_model(ui, &folder);
        if commit {
            self.commit_settings();
        }
        if photos {
            self.toggle_setting(|s| s.summary_photos = !s.summary_photos);
        }
        if let Some(l) = lang {
            self.summaries.lang = l;
            self.toggle_setting(|s| s.summary_lang = l.code().into());
        }
        if let Some(name) = picked {
            self.toggle_setting(|s| s.summary_model = name);
        }
        match models_button {
            Some(0) => self.browse_for_models_folder(),
            Some(_) => {
                // Made on first open, so there is somewhere to put the files.
                let _ = std::fs::create_dir_all(&folder);
                self.open_or_warn(&folder);
            }
            None => {}
        }
        if server_button.is_some() {
            self.browse_for_server();
        }
    }

    /// The short version of `docs/MODELS.md`, folded away under the section.
    fn how_to_add_a_model(&mut self, ui: &mut Ui, folder: &std::path::Path) {
        let p = self.palette;
        let mut open = false;
        egui::CollapsingHeader::new(text("How to add a model", &p))
            .id_salt("summary-how-to")
            .show(ui, |ui| {
                for (i, step) in ADD_A_MODEL.iter().enumerate() {
                    ui.add(egui::Label::new(meta(format!("{}. {step}", i + 1), &p)).wrap());
                    ui.add_space(space::TIGHT);
                }
                ui.horizontal(|ui| {
                    for (label, url) in [
                        ("llama.cpp releases", LLAMA_RELEASES),
                        ("Hugging Face", HUGGING_FACE),
                    ] {
                        // A link is the accent's one exception, as on the
                        // sign-in dialog.
                        let label = egui::RichText::new(label)
                            .font(tgx_ui::fonts::sans(window::READING))
                            .color(p.accent);
                        if action(ui, label, true).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        }
                    }
                    open = button(ui, "Open the models folder", true, false, &p).clicked();
                });
            });
        if open {
            let _ = std::fs::create_dir_all(folder);
            self.open_or_warn(folder);
        }
    }

    /// Browse for the models folder.
    fn browse_for_models_folder(&mut self) {
        let Some(dir) = rfd::FileDialog::new()
            .set_title("Choose the models folder")
            .set_directory(self.settings.summary_models_dir())
            .pick_folder()
        else {
            return;
        };
        // The other fields first, as Browse for the destination does.
        self.form.collect(&mut self.settings);
        self.settings.summary_models = dir.to_string_lossy().into_owned();
        self.form.sync(&self.settings);
        self.commit_settings();
    }

    /// Browse for a `llama-server.exe` outside the models folder.
    fn browse_for_server(&mut self) {
        let Some(file) = rfd::FileDialog::new()
            .set_title("Choose llama-server.exe")
            .add_filter("Program", &["exe"])
            .pick_file()
        else {
            return;
        };
        self.form.collect(&mut self.settings);
        self.settings.summary_server = file.to_string_lossy().into_owned();
        self.form.sync(&self.settings);
        self.commit_settings();
    }

    /// **The appearance is a setting, and this is where settings live.**
    fn appearance_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        section(ui, "Appearance", &p);
        // The stored name decides which value is shown, so an edited settings
        // file naming neither shows Dark — which is what `Palette::named` will
        // actually have produced.
        if let Some(i) = cycle(
            ui,
            "theme",
            "Theme",
            &["Light", "Dark", "System"],
            theme_index(&self.settings.theme),
            true,
            "System follows Windows' own light or dark mode.",
            &p,
        ) {
            self.set_theme(THEMES[i]);
        }
        // Egui's own zoom, which `track_window` copies into the settings each
        // frame — so setting it here is all a change takes.
        let zoom = ui.ctx().zoom_factor();
        let step = option(
            ui,
            "zoom",
            Some(
                "Click to zoom in, right-click to zoom out. Ctrl+plus, Ctrl+minus and Ctrl+0 \
                 do the same from anywhere. Kept between launches.",
            ),
            &p,
            |ui, width| {
                let hit = option_box(ui, width, &format!("Zoom: {:.0}%", zoom * 100.0), true, &p);
                if hit.clicked() {
                    Some(ZOOM_STEP)
                } else if hit.secondary_clicked() {
                    Some(-ZOOM_STEP)
                } else {
                    None
                }
            },
        );
        if let Some(step) = step {
            ui.ctx().set_zoom_factor(next_zoom(zoom, step));
        }
    }

    /// The dial and the folder checks: rarely touched, and each one a reason
    /// to read its `?` first.
    fn advanced_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        section(ui, "Advanced", &p);
        if self.number(
            ui,
            "downloads",
            "Parallel downloads",
            "4 to 6 is the balance. Higher, Telegram rate-limits instead.",
            true,
            |f| &mut f.downloads,
        ) {
            self.commit_settings();
        }
        // Read-only: what is found goes to the log. The fixes are the CLI's
        // (`doctor --fix`), because each moves or deletes files and a button
        // that does that one click after reporting is a button pressed twice
        // by accident.
        let (checking, deduping) = (self.checking, self.deduping);
        let check = action_option(
            ui,
            "check_folder",
            "Check the export folder",
            !checking,
            "Lists leftovers and duplicate databases in the log. Changes nothing.",
            &p,
        );
        if check {
            self.check_folder();
        }
        let dupes = action_option(
            ui,
            "find_dupes",
            "Find duplicate media",
            !deduping,
            "Lists files stored more than once, largest first, in the log. Deletes nothing.",
            &p,
        );
        if dupes {
            self.find_dupes();
        }

        // **Where to look when something goes wrong**, and the way to hand it
        // to someone else without handing them the account.
        let (data, log) = option(
            ui,
            "log_and_data",
            Some(
                "The data folder holds the sign-in, the settings and tgx.log. The log is \
                 every run's account of itself, and never holds message text.",
            ),
            &p,
            |ui, width| {
                let half = (width - ui.spacing().item_spacing.x) / 2.0;
                let data = option_box(ui, half, "Open data folder", true, &p).clicked();
                let log = option_box(ui, half, "Open log", true, &p).clicked();
                (data, log)
            },
        );
        let copy = action_option(
            ui,
            "copy_diagnostics",
            "Copy diagnostics",
            true,
            "For a bug report: version, Windows, and settings without credentials. Paths \
             can include your Windows user name — look it over before posting it.",
            &p,
        );
        if log {
            self.open_or_warn(&tgx_tg::logging::log_file());
        }
        if data {
            self.open_or_warn(&tgx_tg::config::data_dir());
        }
        if copy {
            ui.ctx()
                .copy_text(crate::diagnostics::report(&self.settings));
            self.status = "Diagnostics copied — no credentials in them".into();
        }

        let label = if self.reset_armed {
            "Press again to reset"
        } else {
            "Reset settings"
        };
        let reset = action_option(
            ui,
            "reset",
            label,
            !self.exporting,
            "Everything here back to its default, on a second press. Keeps the sign-in, \
             the export folder and the models folder.",
            &p,
        );
        if reset {
            self.reset_settings();
        }

        ui.add_space(space::STEP);
        ui.vertical_centered(|ui| {
            ui.add(
                egui::Label::new(meta(
                    "Ctrl+1–5 views · Ctrl+F filter · Ctrl+A tick all shown · Ctrl+E export · \
                     F5 reload · Esc clear filter",
                    &p,
                ))
                .wrap(),
            );
            ui.label(meta(
                format!("TelegramTools {}", env!("CARGO_PKG_VERSION")),
                &p,
            ));
        });
    }

    fn open_or_warn(&mut self, path: &std::path::Path) {
        if let Err(e) = crate::actions::open_path(path) {
            self.journal.warn(e);
            self.log_copied = false;
        }
    }

    /// Reset settings, on the second press: every setting to its default
    /// **except** what is not a preference — the api_id, api_hash and phone
    /// the sign-in needs, the destination (Analysis and every database
    /// path hang off it), the models folder (gigabytes the account holder put
    /// somewhere on purpose), and the saved selections, which are work.
    pub(super) fn reset_settings(&mut self) {
        if self.exporting {
            return;
        }
        if !self.reset_armed {
            self.reset_armed = true;
            return;
        }
        self.reset_armed = false;
        let old = std::mem::take(&mut self.settings);
        self.settings = tgx_tg::config::Settings {
            api_id: old.api_id,
            api_hash: old.api_hash,
            phone: old.phone,
            output_dir: old.output_dir,
            summary_models: old.summary_models,
            saved_selections: old.saved_selections,
            window: old.window,
            window_maximized: old.window_maximized,
            ..Default::default()
        };
        // The fields first: `commit_settings` reads them back, and the old
        // text in them would undo the reset.
        self.form.sync(&self.settings);
        self.view.sort = list::SortMode::from_key(&self.settings.sort_mode);
        self.view.grouped = self.settings.group_by_type;
        self.view.folded.clear();
        self.palette = Palette::named(&self.settings.theme);
        self.theme_stale = true;
        self.zoom_stale = true;
        self.rebuild_rows();
        self.arm_schedule(std::time::Instant::now());
        self.commit_settings();
        self.status = "Settings reset to their defaults".into();
        self.journal.push(self.status.clone());
    }

    /// **From and Until: a folder export of only those days.** Folder mode
    /// only, so drawn only there: a database judges deletions by what a whole
    /// walk saw. A date that does not parse is said under the fields at once,
    /// and the run refuses it rather than exporting the whole chat.
    fn date_rows(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let mut commit = false;
        commit |= self.entry(
            ui,
            "date_from",
            "From",
            "Only messages from this day on, as 2026-01-31. Blank is the start of the chat.",
            true,
            DATE_W,
            "YYYY-MM-DD",
            |f| &mut f.date_from,
        );
        commit |= self.entry(
            ui,
            "date_until",
            "Until",
            "Up to and including this day, as 2026-01-31. Blank is up to now.",
            true,
            DATE_W,
            "YYYY-MM-DD",
            |f| &mut f.date_until,
        );
        if commit {
            self.commit_settings();
        }
        if let Err(e) = self.settings.date_range() {
            ui.vertical_centered(|ui| ui.label(text(e, &p)));
        }
    }

    /// A number: its name in the box, a narrow field on the right. Returns
    /// whether the field just lost focus, which is when the value is a
    /// decision.
    fn number(
        &mut self,
        ui: &mut Ui,
        key: &str,
        label: &str,
        why: &str,
        enabled: bool,
        which: impl Fn(&mut crate::settings_form::SettingsForm) -> &mut String,
    ) -> bool {
        self.entry(ui, key, label, why, enabled, NUMBER_W, "", which)
    }

    /// A short value typed into the box: [`Shell::number`], and the dates.
    #[allow(clippy::too_many_arguments)]
    fn entry(
        &mut self,
        ui: &mut Ui,
        key: &str,
        label: &str,
        why: &str,
        enabled: bool,
        width: f32,
        hint: &str,
        which: impl Fn(&mut crate::settings_form::SettingsForm) -> &mut String,
    ) -> bool {
        let p = self.palette;
        let form = &mut self.form;
        option(ui, key, Some(why), &p, |ui, room| {
            field_box(ui, room, label, enabled, &p, |ui| {
                ui.add_enabled(
                    enabled,
                    field(which(form), &p).hint_text(hint).desired_width(width),
                )
                .lost_focus()
            })
        })
    }

    /// Pick a destination folder.
    ///
    /// The path is free text and may be unwritable, disconnected or invalid, so
    /// a picker is not a nicety — it is the one way to get a path that exists.
    /// `rfd`'s blocking picker holds this thread until the user answers, which
    /// removes the window in which the fields could be edited underneath a
    /// pending write-back.
    fn browse_for_output_dir(&mut self) {
        let Some(dir) = rfd::FileDialog::new()
            .set_title("Choose export folder")
            .set_directory(&self.settings.output_dir)
            .pick_folder()
        else {
            return;
        };
        // **Read the other fields back first.** `commit_settings` rewrites all
        // of them from `settings`, so anything typed and not yet blurred —
        // `2000` in Messages per page — would be replaced by the stored value
        // with nothing said.
        self.form.collect(&mut self.settings);
        self.settings.output_dir = dir.to_string_lossy().into_owned();
        self.form.output_dir = self.settings.output_dir.clone();
        self.commit_settings();
    }
}

/// What a switch does to the settings when pressed.
type Flip = fn(&mut tgx_tg::config::Settings);

/// How wide the column is at most. Wide enough for the longest `Name: Value`
/// and a path, narrow enough that the eye runs straight down it.
const COLUMN_W: f32 = 560.0;
/// How tall every box is, the `?` included — it is square.
const OPTION_H: f32 = 34.0;
/// The gap between two boxes, across and down.
const OPTION_GAP: f32 = 6.0;
/// How wide a button beside a path field is.
const BUTTON_W: f32 = 86.0;
/// How wide a `?`'s explanation is.
const HELP_W: f32 = 340.0;
/// How wide a number field is. Every value in this panel is at most five
/// digits, and a field sized for a sentence invites one.
const NUMBER_W: f32 = 72.0;
/// How wide a date field is: `2026-01-31` and a little air.
const DATE_W: f32 = 96.0;
/// One click on Zoom, and how far it may go either way.
const ZOOM_STEP: f32 = 0.1;
const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;

/// Settings › Summaries › How to add a model. `docs/MODELS.md` is the long
/// version; keep the two saying the same.
const ADD_A_MODEL: [&str; 5] = [
    "Once: get llama.cpp from its releases page. For an NVIDIA card take the \
     llama-…-bin-win-cuda-…-x64.zip and the cudart-llama-bin-win-cuda-…-x64.zip of the \
     same CUDA version (for AMD or Intel, the bin-win-vulkan-x64.zip), and unzip both into \
     one folder inside the models folder.",
    "On Hugging Face, search for the model's name and GGUF — \"gemma 4 26B A4B GGUF\" — \
     and open a repository that lists .gguf files under Files and versions.",
    "Download one .gguf. Pick the size by your graphics card: the file a few GB smaller \
     than its memory, for the context. Q4_K_M is the usual choice. Leave the mmproj files: \
     they are for pictures. A model split into …-00001-of-00003.gguf needs every piece, in \
     one folder.",
    "Put it in the models folder, or a folder of its own inside it, and choose it under \
     Model above. Some models ask you to sign in and accept a licence on their page first.",
    "Text in photos, if you want it: search \"PaddleOCR-VL GGUF\" and put the model, its \
     mmproj and chat_template.jinja, if there is one, in a folder of their own inside the \
     models folder.",
];
const LLAMA_RELEASES: &str = "https://github.com/ggml-org/llama.cpp/releases";
const HUGGING_FACE: &str = "https://huggingface.co/models?library=gguf";

/// How far a `?` hangs past the column's right edge: its gap and its square.
const HANG: f32 = OPTION_GAP + OPTION_H;

/// The column, centred in the panel, at most [`COLUMN_W`] wide.
///
/// **The boxes are what is centred; the `?`s hang outside.** The column was
/// the boxes and the `?`s together, so the boxes sat half a square left of
/// the middle and the page never looked centred. Now the column is the boxes
/// alone, and the room either side keeps space for the `?`s, so the
/// headings and the centred lines are centred on the boxes too.
fn column(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let room = ui.available_width();
    let width = COLUMN_W
        .min(room - 2.0 * (GUTTER + HANG))
        .max(OPTION_H * 6.0);
    let left = ((room - width) / 2.0).max(0.0);
    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
        ui.add_space(left);
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = egui::vec2(OPTION_GAP, OPTION_GAP);
            add(ui);
        });
    });
}

/// A section heading, centred over its boxes. **Space above it, and no line
/// under it**: a gap four times the one between two boxes is not ambiguous
/// about where a group starts.
fn section(ui: &mut Ui, heading: &str, p: &Palette) {
    ui.add_space(space::BREAK);
    ui.vertical_centered(|ui| ui.label(eyebrow(heading, p)));
    ui.add_space(space::TIGHT);
}

/// One line of the column: `control` gets the column's whole width and
/// returns what was pressed; the `?` goes just past the column's right edge,
/// outside it (see [`column`]). `why: None` has no `?`.
fn option<R>(
    ui: &mut Ui,
    key: &str,
    why: Option<&str>,
    p: &Palette,
    control: impl FnOnce(&mut Ui, f32) -> R,
) -> R {
    ui.horizontal(|ui| {
        let width = ui.available_width();
        let origin = ui.cursor().min;
        let out = control(ui, width);
        if let Some(why) = why {
            // A child placed by hand, not a widget in the line: the line is
            // the column's width, and the `?` is not in it.
            let square = egui::Rect::from_min_size(
                egui::pos2(origin.x + width + OPTION_GAP, origin.y),
                egui::Vec2::splat(OPTION_H),
            );
            let mut beside = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(square)
                    .layout(Layout::left_to_right(Align::Min)),
            );
            help_button(&mut beside, key, why, p);
        }
        out
    })
    .inner
}

/// A setting with a fixed set of values, as `Name: Value`. A click moves to
/// the next value and a right-click to the one before; returns the new index
/// only when it changed, so a stray click writes no `settings.json`.
#[allow(clippy::too_many_arguments)]
fn cycle(
    ui: &mut Ui,
    key: &str,
    label: &str,
    values: &[&str],
    at: usize,
    enabled: bool,
    why: &str,
    p: &Palette,
) -> Option<usize> {
    let n = values.len();
    let at = at.min(n - 1);
    option(ui, key, Some(why), p, |ui, width| {
        let hit = option_box(ui, width, &format!("{label}: {}", values[at]), enabled, p);
        if hit.clicked() {
            Some((at + 1) % n)
        } else if hit.secondary_clicked() {
            Some((at + n - 1) % n)
        } else {
            None
        }
    })
    .filter(|i| *i != at)
}

/// An on/off setting: [`cycle`] over Off and On. Returns whether it was
/// pressed.
fn switch(
    ui: &mut Ui,
    key: &str,
    label: &str,
    on: bool,
    enabled: bool,
    why: &str,
    p: &Palette,
) -> bool {
    cycle(
        ui,
        key,
        label,
        &["Off", "On"],
        usize::from(on),
        enabled,
        why,
        p,
    )
    .is_some()
}

/// A box that does something rather than holding a value.
fn action_option(
    ui: &mut Ui,
    key: &str,
    label: &str,
    enabled: bool,
    why: &str,
    p: &Palette,
) -> bool {
    option(ui, key, Some(why), p, |ui, width| {
        option_box(ui, width, label, enabled, p).clicked()
    })
}

/// A path: its name over the line, then the field, its buttons and the `?`.
/// Returns whether the field just lost focus, and which button was pressed.
#[allow(clippy::too_many_arguments)]
fn path_option(
    ui: &mut Ui,
    key: &str,
    label: &str,
    why: &str,
    buffer: &mut String,
    hint: &str,
    buttons: &[&str],
    p: &Palette,
) -> (bool, Option<usize>) {
    ui.label(meta(label, p));
    option(ui, key, Some(why), p, |ui, width| {
        let gap = ui.spacing().item_spacing.x;
        let room = (width - buttons.len() as f32 * (BUTTON_W + gap)).max(NUMBER_W);
        let entry = ui.add_sized(egui::vec2(room, OPTION_H), field(buffer, p).hint_text(hint));
        let commit = entry.lost_focus();
        if !buffer.is_empty() {
            entry.on_hover_text(buffer.as_str());
        }
        let mut pressed = None;
        for (i, b) in buttons.iter().enumerate() {
            if option_box(ui, BUTTON_W, b, true, p).clicked() {
                pressed = Some(i);
            }
        }
        (commit, pressed)
    })
}

/// The box every setting is: `width` wide, [`OPTION_H`] tall, its words in
/// the middle. **Painted, not an `egui::Button`**, because a button places
/// its text by the enclosing layout's alignment, and a box whose words sit
/// at its left edge in one place and its middle in another is two designs.
///
/// Hover brings the border up to the ink colour — the box is the control, so
/// it says so under the pointer. Disabled keeps the fill, loses the border's
/// brightness and greys the words: still plainly a setting, plainly not
/// available.
fn option_box(ui: &mut Ui, width: f32, label: &str, enabled: bool, p: &Palette) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, OPTION_H), sense);
    paint_box(ui, rect, enabled && response.hovered(), enabled, p);
    ui.painter_at(rect.shrink(1.0)).text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        tgx_ui::fonts::sans(window::READING),
        if enabled { p.fg } else { p.muted },
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The same box with its name on the left and a field on the right, for a
/// value that is typed rather than chosen.
fn field_box<R>(
    ui: &mut Ui,
    width: f32,
    label: &str,
    enabled: bool,
    p: &Palette,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, OPTION_H), Sense::hover());
    paint_box(ui, rect, false, enabled, p);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(space::TIGHT, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    let out = add(&mut inner);
    inner.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.label(if enabled {
            text(label, p)
        } else {
            meta(label, p)
        })
    });
    out
}

fn paint_box(ui: &Ui, rect: egui::Rect, hovered: bool, enabled: bool, p: &Palette) {
    let edge = match (enabled, hovered) {
        (true, true) => p.fg,
        (true, false) => p.hairline,
        (false, _) => p.rule,
    };
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::ZERO, p.surface);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::ZERO,
        Stroke::new(1.0_f32, edge),
        egui::StrokeKind::Inside,
    );
}

/// The square `?` beside a box, and the explanation it opens.
///
/// **A click, not a hover**: a hover is not discoverable, and on a touchpad it
/// means holding still. Opened below and to the left, since the `?` is at the
/// column's right edge and the explanation is about the box beside it.
///
/// `key` is the setting's own name and **is** the popup's identity. It must
/// not be left to the auto-id: the panel shows a different set of boxes in
/// each format, so an id derived from position would hand an explanation to a
/// different setting the moment the format changed.
fn help_button(ui: &mut Ui, key: &str, why: &str, p: &Palette) {
    let hit = option_box(ui, OPTION_H, "?", true, p);
    let _ = egui::Popup::from_toggle_button_response(&hit)
        .id(egui::Id::new(("settings-help", key)))
        .align(egui::RectAlign::BOTTOM_END)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(HELP_W)
        .gap(4.0)
        // The window's own frame, not egui's: `Frame::popup` is rounded and
        // shadowed, which `theme::install` takes off everything else.
        .frame(
            egui::Frame::NONE
                .fill(p.bg)
                .stroke(Stroke::new(1.0_f32, p.hairline))
                .inner_margin(Margin::same(12)),
        )
        .show(|ui| {
            ui.set_max_width(HELP_W);
            ui.add(egui::Label::new(text(why, p)).wrap());
        });
}

fn on_off(on: bool) -> &'static str {
    if on {
        "On"
    } else {
        "Off"
    }
}

/// The zoom one step on, rounded to a whole percent so that ten clicks do
/// not drift to 109.99999%, and held to [`ZOOM_RANGE`].
fn next_zoom(zoom: f32, step: f32) -> f32 {
    ((zoom + step) * 100.0)
        .round()
        .clamp(*ZOOM_RANGE.start() * 100.0, *ZOOM_RANGE.end() * 100.0)
        / 100.0
}

/// Which of [`THEMES`] a stored name is. Anything unknown is Dark, which is
/// what `Palette::named` will have put on screen for it.
fn theme_index(name: &str) -> usize {
    THEMES.iter().position(|t| *t == name).unwrap_or(1)
}

fn extra(s: &tgx_tg::config::Settings, key: &str) -> bool {
    match key {
        "full_reactions" => s.full_reactions,
        "chat_metadata" => s.chat_metadata,
        "invite_links" => s.invite_links,
        "refresh_polls" => s.refresh_polls,
        "scheduled_messages" => s.scheduled_messages,
        "member_roster" => s.member_roster,
        _ => false,
    }
}

fn set_extra(s: &mut tgx_tg::config::Settings, key: &str, on: bool) {
    match key {
        "full_reactions" => s.full_reactions = on,
        "chat_metadata" => s.chat_metadata = on,
        "invite_links" => s.invite_links = on,
        "refresh_polls" => s.refresh_polls = on,
        "scheduled_messages" => s.scheduled_messages = on,
        "member_roster" => s.member_roster = on,
        _ => {}
    }
}

// -- the long explanations -------------------------------------------------
//
// Where a choice has consequences a sentence cannot hold.

/// What the format really chooses, which is more than a file format.
const MODE_HELP: &str = "Folder reproduces Telegram Desktop's own export: a folder per chat, \
     browsable HTML and result.json, read from the beginning of the chat every \
     run. Database gives each chat its own <chat>.sqlite that is added to — a \
     re-run syncs what is new instead of exporting everything again, and it \
     keeps messages that were later deleted on Telegram, along with every \
     earlier version of an edited one. The two are exclusive on purpose: \
     Folder re-reads the whole chat anyway, so running both would cost exactly \
     what Folder costs and be incremental in name only.";

const REREAD_ALL_HELP: &str =
    "Read the whole chat again instead of the newest few hundred. Slow on a large \
     chat, and the only way to catch an edit or a deletion older than Messages to \
     re-read — or to pick up media after raising the size limit.";

/// The [`tgx_tg::config::Settings::profile_photos`] values, in the order the
/// box steps through them.
const USERPICS: [&str; 3] = [
    tgx_tg::config::USERPICS_OFF,
    tgx_tg::config::USERPICS_CURRENT,
    tgx_tg::config::USERPICS_HISTORY,
];

const PHOTOS_HELP: &str = "Keep the profile picture of everyone in the chat — its members and \
     anyone who posted — in the chat's database, and show them when reading. Current keeps the \
     picture each person wears now: one small download per person, and none on a sync that finds \
     it unchanged; a new one is added and the old one kept. History keeps every picture they \
     still have up, which costs one more request per person whose picture changed. Desktop's own \
     export has no pictures, so a folder export never gets them. Someone who hides their \
     picture from you is skipped.";

#[cfg(test)]
mod tests {
    use super::*;
    use tgx_tg::config::{Settings, MEDIA_KINDS};

    #[test]
    fn every_media_category_the_planner_knows_has_a_label() {
        // A key here that `plan.rs` does not compare against is a box that
        // switches nothing off; a key there with no box is a category the user
        // cannot reach.
        let labelled: Vec<&str> = MEDIA_LABELS.iter().map(|(k, _)| *k).collect();
        for key in MEDIA_KINDS {
            assert!(labelled.contains(&key), "{key} has no box");
        }
        assert_eq!(labelled.len(), MEDIA_KINDS.len());
    }

    #[test]
    fn every_extra_reads_and_writes_its_own_field() {
        // A copy-paste in `set_extra` would silently point two boxes at one
        // field, and the panel would look fine while switching the wrong thing.
        for (key, _, _) in EXTRAS {
            let mut s = Settings::default();
            set_extra(&mut s, key, false);
            assert!(!extra(&s, key), "{key} did not clear");
            for (other, _, _) in EXTRAS {
                if other != key {
                    assert!(extra(&s, other), "clearing {key} also cleared {other}");
                }
            }
            set_extra(&mut s, key, true);
            assert!(extra(&s, key), "{key} did not set");
        }
    }

    #[test]
    fn an_unknown_extra_is_ignored_rather_than_matching_the_first_arm() {
        let mut s = Settings::default();
        set_extra(&mut s, "nonsense", false);
        assert_eq!(s, Settings::default());
        assert!(!extra(&s, "nonsense"));
    }

    #[test]
    fn every_explanation_is_a_sentence_and_no_two_are_the_same() {
        // What a `?` says is the only place the panel explains a setting, so
        // it has to be there, say something, and not be a copy of its
        // neighbour — the shape a copy-paste takes here.
        let mut lines: Vec<&str> = EXTRAS.iter().map(|(_, _, why)| *why).collect();
        lines.extend([MODE_HELP, REREAD_ALL_HELP, PHOTOS_HELP]);
        for why in &lines {
            assert!(why.len() > 20, "too short to say anything: {why}");
            assert!(why.trim_end().ends_with('.'), "not a sentence: {why}");
        }
        let mut unique = lines.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            lines.len(),
            "two settings share an explanation"
        );
    }

    #[test]
    fn the_theme_box_shows_the_palette_that_is_actually_on_screen() {
        // `Palette::named` treats anything that is not `light` as dark, so a
        // settings file naming something else must read Dark — a box saying
        // Light over a dark window is worse than no box.
        for (name, expect) in [("light", 0), ("dark", 1), ("chartreuse", 1)] {
            assert_eq!(theme_index(name), expect, "{name}");
            assert_eq!(
                Palette::named(THEMES[theme_index(name)]),
                Palette::named(name)
            );
        }
        assert_eq!(theme_index("system"), 2);
    }

    #[test]
    fn zoom_steps_land_on_whole_percents_and_stop_at_the_ends() {
        let mut z = 1.0;
        for _ in 0..10 {
            z = next_zoom(z, ZOOM_STEP);
        }
        assert_eq!(z, 2.0);
        assert_eq!(next_zoom(3.0, ZOOM_STEP), 3.0);
        assert_eq!(next_zoom(0.5, -ZOOM_STEP), 0.5);
    }
}
