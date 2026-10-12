//! The Telegram export analyser.
//!
//! **One executable, two front ends.** Run it with arguments and it behaves as
//! the old `tga.exe` did — read an export, write a report, print what it found.
//! Run it with none and the window opens. There was never a difference between
//! the two beyond how they were launched, and shipping that difference as a
//! second file meant two things to copy and keep in step.

// **A release build is a GUI binary, so double-clicking it does not open a
// console window behind the app.** Without this the exe defaults to the console
// subsystem and Windows allocates one, which looks like the program started
// something else.
//
// Debug builds keep the console, because that is where `cargo run` lives and
// the startup diagnostics below are worth more than the tidiness.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod cli;
mod job;
mod open;
mod refresh;
mod shot;
mod state;
mod theme;

/// Record a failure somewhere a user can actually find it.
///
/// A GUI-subsystem binary has no stderr unless it was launched from a terminal,
/// and **a blank window is the worst possible failure precisely because it says
/// nothing**. So the message goes to stderr, which reaches a developer running
/// `cargo run`, and to a file beside the executable, which is the only one that
/// survives a double-click.
fn report_startup_failure(message: &str) {
    eprintln!("{message}");
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = std::fs::write(dir.join("startup-error.log"), message);
        }
    }
}

/// Whether the window ever opened.
///
/// **A panic six seconds into a large export is not a startup failure**, and
/// telling someone their machine needs working graphics drivers when the app
/// has been running all afternoon sends them to fix something that is not
/// broken. The hook says which kind of failure it is, and it can only know that
/// by being told — the exporter carried this flag for a while without anything
/// ever setting it, so every panic got the wrong message.
static WINDOW_OPENED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn panic_message(window_opened: bool, panic: &str) -> String {
    if window_opened {
        // The renderer plainly works — it has been drawing. Naming the GPU here
        // would send someone to update a driver that is fine.
        format!(
            "{} stopped unexpectedly: {panic}\n\
             Nothing was sent anywhere and the export on disk is untouched; \
             this app only ever reads it.",
            app::TITLE
        )
    } else {
        format!(
            "{} could not start: {panic}\n\
             This build draws with OpenGL and needs working graphics drivers.",
            app::TITLE
        )
    }
}

/// Borrow the console of whatever launched us, if there was one.
///
/// **A release build is a GUI-subsystem binary, and a GUI binary starts with no
/// stdout at all** — so `println!` from the command half would write into
/// nothing and the program would look like it had silently done nothing. Windows
/// hands the parent's console over on request, and `AttachConsole` is that
/// request. It fails when there is no parent console (double-clicked, or piped),
/// and failing is fine: the output goes where it was already going.
///
/// This is the whole price of shipping one executable instead of two, and it is
/// paid once, here.
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

fn main() {
    // **Arguments mean the command, none means the window.** This used to be
    // two executables; the only difference between them was ever how they were
    // started, which is a thing argv already records.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        attach_parent_console();
        if let Err(e) = cli::run(args) {
            // `{e:#}` so anyhow's context chain prints, not just the outermost
            // message -- "Not a folder" without the path it tried is no help.
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    std::panic::set_hook(Box::new(|info| {
        use std::sync::atomic::Ordering;
        let message = panic_message(WINDOW_OPENED.load(Ordering::Relaxed), &info.to_string());
        report_startup_failure(&message);
    }));

    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title(app::TITLE)
        // `TGA_SHOT_SIZE` opens it at a chosen size, so `tools/shot.ps1` can
        // photograph the floor too; clamped by the floor like a drag.
        .with_inner_size(shot::size().unwrap_or(theme::WINDOW))
        // Stated rather than left to chance: the exporter's window drifted
        // under its own floor and squeezed the only column that named the rows
        // down to nothing.
        .with_min_inner_size(theme::min_window())
        .with_drag_and_drop(true);
    // **eframe stamps its own logo on the window** when given no icon, and the
    // taskbar draws the window's icon, not the exe's — so the one `build.rs`
    // embeds would show in Explorer and nowhere the app is actually running.
    // The exporter shipped exactly that. The same file, so the two cannot
    // disagree.
    let viewport = match window_icon(ICON) {
        Some(icon) => viewport.with_icon(icon),
        None => viewport,
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let started = eframe::run_native(
        app::TITLE,
        options,
        Box::new(|cc| {
            // The earliest point at which "the window opened" is true — past
            // here a panic is not a startup failure. See `panic_message`.
            WINDOW_OPENED.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(Box::new(app::Shell::new(&cc.egui_ctx)))
        }),
    );
    if let Err(e) = started {
        report_startup_failure(&format!("could not open a window: {e}"));
    }
}

/// The icon `build.rs` embeds in the exe, drawn by `tools/make_icon.py`.
const ICON: &[u8] = include_bytes!("../icon/TelegramAnalyser.ico");

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
    }

    #[test]
    fn a_broken_icon_file_falls_back_rather_than_failing_the_launch() {
        assert!(window_icon(b"").is_none());
        assert!(window_icon(&[0, 0, 1, 0, 9, 0]).is_none());
    }

    #[test]
    fn a_panic_before_the_window_opens_blames_the_graphics_stack() {
        let message = panic_message(false, "boom");
        assert!(message.contains("could not start"));
        assert!(message.contains("graphics drivers"));
    }

    #[test]
    fn a_panic_after_the_window_opened_does_not_blame_the_graphics_stack() {
        // The branch that was unreachable in the exporter for a long time,
        // because nothing ever set the flag: a panic deep into a run was
        // reported as a graphics driver problem.
        let message = panic_message(true, "boom");
        assert!(!message.contains("graphics drivers"));
        assert!(message.contains("stopped unexpectedly"));
        assert!(message.contains("untouched"));
    }

    #[test]
    fn the_flag_starts_false_so_a_startup_panic_gets_the_driver_message() {
        assert!(!WINDOW_OPENED.load(std::sync::atomic::Ordering::Relaxed));
    }
}
