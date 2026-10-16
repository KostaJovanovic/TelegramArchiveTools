//! Running the analyser: everything between an argv or a click and a written
//! report, with no window in it.
//!
//! - [`cli`] — `TelegramTools analyse <export> [options]`.
//! - [`refresh`] — every export under a folder, whether its report is stale,
//!   and the one function that analyses one.
//! - [`job`] — that function on a worker thread, a queue of rows at a time.
//! - [`state`] — every rule the library window applies, testable without it.
//! - [`open`] — handing a finished report to whatever opens HTML.
//! - [`summary`] — `TelegramTools summarise`: a local model's summary of a
//!   date range, through `tga-sum`.

use std::path::Path;

pub mod catchup;
pub mod cli;
pub mod job;
pub mod open;
pub mod refresh;
pub mod state;
pub mod summary;

/// The summary's vocabulary — lengths, languages, events, the server keeper —
/// for the window, which may name no `tga-*` crate but this one.
pub use tga_sum;

/// Whether something dropped on the window is an export the library can take:
/// a folder, or a file that is one of the exporter's databases.
///
/// Here rather than in the window so the window needs no path to `tga-db`;
/// see `layering.rs`.
pub fn can_open(path: &Path) -> bool {
    path.is_dir() || tga_db::find(path).is_some()
}

/// The report stylesheet's value for one of its colour tokens, as `#rrggbb`.
///
/// The window checks its own palette against this in a test: the report and
/// the window that wrote it are meant to be one design.
pub fn report_colour(name: &str) -> &'static str {
    tga_report::palette::token(name)
}
