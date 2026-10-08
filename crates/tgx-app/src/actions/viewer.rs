//! What the Viewer's controls do. **No connection** — every one of these reads
//! a database in the destination, read-only, and nothing else.
//!
//! The answers carry message text, for the window. **None of them is written
//! to the log**, and the failures they report name files and errors, never a
//! message.

use crate::bridge::{Event, Events};
use std::path::PathBuf;
use tgx_tg::viewer::{self, Media};

/// How many hits a search keeps per chat. Enough to page through on screen;
/// past it the answer is "search for something narrower".
pub const SEARCH_LIMIT: usize = 500;

/// The databases in the destination, for the picker.
pub async fn databases(output_dir: String, tx: Events) {
    let found = viewer::databases(std::path::Path::new(&output_dir))
        .await
        .map(|(dbs, _unreadable)| dbs)
        .map_err(|e| format!("could not read {output_dir}: {e}"));
    let _ = tx.send(Event::ViewerDatabases { output_dir, found });
}

/// Every message of one chat, flattened.
pub async fn load(path: PathBuf, chat_id: i64, tx: Events) {
    let rows = viewer::load(&path, chat_id)
        .await
        .map_err(|e| format!("could not read {}: {e}", path.display()));
    let _ = tx.send(Event::ViewerLoaded {
        path,
        chat_id,
        rows,
    });
}

/// One message in full.
pub async fn detail(path: PathBuf, chat_id: i64, id: i64, tx: Events) {
    let detail = viewer::detail(&path, chat_id, id)
        .await
        .map(|d| d.map(Box::new))
        .map_err(|e| e.to_string());
    let _ = tx.send(Event::ViewerDetail {
        chat_id,
        id,
        detail,
    });
}

/// Search one chat — `scope` is its database, its id and its title — or,
/// with no scope, every database in `output_dir`.
pub async fn search(
    output_dir: String,
    scope: Option<(PathBuf, i64, String)>,
    needle: String,
    tx: Events,
) {
    let found = match &scope {
        Some((path, chat_id, title)) => tgx_tg::search::chat(path, *chat_id, &needle, SEARCH_LIMIT)
            .await
            .map(|hits| {
                let mut found = tgx_tg::search::Found::default();
                if !hits.is_empty() {
                    found.chats.push(tgx_tg::search::ChatHits {
                        chat_id: *chat_id,
                        title: title.clone(),
                        path: path.clone(),
                        hits,
                    });
                }
                Box::new(found)
            })
            .map_err(|e| e.to_string()),
        None => tgx_tg::search::across(std::path::Path::new(&output_dir), &needle, SEARCH_LIMIT)
            .await
            .map(Box::new)
            .map_err(|e| format!("could not read {output_dir}: {e}")),
    };
    let _ = tx.send(Event::SearchHits {
        needle,
        scope: scope.map(|(_, id, _)| id),
        found,
    });
}

/// The longest side a picture is kept at. Bubbles are at most a few hundred
/// points wide; a 4000px photo decoded at full size would be 64 MB of texture
/// for a thumbnail's worth of screen.
pub const PICTURE_SIDE: u32 = 720;

/// Decode the pictures of `ids`, one event each, as they are ready.
///
/// Decoding is CPU work, so it runs on the blocking pool; the reads are one
/// read-only open of the database for the whole batch.
pub async fn pictures(path: PathBuf, chat_id: i64, ids: Vec<i64>, tx: Events) {
    let found = viewer::picture_bytes(&path, chat_id, ids.clone()).await;
    let found = match found {
        Ok(found) => found,
        // Nothing could be read: every bubble falls back to its card rather
        // than waiting on a picture that is not coming.
        Err(_) => ids
            .into_iter()
            .map(|id| viewer::Pictured {
                id,
                candidates: Vec::new(),
                has_file: false,
            })
            .collect(),
    };
    for found in found {
        let viewer::Pictured {
            id,
            candidates,
            has_file,
        } = found;
        let image = tokio::task::spawn_blocking(move || decode_first(&candidates))
            .await
            .ok()
            .flatten()
            .map(std::sync::Arc::new);
        let _ = tx.send(Event::ViewerPicture {
            path: path.clone(),
            chat_id,
            id,
            image,
            has_file,
        });
    }
}

/// The first of `candidates` that decodes, scaled down to [`PICTURE_SIDE`].
pub fn decode_first(candidates: &[Vec<u8>]) -> Option<eframe::egui::ColorImage> {
    candidates.iter().find_map(|bytes| {
        let img = image::load_from_memory(bytes).ok()?;
        let img = if img.width().max(img.height()) > PICTURE_SIDE {
            img.thumbnail(PICTURE_SIDE, PICTURE_SIDE)
        } else {
            img
        };
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        Some(eframe::egui::ColorImage::from_rgba_unmultiplied(
            size,
            rgba.as_raw(),
        ))
    })
}

/// A click on a message's media: open the file it carries.
pub async fn open_message_media(path: PathBuf, chat_id: i64, id: i64, tx: Events) {
    match viewer::main_file(&path, chat_id, id).await {
        Ok(Some(media)) => open_media(path, media, tx).await,
        Ok(None) => {
            let _ = tx.send(Event::Status(
                "The database holds no file for this message — the export did not save it".into(),
            ));
        }
        Err(e) => {
            let _ = tx.send(Event::Warn(format!("Could not read the file: {e}")));
        }
    }
}

/// Copy one stored file out to `%TEMP%\tgx-view` and hand it to the system.
///
/// The status line says which file opened; a failure is a warning, naming
/// the file and the reason — a file name is not message text.
pub async fn open_media(path: PathBuf, media: Media, tx: Events) {
    let opened = match viewer::extract(&path, &media).await {
        Ok(file) => super::open_path(&file),
        Err(e) => Err(e),
    };
    let _ = tx.send(match opened {
        Ok(()) => Event::Status(format!("Opened {}", media.name)),
        Err(e) => Event::Warn(format!("Could not open {}: {e}", media.name)),
    });
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
