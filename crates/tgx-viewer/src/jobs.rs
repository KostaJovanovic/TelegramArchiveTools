//! What the Viewer's controls do, off the window's thread. **No connection** —
//! every one of these reads a database, read-only, and nothing else.
//!
//! A [`Job`] goes out through the window's [`crate::Host`] and comes back as
//! one or more [`Reply`]s through whatever sink that window gives [`run`]:
//! the bridge's events in TelegramTools, a channel in TelegramViewer. The
//! work itself is the same code in both.
//!
//! The answers carry message text, for the window. **None of them is written
//! to the log**, and the failures they report name files and errors, never a
//! message.

use crate::data::{self, Database, Detail, Loaded, Media};
use crate::search::{self, Found};
use eframe::egui::ColorImage;
use std::path::PathBuf;
use std::sync::Arc;

/// How many hits a search keeps per chat. Enough to page through on screen;
/// past it the answer is "search for something narrower".
pub const SEARCH_LIMIT: usize = 500;

/// The longest side a picture is kept at. Bubbles are at most a few hundred
/// points wide; a 4000px photo decoded at full size would be 64 MB of texture
/// for a thumbnail's worth of screen.
pub const PICTURE_SIDE: u32 = 720;

/// The side a profile picture is kept at: a few times the userpic circle, so
/// it stays sharp on a high-density screen without holding Telegram's 640.
pub const USERPIC_SIDE: u32 = 96;

/// Something the Viewer wants done off the window's thread.
#[derive(Debug, Clone)]
pub enum Job {
    /// The databases in a folder.
    Databases {
        dir: String,
    },
    /// Every message of one chat, flattened.
    Load {
        path: PathBuf,
        chat_id: i64,
    },
    /// The chat's profile pictures.
    Userpics {
        path: PathBuf,
        chat_id: i64,
    },
    /// One message in full.
    Detail {
        path: PathBuf,
        chat_id: i64,
        id: i64,
    },
    /// These messages' pictures, decoded.
    Pictures {
        path: PathBuf,
        chat_id: i64,
        ids: Vec<i64>,
    },
    /// Search one chat — `scope` is its database, its id and its title — or,
    /// with no scope, every database in `dir`.
    Search {
        dir: String,
        scope: Option<(PathBuf, i64, String)>,
        needle: String,
    },
    /// A click on a message's media: open the file it carries.
    OpenMessageMedia {
        path: PathBuf,
        chat_id: i64,
        id: i64,
    },
    OpenMedia {
        path: PathBuf,
        media: Media,
    },
    SaveMedia {
        path: PathBuf,
        media: Media,
        to: PathBuf,
    },
}

/// What came back. Each carries what it is about, so an answer for a chat
/// no longer open is recognised and dropped.
#[derive(Debug, Clone)]
pub enum Reply {
    /// The databases the Viewer can open. Carries the folder it describes, so
    /// a list of a folder since changed is dropped.
    Databases {
        dir: String,
        found: Result<Vec<Database>, String>,
    },
    /// One chat's rows. Carries which chat, so a slow load of a database the
    /// user has since moved away from cannot replace the one on screen.
    Loaded {
        path: PathBuf,
        chat_id: i64,
        rows: Result<Loaded, String>,
    },
    /// One message in full, for the detail pane. Dropped unless it is still
    /// the selected one.
    Detail {
        chat_id: i64,
        id: i64,
        detail: Result<Option<Box<Detail>>, String>,
    },
    /// One message's picture, decoded and scaled for the screen — or `None`
    /// when the database holds nothing that decodes, so the bubble falls back
    /// to a card. Carries the database, so a picture for a chat since closed
    /// is dropped.
    Picture {
        path: PathBuf,
        chat_id: i64,
        id: i64,
        image: Option<Arc<ColorImage>>,
        /// The file itself is in the database, picture or not.
        has_file: bool,
    },
    /// The open chat's profile pictures, decoded, by sender (`"user1234"`).
    /// Carries the database, so pictures for a chat since closed are dropped.
    Userpics {
        path: PathBuf,
        chat_id: i64,
        pics: Vec<(String, Arc<ColorImage>)>,
    },
    /// What a search found. `scope` is the chat searched, or `None` for every
    /// database in the folder; with `needle`, it is how an answer to a
    /// question nobody is asking any more is recognised and dropped.
    Hits {
        needle: String,
        scope: Option<i64>,
        found: Result<Box<Found>, String>,
    },
    /// A line for the status bar: a file name or a count, never message text.
    Status(String),
    /// A failure worth keeping where warnings are kept.
    Warn(String),
}

/// Do `job`, handing each answer to `send` as it is ready. Must run inside a
/// tokio runtime with a blocking pool: the store blocks underneath.
pub async fn run(job: Job, send: impl Fn(Reply)) {
    match job {
        Job::Databases { dir } => {
            let found = data::databases(std::path::Path::new(&dir))
                .await
                .map(|(dbs, _unreadable)| dbs)
                .map_err(|e| format!("could not read {dir}: {e}"));
            send(Reply::Databases { dir, found });
        }
        Job::Load { path, chat_id } => {
            let rows = data::load(&path, chat_id)
                .await
                .map_err(|e| format!("could not read {}: {e}", path.display()));
            send(Reply::Loaded {
                path,
                chat_id,
                rows,
            });
        }
        Job::Userpics { path, chat_id } => userpics(path, chat_id, &send).await,
        Job::Detail { path, chat_id, id } => {
            let detail = data::detail(&path, chat_id, id)
                .await
                .map(|d| d.map(Box::new))
                .map_err(|e| e.to_string());
            send(Reply::Detail {
                chat_id,
                id,
                detail,
            });
        }
        Job::Pictures { path, chat_id, ids } => pictures(path, chat_id, ids, &send).await,
        Job::Search { dir, scope, needle } => {
            let found = find(&dir, &scope, &needle).await;
            send(Reply::Hits {
                needle,
                scope: scope.map(|(_, id, _)| id),
                found,
            });
        }
        Job::OpenMessageMedia { path, chat_id, id } => {
            match data::main_file(&path, chat_id, id).await {
                Ok(Some(media)) => send(open_media(path, media).await),
                Ok(None) => send(Reply::Status(
                    "The database holds no file for this message — the export did not save it"
                        .into(),
                )),
                Err(e) => send(Reply::Warn(format!("Could not read the file: {e}"))),
            }
        }
        Job::OpenMedia { path, media } => send(open_media(path, media).await),
        Job::SaveMedia { path, media, to } => send(match data::save(&path, &media, &to).await {
            Ok(()) => Reply::Status(format!("Saved {}", to.display())),
            Err(e) => Reply::Warn(format!("Could not save {}: {e}", media.name)),
        }),
    }
}

async fn find(
    dir: &str,
    scope: &Option<(PathBuf, i64, String)>,
    needle: &str,
) -> Result<Box<Found>, String> {
    match scope {
        Some((path, chat_id, title)) => search::chat(path, *chat_id, needle, SEARCH_LIMIT)
            .await
            .map(|hits| {
                let mut found = Found::default();
                if !hits.is_empty() {
                    found.chats.push(search::ChatHits {
                        chat_id: *chat_id,
                        title: title.clone(),
                        path: path.clone(),
                        hits,
                    });
                }
                Box::new(found)
            })
            .map_err(|e| e.to_string()),
        None => search::across(std::path::Path::new(dir), needle, SEARCH_LIMIT)
            .await
            .map(Box::new)
            .map_err(|e| format!("could not read {dir}: {e}")),
    }
}

/// Decode the pictures of `ids`, one reply each, as they are ready.
///
/// Decoding is CPU work, so it runs on the blocking pool; the reads are one
/// read-only open of the database for the whole batch.
async fn pictures(path: PathBuf, chat_id: i64, ids: Vec<i64>, send: &impl Fn(Reply)) {
    let found = match data::picture_bytes(&path, chat_id, ids.clone()).await {
        Ok(found) => found,
        // Nothing could be read: every bubble falls back to its card rather
        // than waiting on a picture that is not coming. The reason goes to the
        // log — a file name and a database error, never a message — or a
        // Viewer with no pictures at all leaves nothing to go on.
        Err(e) => {
            log::warn!("viewer pictures: {}: {e}", path.display());
            ids.into_iter()
                .map(|id| data::Pictured {
                    id,
                    candidates: Vec::new(),
                    has_file: false,
                })
                .collect()
        }
    };
    for found in found {
        let data::Pictured {
            id,
            candidates,
            has_file,
        } = found;
        let image = tokio::task::spawn_blocking(move || decode_first(&candidates))
            .await
            .ok()
            .flatten()
            .map(Arc::new);
        send(Reply::Picture {
            path: path.clone(),
            chat_id,
            id,
            image,
            has_file,
        });
    }
}

/// The open chat's profile pictures, one per person, decoded together on the
/// blocking pool. A database that cannot be read, or holds none, sends none —
/// the bubbles keep their initials.
async fn userpics(path: PathBuf, chat_id: i64, send: &impl Fn(Reply)) {
    let found = match data::userpics(&path, chat_id).await {
        Ok(found) => found,
        // Logged for the same reason as the pictures above.
        Err(e) => {
            log::warn!("viewer userpics: {}: {e}", path.display());
            return;
        }
    };
    if found.is_empty() {
        return;
    }
    let pics = tokio::task::spawn_blocking(move || {
        found
            .into_iter()
            .filter_map(|(peer, bytes)| {
                let img = decode_scaled(&[bytes], USERPIC_SIDE)?;
                Some((peer, Arc::new(img)))
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    send(Reply::Userpics {
        path,
        chat_id,
        pics,
    });
}

/// The first of `candidates` that decodes, scaled down to [`PICTURE_SIDE`].
pub fn decode_first(candidates: &[Vec<u8>]) -> Option<ColorImage> {
    decode_scaled(candidates, PICTURE_SIDE)
}

/// The first of `candidates` that decodes, its longer side at most `side`.
fn decode_scaled(candidates: &[Vec<u8>], side: u32) -> Option<ColorImage> {
    candidates.iter().find_map(|bytes| {
        let img = image::load_from_memory(bytes).ok()?;
        let img = if img.width().max(img.height()) > side {
            img.thumbnail(side, side)
        } else {
            img
        };
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        Some(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
    })
}

/// Copy one stored file out to `%TEMP%\tgx-view` and hand it to the system
/// — or, for a type that could run ([`crate::open::openable`]), open its
/// folder.
///
/// The status line says which file opened; a failure is a warning, naming
/// the file and the reason — a file name is not message text.
async fn open_media(path: PathBuf, media: Media) -> Reply {
    let opens = crate::open::openable(&media.name);
    let opened = match data::extract(&path, &media).await {
        Ok(file) if opens => crate::open::path(&file),
        Ok(file) => crate::open::path(file.parent().unwrap_or(&file)),
        Err(e) => Err(e),
    };
    match opened {
        Ok(()) if opens => Reply::Status(format!("Opened {}", media.name)),
        Ok(()) => Reply::Status(format!(
            "Showed {} in its folder: a file of that type could be a program, so it is \
             not opened from here",
            media.name
        )),
        Err(e) => Reply::Warn(format!("Could not open {}: {e}", media.name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn the_first_candidate_that_decodes_is_the_picture_and_a_large_one_is_scaled() {
        // An animated sticker's own bytes do not decode; its thumbnail does.
        let pic = decode_first(&[b"not an image".to_vec(), png(40, 20)]).unwrap();
        assert_eq!(pic.size, [40, 20]);
        let big = decode_first(&[png(2000, 1000)]).unwrap();
        assert_eq!(big.size, [PICTURE_SIDE as usize, PICTURE_SIDE as usize / 2]);
        assert!(decode_first(&[]).is_none());
    }
}
