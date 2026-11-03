//! **TelegramViewer.exe: read the databases TelegramTools wrote.** It lives
//! in the export folder — Settings › What an export contains › Viewer beside
//! the databases puts it there — and opens on the databases beside it, so a folder of
//! exports copied to a drive or handed to somebody can be read with nothing
//! installed and no Telegram account anywhere near it.
//!
//! It is TelegramTools' own Viewer ([`tgx_viewer::Viewer`]), the same code
//! and the same screen, with a list of the folder's chats beside it in place
//! of TelegramTools' Analysis. It links no Telegram client, no exporter and
//! no analyser, which is why it is a fraction of TelegramTools' size.
//!
//! **Read-only, and it writes nothing beside itself**: no settings, no log.
//! The only files it makes are the copies Open hands to Windows, in a folder
//! under `%TEMP%` emptied on the way in and the way out.
//!
//! `TelegramViewer [folder | file.sqlite]`: a folder to list, or one database
//! to open; with neither, the folder the exe is in.

// A GUI binary in release, so double-clicking it opens no console behind it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Context};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use tgx_ui::components::{action, field, figure, meta, row, rule, text, thousands, GUTTER};
use tgx_ui::tokens::{space, Palette};
use tgx_viewer::{Head, Host, Job, Reply, Viewer};

const TITLE: &str = "TelegramViewer";

/// TelegramTools' icon: the two are one product.
const ICON: &[u8] = include_bytes!("../../tt-app/icon/TelegramTools.ico");

/// The chat list beside the Viewer.
const LIST_W: f32 = 240.0;
/// One chat in it.
const LIST_ROW: f32 = 28.0;

fn main() {
    let (folder, file) = start_from(std::env::args_os().nth(1).map(PathBuf::from));
    // From an earlier run that ended without reaching `on_exit`.
    tgx_viewer::data::clear_temp();

    // Two workers: one reading a chat while the other decodes its pictures.
    // The reads themselves go to the blocking pool (`jobs`).
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("{TITLE} could not start its worker: {e}");
            return;
        }
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title_for(&folder))
        .with_inner_size([1180.0, 760.0])
        .with_min_inner_size([760.0, 480.0]);
    if let Some(icon) = tgx_ui::theme::window_icon(ICON) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let started = eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| {
            let palette = palette_for(cc.egui_ctx.system_theme());
            tgx_ui::theme::install(&cc.egui_ctx, &palette);
            Ok(Box::new(App::new(runtime, folder, file, palette)))
        }),
    );
    if let Err(e) = started {
        eprintln!("{TITLE} could not open a window: {e}");
    }
}

/// What the command line asks for: a folder to list, and a database in it to
/// open first. Nothing, or something that is neither, is the exe's own folder
/// — where Settings puts it, beside the databases.
fn start_from(arg: Option<PathBuf>) -> (PathBuf, Option<PathBuf>) {
    if let Some(arg) = arg {
        if arg.is_dir() {
            return (arg, None);
        }
        if arg.is_file() {
            if let Some(parent) = arg.parent() {
                // `database_files` looks in a folder and one down, so the
                // file's own folder lists it.
                return (parent.to_path_buf(), Some(arg));
            }
        }
    }
    let own = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    (own, None)
}

fn title_for(folder: &Path) -> String {
    match folder.file_name() {
        Some(name) => format!("{TITLE} — {}", name.to_string_lossy()),
        None => TITLE.to_string(),
    }
}

/// Windows' own light or dark mode; dark when it cannot be told.
fn palette_for(theme: Option<egui::Theme>) -> Palette {
    match theme {
        Some(egui::Theme::Light) => Palette::light(),
        _ => Palette::dark(),
    }
}

struct App {
    runtime: tokio::runtime::Runtime,
    tx: Sender<Reply>,
    rx: Receiver<Reply>,
    viewer: Viewer,
    palette: Palette,
    /// The status line: counts and file names, never message text.
    status: String,
    /// The chat list's filter box.
    filter: String,
}

impl App {
    fn new(
        runtime: tokio::runtime::Runtime,
        folder: PathBuf,
        file: Option<PathBuf>,
        palette: Palette,
    ) -> Self {
        let (tx, rx) = channel();
        let mut app = Self {
            runtime,
            tx,
            rx,
            viewer: Viewer::default(),
            palette,
            status: String::new(),
            filter: String::new(),
        };
        app.look_in(folder);
        app.viewer.want_path = file;
        app
    }

    /// Start over on another folder: what was open belongs to the old one.
    fn look_in(&mut self, folder: PathBuf) {
        self.viewer = Viewer {
            folder: folder.to_string_lossy().into_owned(),
            ..Default::default()
        };
        self.filter.clear();
    }

    /// The Viewer's way to the runtime, the status line and the window.
    fn host<'a>(
        runtime: &'a tokio::runtime::Runtime,
        tx: &'a Sender<Reply>,
        ctx: &'a Context,
        status: &'a mut String,
    ) -> Port<'a> {
        Port {
            runtime,
            tx,
            ctx,
            status,
        }
    }

    /// The chats in the folder, filtered by the box above them. A click opens
    /// one.
    fn chat_list(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let p = self.palette;
        let mut open = None;
        let mut pick = false;
        ui.add_space(space::STEP);
        row(ui, |ui| {
            ui.label(text("Chats", &p).strong());
            ui.label(figure(thousands(self.viewer.databases.len() as i64), &p));
        });
        ui.add_space(space::TIGHT);
        row(ui, |ui| {
            ui.add(
                field(&mut self.filter, &p)
                    .hint_text("Filter…")
                    .desired_width(ui.available_width()),
            );
        });
        ui.add_space(space::TIGHT);
        rule(ui, &p);
        let shown: Vec<usize> = (0..self.viewer.databases.len())
            .filter(|&i| tgx_viewer::fold::matches(&self.viewer.databases[i].title, &self.filter))
            .collect();
        egui::TopBottomPanel::bottom("viewer-list-foot")
            .frame(egui::Frame::NONE.fill(p.bg))
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                rule(ui, &p);
                ui.add_space(space::TIGHT);
                row(ui, |ui| {
                    if action(ui, text("Open folder…", &p), true).clicked() {
                        pick = true;
                    }
                });
                ui.add_space(space::TIGHT);
            });
        egui::ScrollArea::vertical()
            .id_salt("viewer-chats")
            .auto_shrink([false, false])
            .show_rows(ui, LIST_ROW, shown.len(), |ui, range| {
                for i in range {
                    let db = &self.viewer.databases[shown[i]];
                    let on = self
                        .viewer
                        .open
                        .as_ref()
                        .is_some_and(|o| o.path == db.path && o.chat_id == db.chat_id);
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), LIST_ROW),
                        egui::Sense::click(),
                    );
                    if on || response.hovered() {
                        ui.painter()
                            .rect_filled(rect, egui::CornerRadius::ZERO, p.surface);
                    }
                    if on {
                        let bar =
                            egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height()));
                        ui.painter()
                            .rect_filled(bar, egui::CornerRadius::ZERO, p.accent);
                    }
                    let inner = rect.shrink2(egui::vec2(GUTTER, 0.0));
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(inner)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| ui.add(egui::Label::new(text(&db.title, &p)).truncate()),
                    );
                    if response.clicked() {
                        open = Some(db.clone());
                    }
                }
            });
        if let Some(db) = open {
            let mut host = Self::host(&self.runtime, &self.tx, ctx, &mut self.status);
            self.viewer.open_db(db, None, &mut host);
        }
        if pick {
            let start = PathBuf::from(&self.viewer.folder);
            if let Some(folder) = rfd::FileDialog::new()
                .set_title("Open the folder the databases are in")
                .set_directory(&start)
                .pick_folder()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title_for(&folder)));
                self.look_in(folder);
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Follow Windows' light or dark mode while open.
        let wanted = palette_for(ctx.system_theme());
        if wanted != self.palette {
            self.palette = wanted;
            tgx_ui::theme::install(ctx, &self.palette);
        }
        while let Ok(reply) = self.rx.try_recv() {
            let mut host = Self::host(&self.runtime, &self.tx, ctx, &mut self.status);
            self.viewer.apply(reply, &mut host);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            self.viewer.focus_search = true;
        }

        let p = self.palette;
        let bare = egui::Frame::NONE.fill(p.bg);
        egui::TopBottomPanel::bottom("viewer-status")
            .frame(bare)
            .show_separator_line(false)
            .show(ctx, |ui| {
                rule(ui, &p);
                ui.add_space(space::TIGHT / 2.0);
                row(ui, |ui| {
                    ui.add(egui::Label::new(meta(&self.status, &p)).truncate());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(figure(&self.viewer.folder, &p)).truncate());
                    });
                });
                ui.add_space(space::TIGHT / 2.0);
            });
        egui::SidePanel::left("viewer-chats-panel")
            .frame(bare)
            .exact_width(LIST_W)
            .resizable(false)
            .show(ctx, |ui| self.chat_list(ui, ctx));
        egui::CentralPanel::default().frame(bare).show(ctx, |ui| {
            let none_open = if !self.viewer.listing && self.viewer.databases.is_empty() {
                "There are no TelegramTools databases in this folder. Open folder… picks \
                 the one they are in."
            } else {
                "Pick a chat on the left."
            };
            let mut host = Self::host(&self.runtime, &self.tx, ctx, &mut self.status);
            self.viewer.panel(
                ui,
                &p,
                &mut host,
                Head {
                    back: None,
                    unpack: false,
                    none_open,
                },
            );
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        tgx_viewer::data::clear_temp();
    }
}

/// [`Host`] for this window: work on the runtime, answers down a channel
/// that wakes the window on every send — without the wake, an answer would
/// wait in the channel until the mouse moved.
struct Port<'a> {
    runtime: &'a tokio::runtime::Runtime,
    tx: &'a Sender<Reply>,
    ctx: &'a Context,
    status: &'a mut String,
}

impl Host for Port<'_> {
    fn run(&mut self, job: Job) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.runtime.spawn(async move {
            tgx_viewer::jobs::run(job, move |reply| {
                let _ = tx.send(reply);
                ctx.request_repaint();
            })
            .await;
        });
    }

    fn status(&mut self, line: String) {
        *self.status = line;
    }

    /// No log to keep it in — this program writes nothing beside itself — so
    /// the status line, where it has already been said, is where it stays.
    fn warn(&mut self, _line: String) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_database_on_the_command_line_opens_in_its_own_folder() {
        let dir = std::env::temp_dir().join(format!("tt-viewer-{}", std::process::id()));
        let chat = dir.join("Dev");
        std::fs::create_dir_all(&chat).unwrap();
        let file = chat.join("Dev.sqlite");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(start_from(Some(file.clone())), (chat.clone(), Some(file)));
        assert_eq!(start_from(Some(dir.clone())), (dir.clone(), None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_on_the_command_line_is_the_exes_own_folder() {
        let (folder, file) = start_from(None);
        let exe = std::env::current_exe().unwrap();
        assert_eq!(Some(folder.as_path()), exe.parent());
        assert!(file.is_none());
        // And a path that is not there at all falls back the same way.
        assert_eq!(
            start_from(Some(PathBuf::from(r"Z:\no\such\place"))).0,
            folder
        );
    }

    #[test]
    fn the_title_names_the_folder() {
        assert_eq!(
            title_for(Path::new(r"C:\Exports")),
            "TelegramViewer — Exports"
        );
    }
}
