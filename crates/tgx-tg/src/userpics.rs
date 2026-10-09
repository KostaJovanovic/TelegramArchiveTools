//! People's profile pictures, into the chat's database.
//!
//! **Not part of Desktop's format**, which paints initials and records no
//! picture of anybody but the account itself. So this writes nowhere but the
//! database — see [`Settings::profile_photos`] — and a Classic folder stays
//! exactly Desktop's.
//!
//! Who: everyone the run met whose picture it can ask for — the roster, and
//! the sender of every message read, so someone who posted and then left is
//! still pictured. What it costs is set by the mode:
//!
//! * **current** — one download per person whose picture this database does
//!   not hold yet. Telegram names the picture someone wears by id in the very
//!   user object the read already received, so a sync that finds everybody
//!   unchanged makes no request at all.
//! * **history** — one `photos.getUserPhotos` per person, plus a download for
//!   each picture not held yet. Skipped for anyone whose current picture a
//!   history pass has already dated: their history was read when they last
//!   changed it, and older pictures do not appear later.
//!
//! Every failure is one person without a picture, counted and reported; none
//! of it can fail the export.

use crate::cancel::Cancel;
use crate::config::{Settings, USERPICS_HISTORY, USERPICS_OFF};
use crate::error::{classify, EnrichError};
use grammers_client::media::Downloadable;
use grammers_client::Client;
use grammers_tl_types as tl;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Enough of a user to ask for their picture with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Face {
    pub user_id: i64,
    pub access_hash: i64,
    /// The picture they wear now.
    pub photo_id: i64,
}

impl Face {
    /// `None` for anyone whose picture cannot be asked for: no picture, a
    /// deleted account, or a *min* user object — Telegram's abbreviated form,
    /// whose access hash is not one we may use.
    pub fn of(u: &tl::types::User) -> Option<Self> {
        let photo = match &u.photo {
            Some(tl::enums::UserProfilePhoto::Photo(p)) => Some(p.photo_id),
            _ => None,
        };
        Self::from_parts(u.id, u.access_hash, u.min, u.deleted, photo)
    }

    /// [`Self::of`] without a `tl::types::User`, which carries fifty fields
    /// and gains more on every grammers bump — the reason `NameBook::learn`
    /// is split from `learn_user`. This is the entry point the tests use.
    fn from_parts(
        user_id: i64,
        access_hash: Option<i64>,
        min: bool,
        deleted: bool,
        photo_id: Option<i64>,
    ) -> Option<Self> {
        if min || deleted {
            return None;
        }
        Some(Self {
            user_id,
            access_hash: access_hash?,
            photo_id: photo_id?,
        })
    }

    fn peer(&self) -> String {
        format!("user{}", self.user_id)
    }
}

/// Everyone met during one chat's run, one entry per person. The latest
/// sighting wins: it is the picture they wear now.
#[derive(Debug, Default, Clone)]
pub struct Faces(HashMap<i64, Face>);

impl Faces {
    pub fn note(&mut self, u: &tl::types::User) {
        self.add(Face::of(u));
    }

    fn add(&mut self, face: Option<Face>) {
        if let Some(face) = face {
            self.0.insert(face.user_id, face);
        }
    }

    pub fn extend(&mut self, other: &Faces) {
        self.0.extend(other.0.iter().map(|(k, v)| (*k, *v)));
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What one pass did, for the run's log line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tally {
    pub people: usize,
    /// Pictures downloaded and stored by this pass.
    pub fetched: usize,
    pub bytes: i64,
    /// Pictures the database already held, so nothing was fetched.
    pub held: usize,
    /// People whose picture, or whose history, could not be had.
    pub failed: usize,
}

/// One picture to fetch: whose, which, when set, whether worn now, and where.
struct Want {
    peer: String,
    photo_id: i64,
    date: Option<i64>,
    current: bool,
    location: tl::enums::InputFileLocation,
}

/// A raw file location, so `iter_download` can take it.
struct Location(tl::enums::InputFileLocation);

impl Downloadable for Location {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(self.0.clone())
    }
}

/// How many times one picture is asked for before it is given up on.
const ATTEMPTS: u32 = 3;
/// How many rate limits one picture may wait out.
const WAITS: u32 = 5;
/// Pictures per `photos.getUserPhotos` page.
const PAGE: i32 = 100;

/// Fetch whatever pictures `faces` wear (or have worn) that `store` lacks, and
/// record every one against `chat_id`.
pub async fn sync(
    client: &Client,
    store: &tgx_archive::Store,
    chat_id: i64,
    faces: &Faces,
    settings: &Settings,
    now: i64,
    cancel: &Cancel,
) -> Result<Tally, tgx_archive::Error> {
    let mut tally = Tally {
        people: faces.len(),
        ..Tally::default()
    };
    if settings.profile_photos == USERPICS_OFF || faces.is_empty() {
        return Ok(tally);
    }
    let history = settings.profile_photos == USERPICS_HISTORY;
    // Whose current picture a history pass has already dated.
    let dated: std::collections::HashSet<(String, i64)> = store
        .userpics_of(chat_id)
        .await?
        .into_iter()
        .filter(|u| u.date.is_some())
        .map(|u| (u.peer, u.photo_id))
        .collect();

    let mut faces: Vec<Face> = faces.0.values().copied().collect();
    faces.sort_by_key(|f| f.user_id);

    let permits = Arc::new(Semaphore::new(settings.download_concurrency.max(1)));
    let mut set = tokio::task::JoinSet::new();
    for face in faces {
        let wants_history = history && !dated.contains(&(face.peer(), face.photo_id));
        let permits = permits.clone();
        let client = client.clone();
        let cancel = cancel.clone();
        set.spawn(async move {
            let _permit = permits.acquire_owned().await;
            if cancel.is_cancelled() {
                return (face, Err("stopped".to_string()));
            }
            let wants = if wants_history {
                history_of(&client, &face).await
            } else {
                Ok(vec![current_of(&face)])
            };
            (face, wants)
        });
    }
    let mut wanted = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((_, Ok(wants))) => wanted.extend(wants),
            Ok((face, Err(why))) => {
                tally.failed += 1;
                log::info!("  no picture for user{}: {why}", face.user_id);
            }
            Err(_) => tally.failed += 1,
        }
    }

    // What is already held is recorded and not fetched again.
    let mut fetch = Vec::new();
    for want in wanted {
        if store
            .has_blob(want.photo_id, tgx_archive::USERPIC_KIND)
            .await?
        {
            store
                .record_userpic(
                    chat_id,
                    &want.peer,
                    want.photo_id,
                    want.date,
                    want.current,
                    now,
                )
                .await?;
            tally.held += 1;
        } else {
            fetch.push(want);
        }
    }

    let mut set = tokio::task::JoinSet::new();
    for want in fetch {
        let permits = permits.clone();
        let client = client.clone();
        let cancel = cancel.clone();
        set.spawn(async move {
            let _permit = permits.acquire_owned().await;
            let bytes = bytes_of(&client, &Location(want.location.clone()), &cancel).await;
            (want, bytes)
        });
    }
    while let Some(joined) = set.join_next().await {
        let Ok((want, bytes)) = joined else {
            tally.failed += 1;
            continue;
        };
        let bytes = match bytes {
            Ok(b) => b,
            Err(why) => {
                tally.failed += 1;
                log::info!("  no picture {} for {}: {why}", want.photo_id, want.peer);
                continue;
            }
        };
        match store
            .put_blob(
                want.photo_id,
                tgx_archive::USERPIC_KIND,
                "image/jpeg",
                None,
                &bytes,
                now,
            )
            .await
        {
            Ok(()) => {}
            Err(tgx_archive::Error::TooLarge { .. }) => {
                tally.failed += 1;
                continue;
            }
            Err(e) => return Err(e),
        }
        store
            .record_userpic(
                chat_id,
                &want.peer,
                want.photo_id,
                want.date,
                want.current,
                now,
            )
            .await?;
        tally.fetched += 1;
        tally.bytes += bytes.len() as i64;
    }
    Ok(tally)
}

/// The picture someone wears now, at the larger of Telegram's two sizes
/// (640 px; the other is 160).
fn current_of(face: &Face) -> Want {
    Want {
        peer: face.peer(),
        photo_id: face.photo_id,
        date: None,
        current: true,
        location: tl::types::InputPeerPhotoFileLocation {
            big: true,
            peer: tl::types::InputPeerUser {
                user_id: face.user_id,
                access_hash: face.access_hash,
            }
            .into(),
            photo_id: face.photo_id,
        }
        .into(),
    }
}

/// Every picture someone still has up, newest first, each at its largest size.
async fn history_of(client: &Client, face: &Face) -> Result<Vec<Want>, String> {
    let user: tl::enums::InputUser = tl::types::InputUser {
        user_id: face.user_id,
        access_hash: face.access_hash,
    }
    .into();
    let mut out = Vec::new();
    loop {
        let request = tl::functions::photos::GetUserPhotos {
            user_id: user.clone(),
            offset: out.len() as i32,
            max_id: 0,
            limit: PAGE,
        };
        let (photos, total) = match client.invoke(&request).await {
            Ok(tl::enums::photos::Photos::Photos(p)) => (p.photos, None),
            Ok(tl::enums::photos::Photos::Slice(p)) => (p.photos, Some(p.count as usize)),
            Err(e) => return Err(classify(&e).to_string()),
        };
        let got = photos.len();
        out.extend(photos.iter().filter_map(|p| match p {
            tl::enums::Photo::Photo(p) => Some(dated(face, p)),
            tl::enums::Photo::Empty(_) => None,
        }));
        if got == 0 || total.is_none_or(|t| out.len() >= t) {
            break;
        }
    }
    // The one they wear now is fetched even when the list leaves it out —
    // it can, for a picture set by you rather than by them.
    if !out.iter().any(|w| w.photo_id == face.photo_id) {
        out.push(current_of(face));
    }
    Ok(out)
}

fn dated(face: &Face, p: &tl::types::Photo) -> Want {
    Want {
        peer: face.peer(),
        photo_id: p.id,
        date: Some(p.date as i64),
        current: p.id == face.photo_id,
        location: tl::types::InputPhotoFileLocation {
            id: p.id,
            access_hash: p.access_hash,
            file_reference: p.file_reference.clone(),
            thumb_size: largest_size(&p.sizes),
        }
        .into(),
    }
}

/// The type letter of a photo's largest size. `"c"` (640 px), which every
/// profile photo has, if the list names nothing usable.
pub(crate) fn largest_size(sizes: &[tl::enums::PhotoSize]) -> String {
    sizes
        .iter()
        .filter_map(|s| match s {
            tl::enums::PhotoSize::Size(s) => Some((s.w as i64 * s.h as i64, s.r#type.clone())),
            tl::enums::PhotoSize::Progressive(s) => {
                Some((s.w as i64 * s.h as i64, s.r#type.clone()))
            }
            _ => None,
        })
        .max_by_key(|(area, _)| *area)
        .map(|(_, t)| t)
        .unwrap_or_else(|| "c".into())
}

/// One picture's bytes, in memory: they go into the database, never to disk.
async fn bytes_of(client: &Client, loc: &Location, cancel: &Cancel) -> Result<Vec<u8>, String> {
    let mut attempt = 0;
    let mut waits = 0;
    loop {
        if cancel.is_cancelled() {
            return Err("stopped".into());
        }
        attempt += 1;
        let mut iter = client.iter_download(loc);
        let mut out = Vec::new();
        let failed: Option<EnrichError> = loop {
            match iter.next().await {
                Ok(Some(chunk)) => out.extend_from_slice(&chunk),
                Ok(None) => break None,
                Err(e) => break Some(classify(&e)),
            }
        };
        match failed {
            None if !out.is_empty() => return Ok(out),
            None if attempt >= ATTEMPTS => return Err("Telegram sent no bytes".into()),
            None => {}
            Some(e) => {
                if let Some(wait) = e.retry_after() {
                    waits += 1;
                    if waits >= WAITS {
                        return Err(format!("still rate limited after {waits} waits"));
                    }
                    crate::engine::sleep_in_slices_until(wait, cancel).await;
                    attempt -= 1;
                    continue;
                }
                if attempt >= ATTEMPTS || e.is_stale() {
                    return Err(e.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_someone_with_a_picture_we_may_ask_for_is_a_face() {
        let face = |min, deleted, photo| Face::from_parts(1, Some(10), min, deleted, photo);
        assert_eq!(
            face(false, false, Some(500)),
            Some(Face {
                user_id: 1,
                access_hash: 10,
                photo_id: 500
            })
        );
        assert_eq!(face(false, false, None), None);
        assert_eq!(
            face(true, false, Some(500)),
            None,
            "a min user's hash is not ours to use"
        );
        assert_eq!(face(false, true, Some(500)), None);
        assert_eq!(Face::from_parts(1, None, false, false, Some(500)), None);

        let mut faces = Faces::default();
        faces.add(face(false, false, Some(500)));
        faces.add(face(false, false, Some(501)));
        faces.add(Face::from_parts(2, Some(20), false, false, None));
        assert_eq!(faces.len(), 1, "one entry per person");
        assert_eq!(faces.0[&1].photo_id, 501, "the latest sighting wins");
    }

    #[test]
    fn the_largest_size_is_asked_for() {
        let size = |t: &str, w: i32| -> tl::enums::PhotoSize {
            tl::types::PhotoSize {
                r#type: t.into(),
                w,
                h: w,
                size: 1,
            }
            .into()
        };
        assert_eq!(
            largest_size(&[size("a", 160), size("c", 640), size("b", 320)]),
            "c"
        );
        assert_eq!(largest_size(&[]), "c");
    }
}
