//! TelegramTools: the exporter and the analyser, one window and one exe.
//!
//! The layout is a port; the *interaction rules* are behaviour, hard-won, and
//! survive the toolkit change unchanged. Each one carries the reason it exists
//! at the place it is enforced.
//!
//! **Arguments mean a command, none means the window.** This was three
//! executables — the exporter's window, its `tgx` command line, and the
//! analyser, which already chose between window and command by argv. The only
//! difference between them was ever how they were started, which is a thing
//! argv records.

// **A release build is a GUI binary, so double-clicking it does not open a
// console window behind the app.** Without this the exe defaults to the console
// subsystem and Windows allocates one, which looks like the program started
// something else.
//
// Debug builds keep the console, because that is where `cargo run` lives and
// the startup diagnostics below are worth more than the tidiness.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod bridge;
mod cli;
mod diagnostics;
mod fold;
mod holder;
mod instance;
mod journal;
mod list;
mod login;
mod notify;
mod pace;
mod pending;
mod queue;
mod settings_form;
mod shell;
mod volume;

/// What the window and its failures call themselves.
pub const TITLE: &str = "TelegramTools";

/// The icon `build.rs` embeds in the exe, drawn by `tools/make_icon.py`. The
/// same file for the window, so the two cannot disagree.
const ICON: &[u8] = include_bytes!("../icon/TelegramTools.ico");

/// Record a failure somewhere a user can actually find it.
///
/// A GUI-subsystem binary has no stderr unless it was launched from a terminal,
/// and **a blank window is the worst possible failure precisely because it says
/// nothing**. So the message goes three places: stderr, which reaches a
/// developer running `cargo run`; the log, which reaches anything attached to
/// it; and a file beside the executable, which is the only one that survives a
/// double-click.
fn report_startup_failure(message: &str) {
    eprintln!("{message}");
    log::error!("{message}");
    if let Ok(dir) = tgx_tg::config::ensure_data_dir() {
        let _ = std::fs::write(dir.join("startup-error.log"), message);
    }
}

/// Whether the window ever opened.
///
/// **A panic twenty minutes into an export is not a startup failure**, and
/// telling someone their machine needs working graphics drivers when the app
/// has been running all afternoon sends them to fix something that is not
/// broken. The hook says which kind of failure it is, and it can only know that
/// by being told.
static WINDOW_OPENED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The two ways a panic gets reported, chosen by whether the window ever
/// opened.
///
/// Pulled out of the hook itself so the choice can be tested without staging
/// a real panic or a real window: this crate has no `PanicHookInfo` it can
/// construct in a test, so `panic` here is already the formatted description,
/// which is all the branch below ever looks at.
///
/// **This is the fix for the bug the flag above was written to prevent, and
/// did not.** `WINDOW_OPENED` existed and this branch existed, but nothing
/// ever called `.store(true, ..)` on it — so the hook always took the `else`
/// below, and a panic inside a two-hour-old export was reported as needing a
/// driver update, overwriting `startup-error.log` with a diagnosis that sent
/// the user to fix a graphics card that was never broken.
fn panic_message(window_opened: bool, panic: &str) -> String {
    if window_opened {
        // The renderer plainly works — it has been drawing. Naming the GPU
        // here would send someone to update a driver that is fine, and the
        // thing that actually matters is what was on screen when it went.
        format!(
            "{TITLE} stopped unexpectedly: {panic}\n\
             An export already written to disk is unaffected; the JSON is \
             closed as each chat finishes, and a report is only ever written \
             whole."
        )
    } else {
        format!(
            "{TITLE} could not start: {panic}\n\
             This build draws with OpenGL and needs working graphics drivers."
        )
    }
}

/// Borrow the console of whatever launched us, if there was one.
///
/// **A release build is a GUI-subsystem binary, and a GUI binary starts with no
/// stdout at all** — so `println!` from a command would write into nothing and
/// the program would look like it had silently done nothing. Windows hands the
/// parent's console over on request, and `AttachConsole` is that request. It
/// fails when there is no parent console (double-clicked, or piped), and
/// failing is fine: the output goes where it was already going.
///
/// What it does not do is make cmd.exe *wait*: cmd returns to its prompt as
/// soon as a GUI-subsystem program starts, so a command that reads the
/// keyboard — `login` — must be run as `start /wait TelegramTools login`.
/// PowerShell waits on its own. The usage text says so.
#[cfg(windows)]
fn attach_parent_console() {
    // Declared rather than pulled in with the `windows` crate: one FFI line
    // against a stable kernel32 export does not need a dependency tree.
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    extern "system" {
        fn AttachConsole(dwProcessId: u32) -> i32;
    }
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

/// Which command an argv names. Pulled out of `main` so it can be tested.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Window,
    /// The analyser's command line, with the word `analyse` taken off.
    Analyse(Vec<String>),
    /// A local model's summary of a date range, with the word taken off.
    Summarise(Vec<String>),
    /// Everything else is one of the exporter's: `login`, `export`, `--help`…
    Export(Vec<String>),
}

fn command(mut args: Vec<String>) -> Command {
    match args.first().map(String::as_str) {
        None => Command::Window,
        // Both spellings, and `report` for the thing it makes: the window's
        // button says "Make report", and a command line should not make you
        // remember which English the author writes.
        Some("analyse" | "analyze" | "report") => {
            args.remove(0);
            Command::Analyse(args)
        }
        Some("summarise" | "summarize" | "summary") => {
            args.remove(0);
            Command::Summarise(args)
        }
        Some(_) => Command::Export(args),
    }
}

/// A command's ending: nothing on success, the whole error and exit 1 on
/// failure.
fn finish(ran: anyhow::Result<()>) {
    if let Err(e) = ran {
        // `{e:#}` so anyhow's context chain prints, not just the outermost
        // message -- "Not a folder" without the path it tried is no help.
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = command(args);
    if command != Command::Window {
        attach_parent_console();
    }
    // **The analyser runs before the log opens**, as it always did: opening it
    // rotates `tgx.log` to `tgx.prev.log`, and a report written from a
    // terminal must not push the window's last session out of the one slot
    // that keeps it. Nothing in the analyser logs.
    if let Command::Analyse(args) = command {
        finish(tga_run::cli::run(args));
        return;
    }
    // The same for a summary: the analyser's side, offline but for loopback.
    if let Command::Summarise(args) = command {
        let models = tgx_tg::config::Settings::load().summary_models_dir();
        finish(tga_run::summary::run(args, &models));
        return;
    }
    // **And a second window leaves before it too**, for the same reason and a
    // worse one: it would rotate the log the first window is writing. See
    // `instance`.
    // A photograph (`TGX_SHOT`) is exempt: it touches no account and quits
    // on its own, and it is taken while a real window is open.
    if command == Command::Window
        && std::env::var_os("TGX_SHOT").is_none()
        && !instance::claim(&tgx_tg::config::data_dir())
    {
        return;
    }

    // **First, because until this line every `log::` call in the program was a
    // no-op.** Nothing installed a `log` implementation, so our own warnings —
    // and grammers' `connecting...`, `generating new authorization key...`,
    // `authorization key generated successfully`, which between them are the
    // whole timing of a sign-in — went nowhere. A GUI-subsystem binary has no
    // stderr, so the file is the only channel that survives a double-click.
    //
    // A failure to open it is recorded and stepped over: refusing to start
    // because the *diagnostic* could not be written would cost someone an
    // export over a file they never asked for.
    if let Err(e) = tgx_tg::logging::init() {
        eprintln!("could not open the log: {e}");
    }

    if let Command::Export(args) = command {
        finish(cli::run(args));
        return;
    }

    // What the Viewer's Open media copied out last time, if that run ended
    // without reaching `on_exit` — a crash, or a kill from Task Manager.
    tgx_tg::viewer::clear_temp();

    std::panic::set_hook(Box::new(|info| {
        use std::sync::atomic::Ordering;
        let message = panic_message(WINDOW_OPENED.load(Ordering::Relaxed), &info.to_string());
        report_startup_failure(&message);
    }));

    // The views' floor, plus the rail beside them: the floor was measured for
    // the body, and the rail came out of nothing until it was added here.
    let (w, h) = tgx_ui::components::min_window();
    let w = w + shell::RAIL_WIDTH;
    // `TGX_SHOT_SIZE=WxH` opens the window at that size, so `tools/shot.ps1`
    // can photograph the narrow window — which is where layouts break — and
    // not only the default one. Clamped by the floor below like a drag would be.
    let shot_size = std::env::var("TGX_SHOT_SIZE").ok().and_then(|s| {
        let (a, b) = s.split_once('x')?;
        Some([a.trim().parse().ok()?, b.trim().parse().ok()?])
    });
    let settings = tgx_tg::config::Settings::load();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_inner_size(shot_size.unwrap_or([w + 320.0, h + 160.0]))
        // **The floor the layout was measured at.** Without it the window
        // drags below 900x620, where the queue's Chat column — the only
        // thing naming which row is which — is squeezed to 11px, i.e. gone,
        // while the count columns beside it keep their full width.
        .with_min_inner_size([w, h]);
    // **Where it was last time** — unless a photograph asked for a size, which
    // has to be the same every time. A position on a monitor that is no
    // longer there is dropped, or the window would open somewhere nobody can
    // reach; the size is kept either way.
    if shot_size.is_none() {
        if let [x, y, width, height] = settings.window[..] {
            if on_screen(x, y) {
                viewport = viewport.with_position([x, y]);
            }
            viewport = viewport.with_inner_size([width.max(w), height.max(h)]);
        }
        if settings.window_maximized {
            viewport = viewport.with_maximized(true);
        }
    }
    // **eframe stamps its own logo on the window** when given no icon, and the
    // taskbar draws the window's icon, not the exe's — so the one embedded by
    // `build.rs` showed in Explorer and nowhere the app was actually running.
    let viewport = match window_icon(ICON) {
        Some(icon) => viewport.with_icon(icon),
        None => viewport,
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let started = eframe::run_native(
        TITLE,
        options,
        Box::new(|cc| {
            // The earliest point at which "the window opened" is true — past
            // here a panic is not a startup failure. See `panic_message`. The
            // GPUI version had this flag and never set it, so every panic took
            // the other branch.
            WINDOW_OPENED.store(true, std::sync::atomic::Ordering::Relaxed);
            // The design, before the first frame draws anything in egui's own
            // rounded, shadowed defaults. `Shell::update` reinstalls it when the
            // appearance changes and not otherwise — see `theme_stale`.
            tgx_ui::theme::install(
                &cc.egui_ctx,
                &tgx_ui::tokens::Palette::named(&settings.theme),
            );
            Ok(Box::new(shell::Shell::with(settings)))
        }),
    );
    if let Err(e) = started {
        report_startup_failure(&format!("could not open a window: {e}"));
    }
    log::info!("window gone");
}

/// Is a window's top-left corner, in logical points, somewhere on the
/// desktop as it is now? Asked of the whole virtual screen, which is in
/// physical pixels — at a display scale above 100% a logical point is never
/// further out than its pixel, so this errs towards keeping the position.
#[cfg(windows)]
fn on_screen(x: f32, y: f32) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    // SAFETY: plain metric queries.
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_YVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32,
        )
    };
    // The title bar has to be reachable, so a corner just past the far edge
    // does not count.
    x >= left - 8.0 && y >= top - 8.0 && x < left + width - 100.0 && y < top + height - 50.0
}

#[cfg(not(windows))]
fn on_screen(_x: f32, _y: f32) -> bool {
    true
}

/// The largest PNG-encoded image in an `.ico`, decoded for the window.
///
/// `tools/make_icon.py` writes the 256px size as PNG — the others are BMP —
/// and eframe already links a PNG decoder for its own logo, so reading the
/// directory is all this adds. `None` leaves eframe's default: a missing icon
/// is cosmetic and never worth failing a launch over.
fn window_icon(ico: &[u8]) -> Option<eframe::egui::IconData> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(ico.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(ico.get(at..at + 4)?.try_into().ok()?));
    let count = usize::from(u16_at(4)?);
    let png = (0..count)
        .filter_map(|i| {
            let entry = 6 + 16 * i;
            // A width byte of 0 means 256.
            let width = match *ico.get(entry)? {
                0 => 256,
                w => u32::from(w),
            };
            let size = u32_at(entry + 8)? as usize;
            let offset = u32_at(entry + 12)? as usize;
            let bytes = ico.get(offset..offset.checked_add(size)?)?;
            bytes.starts_with(b"\x89PNG").then_some((width, bytes))
        })
        .max_by_key(|(width, _)| *width)?
        .1;
    eframe::icon_data::from_png_bytes(png).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_wears_the_exes_own_icon() {
        let icon = window_icon(ICON).expect("the shipped .ico carries a PNG image");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        // The one red is in it: the mark is the tallest bar, and an icon that
        // decoded to a blank tile would still pass the size checks.
        let red = icon.rgba.chunks(4).any(|px| px == [0xff, 0x33, 0x47, 0xff]);
        assert!(red, "the accent bar is missing from the decoded icon");
        assert!(window_icon(b"not an icon").is_none());
        assert!(window_icon(&[0, 0, 1, 0, 9, 0]).is_none());
    }

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn no_arguments_open_the_window() {
        assert_eq!(command(Vec::new()), Command::Window);
    }

    #[test]
    fn every_spelling_of_analyse_reaches_the_analyser_without_the_word() {
        for word in ["analyse", "analyze", "report"] {
            assert_eq!(
                command(argv(&[word, r"C:\export", "--digest"])),
                Command::Analyse(argv(&[r"C:\export", "--digest"])),
                "{word}"
            );
        }
    }

    #[test]
    fn every_spelling_of_summarise_reaches_the_summary_without_the_word() {
        for word in ["summarise", "summarize", "summary"] {
            assert_eq!(
                command(argv(&[word, r"C:\db.sqlite", "--from", "2024-12-01"])),
                Command::Summarise(argv(&[r"C:\db.sqlite", "--from", "2024-12-01"])),
                "{word}"
            );
        }
    }

    #[test]
    fn anything_else_is_the_exporters_and_keeps_its_first_word() {
        assert_eq!(
            command(argv(&["export", "--db", "Dev Team"])),
            Command::Export(argv(&["export", "--db", "Dev Team"]))
        );
        // A folder alone used to be the analyser's whole command line. It is
        // the exporter's now, which prints its usage — naming `analyse`.
        assert_eq!(
            command(argv(&[r"C:\export"])),
            Command::Export(argv(&[r"C:\export"]))
        );
    }

    #[test]
    fn the_usage_names_both_halves_and_the_cmd_caveat() {
        assert!(cli::USAGE.contains("TelegramTools analyse"));
        assert!(cli::USAGE.contains("start /wait"));
        assert!(tga_run::cli::USAGE.contains("TelegramTools analyse"));
    }

    #[test]
    fn a_panic_before_the_window_opens_blames_the_graphics_stack() {
        let message = panic_message(false, "boom");
        assert!(message.contains("could not start"));
        assert!(message.contains("graphics drivers"));
    }

    #[test]
    fn a_panic_after_the_window_opened_does_not_blame_the_graphics_stack() {
        // The branch that was unreachable for a long time, because nothing ever
        // set the flag: a panic deep into an export was reported as a graphics
        // driver problem.
        let message = panic_message(true, "boom");
        assert!(!message.contains("graphics drivers"));
        assert!(message.contains("stopped unexpectedly"));
        assert!(message.contains("unaffected"));
    }

    #[test]
    fn the_flag_starts_false_so_a_startup_panic_gets_the_driver_message() {
        assert!(!WINDOW_OPENED.load(std::sync::atomic::Ordering::Relaxed));
    }
}
