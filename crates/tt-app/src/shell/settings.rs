//! The settings panel — the site's readout tables, made editable.
//!
//! Six sections, grouped by the question each answers (TELEGRAMTOOLS.md
//! §4.2): **Where exports go** and **What an export contains** down the left;
//! **Media**, **Reports**, **Appearance** and **Advanced** down the right. No
//! group boxes: a section is a letterspaced heading with space above it, and a
//! setting is one [`setting_row`] — its name, one muted line saying what it
//! does, and its control on the right.
//!
//! # What the last two passes got wrong
//!
//! **The explanations were on the page, then behind a `[?]`.** First eight
//! paragraphs of small print interleaved with twenty-five switches, which made
//! the panel unscannable; then every sentence moved behind a mark of its own,
//! which made it quiet and made every switch a question. A setting now carries
//! one line, always visible — what it does, and what it costs when it costs
//! something — and a `[?]` is left only where a choice has consequences one
//! line cannot hold: the export's format, checking the whole history, and
//! profile pictures.
//!
//! **Classic and Database were dimmed, not switched.** The mode was a pair of
//! tick boxes that unticked each other, and the losing mode's controls stayed on
//! screen greyed out. It is a [`segmented`] control, now named **Folder** and
//! **Database** after what each leaves on disk, and the rows below it are
//! *replaced*: the panel shows the format you are in and nothing about the one
//! you are not.
//!
//! **The rhythm did the opposite of grouping.** Rows sit on `item_spacing`
//! plus `space::TIGHT` and sections on `space::BREAK`, which is the
//! four-to-one this file always claimed.
//!
//! **There is no "Chats at once".** This engine exports one chat at a time, and
//! a control that is enabled and does nothing teaches that the interface is
//! unreliable. An old `settings.json` naming it still loads, because unknown
//! keys are dropped per field.

use super::{list, Shell, THEMES};
use eframe::egui::{self, Ui};
use tgx_ui::components::{
    action, block, button, eyebrow, field, figure, help, meta, row, segmented, setting_row, text,
    tick_box,
};
use tgx_ui::tokens::{space, Palette};

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

/// The extra requests, each with the line under it.
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
        "Who is in the chat, as participants.json. Capped below.",
    ),
];

impl Shell {
    pub(super) fn settings_panel(&mut self, ui: &mut Ui) {
        // No `SETTINGS` heading: the view bar above already says so, and a view
        // does not need to introduce itself.
        //
        // **Two columns, because there is a window's width to use.** Left is
        // the export — where it goes and what is in it. Right is everything
        // around it. Balanced by height rather than by section count.
        egui::ScrollArea::vertical()
            .id_salt("settings-body")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.unreadable_settings(ui);
                ui.columns(2, |cols| {
                    self.destination_section(&mut cols[0]);
                    self.contents_section(&mut cols[0]);

                    self.media_section(&mut cols[1]);
                    self.reports_section(&mut cols[1]);
                    self.summaries_section(&mut cols[1]);
                    self.appearance_section(&mut cols[1]);
                    self.advanced_section(&mut cols[1]);
                });
                ui.add_space(space::BREAK);
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
        block(ui, |ui| {
            ui.label(text(
                format!(
                    "Your settings file was not used: {why}. Changes here are not saved until \
                     you choose to replace it."
                ),
                &p,
            ))
        });
        let replace = row(ui, |ui| {
            button(ui, "Replace it with these settings", true, false, &p).clicked()
        });
        if replace {
            tgx_tg::config::allow_settings_overwrite();
            self.commit_settings();
            self.status = "Settings saved over the unreadable file".into();
        }
        ui.add_space(space::STEP);
    }

    /// **The path shows its start, not its end, and the whole of it is the
    /// tooltip.** A field scrolled to its caret renders the default path as a
    /// drive letter sliced in half, which reads as a typo rather than as a
    /// path. Both are redone on every change — not once at construction —
    /// because Browse writes back afterwards.
    fn destination_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        section(ui, "Where exports go", &p);
        let mut browse = false;
        let mut commit = false;
        let full = self.settings.output_dir.clone();
        // A button: the one way to get a path that certainly exists should
        // not be small print. The field gets a line of its own under the
        // name: beside it, at the window's floor, a path field left the
        // name one letter wide.
        setting_row(
            ui,
            "Folder",
            "Each chat gets a folder here, or a database in a folder of its own.",
            &p,
            |ui| browse = button(ui, "Browse", true, false, &p).clicked(),
        );
        row(ui, |ui| {
            let entry = ui.add(
                field(&mut self.form.output_dir, &p)
                    .hint_text("Where exports go")
                    .desired_width(ui.available_width()),
            );
            // Committing on losing focus rather than on every keystroke: a
            // half-typed path is not a decision.
            commit = entry.lost_focus();
            entry.on_hover_text(&full);
        });
        if commit {
            self.commit_settings();
        }
        if browse {
            self.browse_for_output_dir();
        }
        row(ui, |ui| {
            ui.label(meta(
                format!(
                    "Writing to {}",
                    crate::settings_form::elided_start(&full, 56)
                ),
                &p,
            ))
            .on_hover_text(full.clone());
        });
    }

    /// **Folder or Database — an exclusive choice, drawn as one, and it
    /// changes what is on the page.** The rows under the strip are the chosen
    /// format's alone; then what both formats ask Telegram for.
    ///
    /// Split forum topics stays below the switch because it is the one setting
    /// both formats read — a folder per topic, or the topic recorded per
    /// message.
    fn contents_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let db = self.settings.export_db;
        section(ui, "What an export contains", &p);

        let mut chosen = None;
        setting_row(
            ui,
            "Format",
            if db {
                "One file per chat that each run adds to. Keeps deleted and edited messages."
            } else {
                "Telegram Desktop's own layout: HTML and result.json, re-read in full each run."
            },
            &p,
            |ui| {
                help(ui, "export_db", MODE_HELP, &p);
                chosen = pair(ui, ["Folder", "Database"], usize::from(db), &p);
            },
        );
        if let Some(i) = chosen {
            self.toggle_setting(move |s| s.export_db = i == 1);
        }

        if db {
            if self.number_row(
                ui,
                "Messages to re-read",
                "How far back each sync looks again. 0 is new messages only.",
                true,
                |f| &mut f.reread_window,
            ) {
                self.commit_settings();
            }
            let mut flip = false;
            setting_row(
                ui,
                "Check the whole history",
                "Finds deletions and edits further back. Slow on a large chat.",
                &p,
                |ui| {
                    help(ui, "reread_all", REREAD_ALL_HELP, &p);
                    flip = tick_box(ui, self.settings.reread_all, true, &p).clicked();
                },
            );
            if flip {
                self.toggle_setting(|s| s.reread_all = !s.reread_all);
            }
            if self.number_row(
                ui,
                "Sync every (hours)",
                "Updates every database here on a timer while the window is open. 0 is off.",
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
            let mut picked = None;
            setting_row(
                ui,
                "Profile pictures",
                "Kept in the database and shown when reading. Desktop has none.",
                &p,
                |ui| {
                    help(ui, "profile_photos", PHOTOS_HELP, &p);
                    // Drawn right to left, so the strip is given in reverse
                    // and the answer turned back round.
                    let cells = ["History", "Current", "Off"];
                    picked = segmented(ui, &cells, 2 - at, &p).map(|i| 2 - i);
                },
            );
            if let Some(i) = picked {
                self.toggle_setting(move |s| s.profile_photos = USERPICS[i].into());
            }
        } else {
            for (key, label, why, on) in [
                (
                    "export_html",
                    "HTML pages",
                    "Desktop's browsable pages, line for line.",
                    self.settings.export_html,
                ),
                (
                    "export_json",
                    "result.json",
                    "Every message as data, byte for byte as Desktop writes it.",
                    self.settings.export_json,
                ),
            ] {
                if switch_row(ui, label, why, on, true, &p) {
                    self.toggle_setting(move |s| match key {
                        "export_html" => s.export_html = !s.export_html,
                        _ => s.export_json = !s.export_json,
                    });
                }
            }
            if self.number_row(
                ui,
                "Messages per page",
                "Desktop's default is 1,000. Held between 50 and 20,000.",
                true,
                |f| &mut f.page_size,
            ) {
                self.commit_settings();
            }
            self.date_rows(ui);
        }

        if switch_row(
            ui,
            "Split forum topics",
            "A folder per topic — what Telegram Desktop cannot do.",
            self.settings.split_topics,
            true,
            &p,
        ) {
            self.toggle_setting(|s| s.split_topics = !s.split_topics);
        }
        for (key, label, why) in EXTRAS {
            let on = extra(&self.settings, key);
            if switch_row(ui, label, why, on, true, &p) {
                self.toggle_setting(move |s| set_extra(s, key, !extra(s, key)));
            }
        }
        if self.number_row(
            ui,
            "Member list cap",
            "Stop the member list after this many. 0 is no cap.",
            self.settings.member_roster,
            |f| &mut f.member_limit,
        ) {
            self.commit_settings();
        }
        // Not among the extras: they are requests, and this costs none — it
        // changes which of two names already in hand gets written.
        if switch_row(
            ui,
            "Contacts by @username",
            "Not by the name you saved them under, which only you see.",
            self.settings.own_names,
            true,
            &p,
        ) {
            self.toggle_setting(|s| s.own_names = !s.own_names);
        }
    }

    fn media_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let media_on = self.settings.download_media;
        section(ui, "Media", &p);
        if switch_row(
            ui,
            "Download media",
            "Off, the export still records every file's name, size and type.",
            media_on,
            true,
            &p,
        ) {
            self.toggle_setting(|s| s.download_media = !s.download_media);
        }

        // **Seven switches across, not seven rows down.** They are one
        // question — which categories — and as a column they took as much of
        // the panel as everything else in this section put together. Wrapped,
        // so a narrow window turns them into three rows rather than clipping
        // the last two.
        let mut toggled: Option<&'static str> = None;
        // A block, not a row: `horizontal_wrapped` needs a vertical context to
        // wrap *into*.
        block(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(space::STEP, 6.0);
                for (key, label) in MEDIA_LABELS {
                    let on = self.settings.media_kinds.iter().any(|k| k == key);
                    // **The wrap is asked for by hand.** A wrapping layout
                    // wraps *widgets*; the nested `horizontal` below is a
                    // child `Ui`, which is placed at the cursor whether it fits
                    // or not, so without this the strip ran off the right edge
                    // and widened the whole column with it.
                    if ui.available_size_before_wrap().x < tick_width(ui, label, &p) {
                        ui.end_row();
                    }
                    // Nested, so the gap between a box and its own label is
                    // `TIGHT` while the gap between two categories is `STEP`.
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = space::TIGHT;
                        if tick(ui, label, on, media_on, &p) {
                            toggled = Some(key);
                        }
                    });
                }
            });
        });
        if let Some(key) = toggled {
            self.toggle_setting(move |s| {
                if let Some(at) = s.media_kinds.iter().position(|k| k == key) {
                    s.media_kinds.remove(at);
                } else {
                    s.media_kinds.push(key.to_string());
                }
            });
        }
        row(ui, |ui| {
            ui.label(meta(
                "A category left off is recorded as Desktop records one it skipped.",
                &p,
            ));
        });
        ui.add_space(space::TIGHT);

        if self.number_row(
            ui,
            "Size limit, MB",
            "Larger files are recorded, not downloaded. 0 is no limit.",
            media_on,
            |f| &mut f.size_limit,
        ) {
            self.commit_settings();
        }
        if switch_row(
            ui,
            "Link-preview images",
            "Desktop does not save these; on, later photo numbers shift.",
            self.settings.link_previews,
            media_on,
            &p,
        ) {
            self.toggle_setting(|s| s.link_previews = !s.link_previews);
        }
    }

    /// What the analyser writes, and when. The three run options were the
    /// Library's, for the session only; they are kept with the rest now.
    fn reports_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        section(ui, "Reports", &p);
        let rows: [(&str, &str, bool, Flip); 4] = [
            (
                "Report after each export",
                "Written beside each chat while the next one downloads.",
                self.settings.report_after_export,
                |s| s.report_after_export = !s.report_after_export,
            ),
            (
                "Write the digest",
                "digest.jsonl and EVENTS.md beside the report, for a model to read.",
                self.settings.report_digest,
                |s| s.report_digest = !s.report_digest,
            ),
            (
                "Embed fonts",
                "Looks right on any machine. Off, each report is about 300 KB smaller.",
                self.settings.report_fonts,
                |s| s.report_fonts = !s.report_fonts,
            ),
            (
                "People by @handle",
                "Never by the name this account saved them under.",
                self.settings.report_handles,
                |s| s.report_handles = !s.report_handles,
            ),
        ];
        for (label, why, on, flip) in rows {
            if switch_row(ui, label, why, on, true, &p) {
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
        let (mut open, mut pick_folder, mut server, mut commit) = (false, false, false, false);
        let folder = self.settings.summary_models_dir();
        self.summaries.look_for_models(&folder, ui.ctx());
        // Only a look at *this* folder answers for it; until one arrives the
        // rows say so rather than "none in the folder".
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

        setting_row(
            ui,
            "Models folder",
            "The models and llama-server.exe, each in it or in a folder of its own inside it.",
            &p,
            |ui| {
                pick_folder = button(ui, "Browse", true, false, &p).clicked();
                open = button(ui, "Open", true, false, &p).clicked();
            },
        );
        row(ui, |ui| {
            commit |= ui
                .add(
                    field(&mut self.form.summary_models, &p)
                        .hint_text("Where the models are")
                        .desired_width(ui.available_width()),
                )
                .lost_focus();
        });

        let chosen = super::summaries::chosen_model(&self.settings, &models);
        let about_model = if looking {
            "Looking in the folder…".to_string()
        } else if models.is_empty() {
            "None in the folder yet: see How to add a model, below.".to_string()
        } else {
            format!(
                "{} in the folder. The prompts are written for Gemma.",
                match models.len() {
                    1 => "One model".to_string(),
                    n => format!("{n} models"),
                }
            )
        };
        let mut picked: Option<String> = None;
        setting_row(ui, "Model", &about_model, &p, |ui| {
            let shown = chosen
                .as_deref()
                .map(&in_folder)
                .unwrap_or_else(|| "None".into());
            egui::ComboBox::from_id_salt("summary-model")
                .selected_text(shown)
                .width(MODEL_W)
                .show_ui(ui, |ui| {
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
        });

        let about_server = if !self.settings.summary_server.is_empty() {
            "Chosen below; the models folder's is not used.".to_string()
        } else if let Some(exe) = &found_server {
            format!("{}, in the models folder.", in_folder(exe))
        } else if looking {
            "Looking in the folder…".to_string()
        } else {
            "Not in the models folder: see How to add a model, below, or choose one.".to_string()
        };
        setting_row(ui, "Model server", &about_server, &p, |ui| {
            server = button(ui, "Browse", true, false, &p).clicked();
        });
        row(ui, |ui| {
            commit |= ui
                .add(
                    field(&mut self.form.summary_server, &p)
                        .hint_text("Blank: llama-server.exe in the models folder")
                        .desired_width(ui.available_width()),
                )
                .lost_focus();
        });
        commit |= self.number_row(
            ui,
            "Port",
            "On 127.0.0.1; a Gemma already serving there is used as it is.",
            true,
            |f| &mut f.summary_port,
        );
        setting_row(
            ui,
            "Server options",
            "For a server started from here. Blank suits Gemma 4 26B on a 16 GB card.",
            &p,
            |_| {},
        );
        row(ui, |ui| {
            commit |= ui
                .add(
                    field(&mut self.form.summary_options, &p)
                        .hint_text(tga_run::tga_sum::server::DEFAULT_OPTIONS)
                        .desired_width(ui.available_width()),
                )
                .lost_focus();
        });
        commit |= self.number_row(
            ui,
            "Unload after",
            "Minutes after you leave Summaries before a server started here is stopped.",
            true,
            |f| &mut f.summary_idle,
        );
        let photos = switch_row(
            ui,
            "Read text in photos",
            "PaddleOCR-VL, if it is in the models folder, reads the days' photos first, \
             then makes way for the model.",
            self.settings.summary_photos,
            true,
            &p,
        );
        self.how_to_add_a_model(ui, &folder);
        if commit {
            self.commit_settings();
        }
        if photos {
            self.toggle_setting(|s| s.summary_photos = !s.summary_photos);
        }
        if let Some(name) = picked {
            self.toggle_setting(|s| s.summary_model = name);
        }
        if open {
            // Made on first open, so there is somewhere to put the files.
            let _ = std::fs::create_dir_all(&folder);
            self.open_or_warn(&folder);
        }
        if pick_folder {
            self.browse_for_models_folder();
        }
        if server {
            self.browse_for_server();
        }
    }

    /// The short version of `docs/MODELS.md`, folded away under the section.
    fn how_to_add_a_model(&mut self, ui: &mut Ui, folder: &std::path::Path) {
        let p = self.palette;
        let mut open = false;
        block(ui, |ui| {
            egui::CollapsingHeader::new(text("How to add a model", &p))
                .id_salt("summary-how-to")
                .show(ui, |ui| {
                    for (i, step) in ADD_A_MODEL.iter().enumerate() {
                        ui.label(meta(format!("{}. {step}", i + 1), &p));
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
                                .font(tgx_ui::fonts::sans(tgx_ui::tokens::window::READING))
                                .color(p.accent);
                            if action(ui, label, true).clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                            }
                        }
                        open = button(ui, "Open the models folder", true, false, &p).clicked();
                    });
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
        // The stored name decides which cell is filled, so an edited settings
        // file naming neither shows Dark — which is what `Palette::named` will
        // actually have produced.
        let at = theme_index(&self.settings.theme);
        let mut chosen = None;
        setting_row(
            ui,
            "Theme",
            "System follows Windows' own light or dark mode.",
            &p,
            |ui| {
                // Drawn right to left, so given in reverse and turned back.
                chosen = segmented(ui, &["System", "Dark", "Light"], 2 - at, &p).map(|i| 2 - i);
            },
        );
        if let Some(i) = chosen {
            self.set_theme(THEMES[i]);
        }
        setting_row(
            ui,
            "Zoom",
            "Ctrl+plus and Ctrl+minus; Ctrl+0 is 100%. Kept between launches.",
            &p,
            |ui| {
                ui.label(figure(
                    format!("{:.0}%", ui.ctx().zoom_factor() * 100.0),
                    &p,
                ));
            },
        );
    }

    /// The dial and the two folder checks: rarely touched, and each one a
    /// reason to read its line first.
    fn advanced_section(&mut self, ui: &mut Ui) {
        let p = self.palette;
        section(ui, "Advanced", &p);
        if self.number_row(
            ui,
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
        let mut check = false;
        setting_row(
            ui,
            "Check the folder",
            "Lists leftovers and duplicate databases in the log. Changes nothing.",
            &p,
            |ui| check = button(ui, "Check", !checking, false, &p).clicked(),
        );
        if check {
            self.check_folder();
        }
        let mut dupes = false;
        setting_row(
            ui,
            "Duplicate media",
            "Lists files stored more than once, largest first. Deletes nothing.",
            &p,
            |ui| dupes = button(ui, "Find", !deduping, false, &p).clicked(),
        );
        if dupes {
            self.find_dupes();
        }

        // **Where to look when something goes wrong**, and the way to hand it
        // to someone else without handing them the account.
        let (mut log, mut data, mut copy) = (false, false, false);
        setting_row(
            ui,
            "Log and data folder",
            "tgx.log holds every run's account of itself; never message text.",
            &p,
            |ui| {
                data = button(ui, "Data folder", true, false, &p).clicked();
                log = button(ui, "Log", true, false, &p).clicked();
            },
        );
        setting_row(
            ui,
            "Copy diagnostics",
            "For a bug report: version, Windows, and settings without credentials.",
            &p,
            |ui| {
                copy = button(ui, "Copy", true, false, &p)
                    .on_hover_text(
                        "Paths can include your Windows user name — look it over before \
                         posting it.",
                    )
                    .clicked();
            },
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

        let mut reset = false;
        let label = if self.reset_armed {
            "Press again"
        } else {
            "Reset"
        };
        setting_row(
            ui,
            "Reset settings",
            "Everything here back to its default. Keeps the sign-in and the folder.",
            &p,
            |ui| reset = button(ui, label, !self.exporting, false, &p).clicked(),
        );
        if reset {
            self.reset_settings();
        }
        row(ui, |ui| {
            ui.label(meta(
                "Keys: Ctrl+1–5 views · Ctrl+F filter · Ctrl+A tick all shown · \
                 Ctrl+E export · F5 reload · Esc clear filter",
                &p,
            ));
        });
        row(ui, |ui| {
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
    /// the sign-in needs, the destination (the Library and every database
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
        for (label, why, which) in [
            (
                "From",
                "Only messages from this day on, as 2026-01-31. Blank is the start.",
                0,
            ),
            (
                "Until",
                "Up to and including this day. Blank is up to now.",
                1,
            ),
        ] {
            let form = &mut self.form;
            let buffer = if which == 0 {
                &mut form.date_from
            } else {
                &mut form.date_until
            };
            commit |= setting_row(ui, label, why, &p, |ui| {
                ui.add(
                    field(buffer, &p)
                        .hint_text("YYYY-MM-DD")
                        .desired_width(DATE_W),
                )
                .lost_focus()
            });
        }
        if commit {
            self.commit_settings();
        }
        if let Err(e) = self.settings.date_range() {
            row(ui, |ui| ui.label(text(e, &p)));
        }
    }

    /// A label with a narrow number field on the right. Returns whether the
    /// field just lost focus, which is when the value is a decision.
    fn number_row(
        &mut self,
        ui: &mut Ui,
        label: &str,
        why: &str,
        enabled: bool,
        which: impl Fn(&mut crate::settings_form::SettingsForm) -> &mut String,
    ) -> bool {
        let p = self.palette;
        let form = &mut self.form;
        setting_row(ui, label, why, &p, |ui| {
            ui.add_enabled(enabled, field(which(form), &p).desired_width(NUMBER_W))
                .lost_focus()
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

/// How wide a number field is. Every value in this panel is at most five
/// digits, and a field sized for a sentence invites one.
const NUMBER_W: f32 = 80.0;
/// How wide the model dropdown is: a quantised model's long file name.
const MODEL_W: f32 = 320.0;

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
/// How wide a date field is: `2026-01-31` and a little air.
const DATE_W: f32 = 104.0;

/// A two-cell [`segmented`] strip inside a [`setting_row`]'s control, which
/// is laid out right to left: the cells are given reversed and the answer
/// turned back round, so they still read left to right.
fn pair(ui: &mut Ui, cells: [&str; 2], at: usize, p: &Palette) -> Option<usize> {
    segmented(ui, &[cells[1], cells[0]], 1 - at, p).map(|i| 1 - i)
}

/// One switch: [`setting_row`] with a tick box for its control.
fn switch_row(ui: &mut Ui, label: &str, why: &str, on: bool, enabled: bool, p: &Palette) -> bool {
    setting_row(ui, label, why, p, |ui| {
        tick_box(ui, on, enabled, p).clicked()
    })
}

/// A tick box and its label, as one control — for the media categories,
/// which sit seven to a wrapped strip with no room for a row each.
fn tick(ui: &mut Ui, label: &str, on: bool, enabled: bool, p: &Palette) -> bool {
    tick_box(ui, on, enabled, p)
        .union(action(ui, text(label, p), enabled))
        .clicked()
}

/// How wide [`tick`] will draw: the box (`tick_box` is one reading size
/// square), the gap, and the label as a frameless button, padding included.
fn tick_width(ui: &Ui, label: &str, p: &Palette) -> f32 {
    let galley = egui::WidgetText::from(text(label, p)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    tgx_ui::tokens::window::READING
        + space::TIGHT
        + galley.size().x
        + 2.0 * ui.spacing().button_padding.x
}

/// A section heading. **Space above it, and no line under it.**
///
/// `BREAK` above the heading does the work a rule used to: a gap four times
/// the size of the one between two rows is not ambiguous about where a group
/// starts. Capitals, because this is the one thing capitals are for — a
/// section label of three words or fewer.
fn section(ui: &mut Ui, heading: &str, p: &Palette) {
    ui.add_space(space::BREAK);
    row(ui, |ui| ui.label(eyebrow(heading, p)));
    ui.add_space(space::TIGHT);
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

// -- the three `[?]` left --------------------------------------------------
//
// Where a choice has consequences one line cannot hold.

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
    "Read the whole chat again instead of the newest few hundred. Slow, and \
     the only way to catch an edit or a deletion older than the re-read window \
     — or to pick up media after raising the size limit.";

/// The [`tgx_tg::config::Settings::profile_photos`] values, in the order the
/// strip shows them left to right.
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
        // A key here that `plan.rs` does not compare against is a tick box that
        // switches nothing off; a key there with no box is a category the user
        // cannot reach.
        let labelled: Vec<&str> = MEDIA_LABELS.iter().map(|(k, _)| *k).collect();
        for key in MEDIA_KINDS {
            assert!(labelled.contains(&key), "{key} has no tick box");
        }
        assert_eq!(labelled.len(), MEDIA_KINDS.len());
    }

    #[test]
    fn every_extra_reads_and_writes_its_own_field() {
        // A copy-paste in `set_extra` would silently point two rows at one
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
    fn every_line_under_a_setting_is_one_sentence_and_no_two_are_the_same() {
        // The line under a setting is what replaced the `[?]`, so it has to
        // be there, be short enough to stay one line in a column, and not be
        // a copy of its neighbour — the shape a copy-paste takes here.
        let lines: Vec<&str> = EXTRAS.iter().map(|(_, _, why)| *why).collect();
        for why in &lines {
            assert!(why.len() > 20, "too short to say anything: {why}");
            assert!(why.len() <= 80, "more than a line: {why}");
            assert!(why.trim_end().ends_with('.'), "not a sentence: {why}");
        }
        let mut unique = lines.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), lines.len(), "two settings share a line");
        // And the three that kept a `[?]` say more than a line could.
        for long in [MODE_HELP, REREAD_ALL_HELP, PHOTOS_HELP] {
            assert!(long.len() > 120, "a [?] that fits on a line: {long}");
        }
    }

    #[test]
    fn the_appearance_control_shows_the_palette_that_is_actually_on_screen() {
        // `Palette::named` treats anything that is not `light` as dark, so a
        // settings file naming something else must fill the Dark cell — a strip
        // showing Light over a dark window is worse than no strip.
        for (name, expect) in [("light", 0), ("dark", 1), ("chartreuse", 1)] {
            assert_eq!(theme_index(name), expect, "{name}");
            assert_eq!(
                Palette::named(THEMES[theme_index(name)]),
                Palette::named(name)
            );
        }
        assert_eq!(theme_index("system"), 2);
    }
}
