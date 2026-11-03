//! **The Viewer: one chat's database, read-only, on screen.** Shared by
//! TelegramTools' window and by `TelegramViewer.exe`, the small program the
//! export folder can carry so the databases in it can be read on a machine
//! with nothing else installed.
//!
//! - [`data`] reads a database into rows a frame can paint (was
//!   `tgx_tg::viewer`).
//! - [`search`] finds text in one database or every one in a folder.
//! - [`jobs`] is that work off the window's thread, as [`Job`] in and
//!   [`Reply`] out.
//! - [`Viewer`] is the screen, given a [`Host`] by the window it sits in.
//! - [`bubbles`] draws the chat the way Telegram draws one.
//!
//! **No Telegram client in here, and none may be added** —
//! `tgx-parity/tests/layering.rs` holds it. `TelegramViewer.exe` is small
//! because of it, and it is what lets a folder of databases be read with the
//! account nowhere near it.

pub mod bubbles;
pub mod data;
pub mod jobs;
pub mod open;
pub mod search;
mod view;

pub use jobs::{Job, Reply};
/// Re-exported for the same reason as [`search::Hit`]: the windows reach
/// `tgx-archive` through this crate or not at all.
pub use tgx_archive::fold;
pub use view::{Asked, Filter, Head, Host, Viewer};
