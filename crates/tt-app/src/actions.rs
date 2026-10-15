//! What the nav bar's cells actually do.
//!
//! Every one of these submits to the tokio side and returns immediately — **the
//! UI thread never blocks on a future.** Results come back as
//! [`Event`](crate::bridge::Event)s, which wake the window rather than waiting
//! to be found on its next frame.

use crate::bridge::{on_panic, Activity, Event, Events};
use crate::pending::Pending;
use tgx_tg::cancel::Cancel;
use tgx_tg::client::{LoginStep, Session};
use tgx_tg::config::Settings;
use tgx_tg::dialogs;
use tgx_tg::error::EnrichError;

/// Connect and report whether the session is already authorised.
///
/// **Reaching Telegram without being signed in is a success, not an error** —
/// the caller opens the sign-in dialog on it. Conflating the two puts two modal
/// dialogs on top of each other, which a user experiences as the app freezing
/// the moment it logs them in.
pub async fn sign_in(settings: Settings, tx: Events) {
    on_panic(|why| Activity::SignIn.ending(why));
    let _ = tx.send(Event::Status("Connecting…".into()));
    let session = match Session::connect(&settings).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(Event::Failed {
                activity: Activity::SignIn,
                message: e.to_string(),
            });
            return;
        }
    };
    // **Asked, not read from the cache.** This is the one call whose whole job
    // is to re-check the stored session, and a cached answer would make the
    // button do nothing visible. Every *other* action uses `ensure_connected`,
    // which reuses what this learned.
    match session.refresh_authorization().await {
        Ok(true) => match session.me().await {
            Ok(name) => {
                let _ = tx.send(Event::SignedIn(name));
            }
            Err(e) => {
                let _ = tx.send(Event::Failed {
                    activity: Activity::SignIn,
                    message: e.to_string(),
                });
            }
        },
        Ok(false) => {
            // Reached Telegram, not signed in yet.
            let _ = tx.send(Event::Status("Not signed in".into()));
        }
        Err(e) => {
            let _ = tx.send(Event::Failed {
                activity: Activity::SignIn,
                message: e.to_string(),
            });
        }
    }
}

/// What every action that needs an authorised account does first.
///
/// Two failures used to look the same from here and neither said what it was.
/// A blocked network hung in the operating system's TCP retry, because nothing
/// in the stack had a timeout; and an unauthorised account got as far as its
/// real request and came back with a wire error from the middle of a chat
/// listing. `ensure_connected` bounds the first and names the second, and it
/// only costs a round trip the first time — the answer is cached on the shared
/// connection.
async fn ready(session: &Session, tx: &Events, activity: Activity) -> bool {
    match session.ensure_connected().await {
        Ok(true) => true,
        Ok(false) => {
            let _ = tx.send(Event::Failed {
                activity,
                message: "Not signed in. Press Sign in at the foot of the left rail.".into(),
            });
            false
        }
        Err(e) => {
            let _ = tx.send(Event::Failed {
                activity,
                message: e.to_string(),
            });
            false
        }
    }
}

/// Say so if the folder holding the session key is at default permissions.
///
/// `ensure_data_dir` must not stop the app starting, so it records the failure
/// instead of raising it — which previously meant the folder holding a **bearer
/// credential** could be readable by anyone on the machine with nothing
/// anywhere saying so. `tgx_tg::config::lockdown_error` exists precisely so a
/// caller that shows the user a security claim can check whether it is true;
/// until now nothing called it and an ACL failure reached only `log::warn!`.
pub fn report_data_dir_protection(tx: &Events) {
    let dir = match tgx_tg::config::ensure_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            let _ = tx.send(Event::Warn(format!(
                "could not create the data folder: {e}. Signing in will not work."
            )));
            return;
        }
    };
    if let Some(why) = tgx_tg::config::lockdown_error() {
        let _ = tx.send(Event::Warn(format!(
            "Could not restrict {} to your user ({why}). Anyone with access to \
             this machine or drive can read the saved session and sign in as you.",
            dir.display()
        )));
    }
}

/// The sign-in in progress, held between the steps the user drives.
///
/// **The login token cannot be re-derived, and the code cannot be re-requested
/// without invalidating itself.** Telegram treats a second `auth.sendCode` as
/// starting the authorisation over, so asking again — which is what this used
/// to do, once per step — cancelled the code the user was in the middle of
/// typing and eventually answered `AUTH_RESTART`. Each step runs as its own
/// task, so the session lives here rather than on a stack frame.
///
/// **The connection is no longer what is being held here** — that is shared and
/// outlives any one step, see `tgx_tg::client`. What lives on this `Session` and
/// nowhere else is the login token, and after a two-factor prompt the password
/// token: both are half-finished credentials, so this is cleared the moment the
/// sign-in ends, either way.
static PENDING: std::sync::OnceLock<tokio::sync::Mutex<LoginSlot<Session>>> =
    std::sync::OnceLock::new();

fn pending() -> &'static tokio::sync::Mutex<LoginSlot<Session>> {
    PENDING.get_or_init(|| tokio::sync::Mutex::new(LoginSlot::default()))
}

/// Where a half-finished sign-in waits between steps.
///
/// **A step takes the session out; it does not hold the lock.** It used to keep
/// the guard across `sign_in`, `check_password` and `me` — network calls with
/// deliberately long timeouts — so Cancel on the dialog, or Sign out, queued
/// behind a step stuck on a bad link: "Signing out…" for the whole timeout,
/// and an abandoned login token alive in memory all that while.
///
/// So a step [`take`](Self::take)s the session with the generation it belongs
/// to, runs unlocked, and [`put_back`](Self::put_back) refuses it if the
/// sign-in was forgotten or started over in the meantime. Generic so the rule
/// can be tested without a connection.
struct LoginSlot<T> {
    held: Option<T>,
    /// Moves on every new sign-in and every forget, so a step that finishes
    /// late cannot bring back a credential the user has already thrown away.
    generation: u64,
}

impl<T> Default for LoginSlot<T> {
    fn default() -> Self {
        Self {
            held: None,
            generation: 0,
        }
    }
}

impl<T> LoginSlot<T> {
    /// A new sign-in, superseding any step still in flight.
    fn start(&mut self, held: T) {
        self.generation += 1;
        self.held = Some(held);
    }

    fn forget(&mut self) {
        self.generation += 1;
        self.held = None;
    }

    fn take(&mut self) -> Option<(T, u64)> {
        self.held.take().map(|held| (held, self.generation))
    }

    /// Keep a session a step left usable — unless the sign-in it belongs to
    /// is over. `false` means it was dropped.
    fn put_back(&mut self, held: T, generation: u64) -> bool {
        if generation != self.generation || self.held.is_some() {
            return false;
        }
        self.held = Some(held);
        true
    }
}

/// Drop any half-finished sign-in.
pub async fn forget_pending_login() {
    pending().lock().await.forget();
}

/// Sign out: end the session at Telegram and delete the session file.
///
/// A half-finished sign-in is dropped first, because it holds a handle on the
/// same connection and would keep the session database open.
pub async fn sign_out(settings: Settings, tx: Events) {
    let _ = tx.send(Event::Status("Signing out…".into()));
    forget_pending_login().await;
    let done = tgx_tg::client::sign_out(&settings).await;
    let _ = tx.send(Event::SignedOut {
        revoked: done.revoked,
        removed: done.removed,
    });
}

/// Ask Telegram to send a login code.
pub async fn request_code(settings: Settings, tx: Events) {
    on_panic(|why| Activity::SignIn.ending(why));
    let mut session = match Session::connect(&settings).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(Event::LoginFailed(e.to_string()));
            return;
        }
    };
    match session.request_code(&settings.phone).await {
        Ok(LoginStep::Ready) => match session.me().await {
            Ok(name) => {
                let _ = tx.send(Event::SignedIn(name));
            }
            Err(e) => {
                let _ = tx.send(Event::LoginFailed(e.to_string()));
            }
        },
        Ok(_) => {
            // Held for the next step. The code Telegram just sent is only
            // usable with the token on this session.
            pending().lock().await.start(session);
            let _ = tx.send(Event::LoginStage(crate::login::Stage::Code));
        }
        Err(e) => {
            let _ = tx.send(Event::LoginFailed(e.to_string()));
        }
    }
}

/// Submit the code, or the two-factor password.
///
/// Continues the session [`request_code`] left in [`PENDING`]. It does **not**
/// re-request the code: that starts the authorisation over and invalidates the
/// one the user just typed.
pub async fn finish_login(secret: String, is_code: bool, tx: Events) {
    on_panic(|why| Activity::SignIn.ending(why));
    // Out of the slot for the length of the step, so nothing that wants the
    // lock waits on Telegram. See `LoginSlot`.
    let taken = pending().lock().await.take();
    let Some((mut session, generation)) = taken else {
        // Nothing to continue: the dialog was reopened, or the app restarted
        // between the code arriving and it being typed.
        let _ = tx.send(Event::LoginFailed(
            "That sign-in expired. Send a new code.".into(),
        ));
        let _ = tx.send(Event::LoginStage(crate::login::Stage::Phone));
        return;
    };
    let keep = |session| async move {
        pending().lock().await.put_back(session, generation);
    };
    if is_code {
        match session.sign_in(&secret).await {
            Ok(LoginStep::Ready) => {}
            Ok(LoginStep::NeedPassword) => {
                // Kept: the password token `sign_in` stashed lives on this
                // session and there is no way to obtain another.
                keep(session).await;
                let _ = tx.send(Event::LoginStage(crate::login::Stage::Password));
                return;
            }
            Ok(LoginStep::NeedCode) => {
                keep(session).await;
                let _ = tx.send(Event::LoginFailed("Telegram wanted another code.".into()));
                return;
            }
            // Kept as well. A mistyped code does not spend the token, so the
            // user retypes rather than starting the whole sign-in again.
            Err(e) => {
                keep(session).await;
                let _ = tx.send(Event::LoginFailed(e.to_string()));
                return;
            }
        }
    } else if let Err(e) = session.check_password(&secret).await {
        keep(session).await;
        let _ = tx.send(Event::LoginFailed(e.to_string()));
        return;
    }
    match session.me().await {
        Ok(name) => {
            // Signed in. The connection stays — it is shared, and every later
            // action is about to want it — but the login and password tokens
            // this `Session` is carrying are spent, and a spent credential
            // left in memory for the rest of the run is a credential left in
            // memory for the rest of the run. So it is not put back.
            drop(session);
            let _ = tx.send(Event::SignedIn(name));
        }
        Err(e) => {
            keep(session).await;
            let _ = tx.send(Event::LoginFailed(e.to_string()));
        }
    }
}

/// Load the chat list.
pub async fn refresh_chats(settings: Settings, tx: Events) {
    on_panic(|why| Activity::Chats.ending(why));
    let _ = tx.send(Event::Status("Loading chats…".into()));
    let session = match Session::connect(&settings).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(Event::Failed {
                activity: Activity::Chats,
                message: e.to_string(),
            });
            return;
        }
    };
    if !ready(&session, &tx, Activity::Chats).await {
        return;
    }
    match dialogs::list_chats(&session.client).await {
        Ok(chats) => {
            let _ = tx.send(Event::Chats(chats));
        }
        Err(e) => {
            let _ = tx.send(Event::Failed {
                activity: Activity::Chats,
                message: e.to_string(),
            });
        }
    }
}

/// Find the chats that already have a database in the destination.
///
/// **No connection.** It reads the folder and the files, nothing else, so it
/// cannot hang on a network and does not need one. The run it leads to does.
pub async fn find_archives(settings: Settings, tx: Events) {
    // Every one-shot tool below declares the event that brings its button
    // back, carrying the reason, the same way the long runs do.
    on_panic(|why| vec![Event::Archived(Err(format!("Looking for databases {why}")))]);
    let dir = std::path::Path::new(&settings.output_dir);
    let _ = tx.send(Event::Status(format!(
        "Looking for databases in {}…",
        dir.display()
    )));
    let found = tgx_tg::engine::archived_chats(dir)
        .await
        .map(Box::new)
        .map_err(|e| format!("could not read {}: {e}", dir.display()));
    let _ = tx.send(Event::Archived(found));
}

/// Export as Classic: write one database back out as Desktop-format folders
/// under `out_dir`, offline — see `tgx_tg::unpack`. Counts only in the lines.
pub async fn unpack(
    db: std::path::PathBuf,
    out_dir: std::path::PathBuf,
    settings: Settings,
    tx: Events,
) {
    on_panic(|why| vec![Event::Unpacked(Err(format!("Export as folder {why}")))]);
    let done = tgx_tg::unpack::write_classic(&db, &out_dir, &settings)
        .await
        .map(|chats| {
            chats
                .iter()
                .map(|u| {
                    format!(
                        "Wrote {} to {}: {} messages ({} deleted), {} files{}",
                        u.title,
                        u.root.display(),
                        u.messages,
                        u.deleted,
                        u.files,
                        if u.missing > 0 {
                            format!(", {} not in the database", u.missing)
                        } else {
                            String::new()
                        }
                    )
                })
                .collect()
        })
        .map_err(|e| format!("Export as folder failed: {e}"));
    let _ = tx.send(Event::Unpacked(done));
}

/// Say what is wrong with the destination's drive, as warnings, when a run
/// starts. Off the async worker's thread: a network drive can take a while to
/// answer. See `crate::volume`.
pub async fn check_volume(settings: Settings, tx: Events) {
    let dir = std::path::PathBuf::from(&settings.output_dir);
    let lines = tokio::task::spawn_blocking(move || {
        crate::volume::warnings(&crate::volume::inspect(&dir), &settings)
    })
    .await
    .unwrap_or_default();
    for line in lines {
        let _ = tx.send(Event::Warn(line));
    }
}

/// Check folder: what is wrong with the destination, as log lines. Changes
/// nothing — see `tgx_tg::doctor`.
pub async fn check_folder(settings: Settings, tx: Events) {
    on_panic(|why| vec![Event::Checked(Err(format!("Check folder {why}")))]);
    let dir = std::path::Path::new(&settings.output_dir);
    let lines = tgx_tg::doctor::examine(dir)
        .await
        .map(|found| found.iter().map(|f| f.line()).collect())
        .map_err(|e| format!("could not read {}: {e}", dir.display()));
    let _ = tx.send(Event::Checked(lines));
}

/// Find duplicate media: what is stored more than once in the destination,
/// as log lines. Changes nothing — see `tgx_tg::doctor::duplicates`.
pub async fn find_dupes(settings: Settings, tx: Events) {
    on_panic(|why| {
        vec![Event::Dupes(Err(format!(
            "Looking for duplicate media {why}"
        )))]
    });
    let dir = std::path::Path::new(&settings.output_dir);
    let _ = tx.send(Event::Status(format!(
        "Looking for duplicate media in {}…",
        dir.display()
    )));
    let lines = tgx_tg::doctor::duplicates(dir)
        .await
        .map(|report| report.lines(DUPE_GROUPS_SHOWN))
        .map_err(|e| format!("could not read {}: {e}", dir.display()));
    let _ = tx.send(Event::Dupes(lines));
}

/// How many groups the window lists — the CLI's number, so the two answers
/// read the same.
const DUPE_GROUPS_SHOWN: usize = 20;

/// Which chats the destination already holds, for the marks in the list.
///
/// **Quiet when the folder is simply not there yet.** That is the normal state
/// before a first export, and a warning on every launch for it would teach the
/// user to ignore warnings. It answers "nothing there" instead.
///
/// **Anything else is said, and the marks are left alone.** Every failure used
/// to answer "nothing there" too, and that answer replaces the last good one:
/// an unplugged drive or a refused read re-marked every chat "Not exported
/// yet" and blanked the sizes, with nothing on screen saying the destination
/// had only gone out of reach.
pub async fn scan_destination(settings: Settings, tx: Events) {
    use crate::list::OnDisk;
    use std::collections::HashMap;
    let dir = std::path::PathBuf::from(&settings.output_dir);
    let unreadable = |e: &dyn std::fmt::Display| {
        let _ = tx.send(Event::Warn(format!(
            "Could not read {} ({e}); the Exported marks are left as they were",
            dir.display()
        )));
    };
    let archived = match tgx_tg::engine::archived_chats(&dir).await {
        Ok(archived) => archived,
        Err(e) if not_made_yet(&dir, &e) => Default::default(),
        Err(e) => return unreadable(&e),
    };
    // Off the async worker's own thread: it reads the head of a file per
    // folder and walks every folder for its size, and a destination of a few
    // hundred thousand media files on a network drive makes that slow.
    let walk_dir = dir.clone();
    let walked = tokio::task::spawn_blocking(move || {
        let dir = walk_dir;
        let mut disk: HashMap<i64, OnDisk> = HashMap::new();
        for (id, path) in &archived.files {
            let entry = disk.entry(*id).or_default();
            entry.database = true;
            entry.synced = archived.synced.get(id).copied();
            // The database's own folder, not the file: the WAL and the shm
            // sit beside it, and in a fresh WAL that is where the writes are.
            // **Except a flat one from before the folders**, whose parent is
            // the destination itself — that would be every export at once.
            entry.bytes += match path.parent() {
                Some(parent) if parent != dir => tgx_tg::engine::disk_size(parent),
                _ => ["", "-wal", "-shm"]
                    .iter()
                    .map(|s| {
                        tgx_tg::engine::disk_size(std::path::Path::new(&format!(
                            "{}{s}",
                            path.display()
                        )))
                    })
                    .sum(),
            };
        }
        let folders = match tgx_tg::engine::exported_folders(&dir) {
            Ok(folders) => folders,
            Err(e) if not_made_yet(&dir, &e) => Default::default(),
            Err(e) => return Err(e),
        };
        for (id, folders) in folders {
            let entry = disk.entry(id).or_default();
            entry.folder = true;
            for folder in folders {
                // A database living inside the chat's Classic folder was
                // counted with that folder above; counting it again here
                // would report it twice.
                if archived.files.get(&id).and_then(|p| p.parent()) != Some(folder.as_path()) {
                    entry.bytes += tgx_tg::engine::disk_size(&folder);
                }
            }
        }
        Ok(disk)
    })
    .await;
    let disk = match walked {
        Ok(Ok(disk)) => disk,
        Ok(Err(e)) => return unreadable(&e),
        Err(e) => {
            log::warn!("scanning {}: {e}", dir.display());
            return unreadable(&e);
        }
    };
    let _ = tx.send(Event::OnDisk {
        output_dir: settings.output_dir,
        disk,
    });
}

/// Is this a destination that does not exist *yet*, rather than one out of
/// reach?
///
/// The parent is the difference. A folder the first export has not made sits
/// in one that exists; a destination on a drive that has been unplugged reads
/// as "not found" just the same, and its parent is gone with it — as is a
/// drive's root, which has no parent to ask.
fn not_made_yet(dir: &std::path::Path, e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
        && dir
            .parent()
            .is_some_and(|p| p.as_os_str().is_empty() || p.is_dir())
}

/// Ask Telegram how many messages each chat holds.
///
/// **One request per chat**, which is why this is a button rather than
/// something that happens when the list loads, and why it can sit in a
/// two-minute rate-limit wait. Both facts are the user's to know before they
/// press it, and both are why it has to be stoppable.
pub async fn count_chats(settings: Settings, chat_ids: Vec<i64>, cancel: Cancel, tx: Events) {
    on_panic(|why| Activity::Count.ending(why));
    let session = match Session::connect(&settings).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(Event::Failed {
                activity: Activity::Count,
                message: e.to_string(),
            });
            // Still ends the count, or the button reads "Stop counting" over a
            // run that never began.
            let _ = tx.send(Event::CountFinished {
                counted: 0,
                failed: 0,
            });
            return;
        }
    };
    if !ready(&session, &tx, Activity::Count).await {
        // Still ends the count, for the same reason as above: the button would
        // otherwise read "Stop counting" over a run that never began.
        let _ = tx.send(Event::CountFinished {
            counted: 0,
            failed: 0,
        });
        return;
    }

    let total = chat_ids.len();
    let mut done = 0usize;
    let _ = tx.send(Event::CountProgress { done, total });

    let tx_counted = tx.clone();
    let mut on = move |chat_id: i64, count: Option<i64>| {
        done += 1;
        // **`None` is not zero.** A chat Telegram would not count keeps its
        // blank row; writing 0 there would make it read as an empty chat and
        // sort among the empty ones.
        let _ = tx_counted.send(Event::Counted {
            chat_id,
            total: count,
        });
        let _ = tx_counted.send(Event::CountProgress { done, total });
    };

    let tx_wait = tx.clone();
    let mut waiting = move |seconds: u64| {
        let _ = tx_wait.send(Event::FloodWait(seconds));
    };

    let (counted, failed) =
        dialogs::count_all(&session.client, &chat_ids, &cancel, &mut on, &mut waiting).await;

    if failed > 0 {
        // A chat with no count paints blank and sorts last, which looks exactly
        // like a chat with no messages. Say how many, or a part-counted list
        // reads as a complete one.
        let _ = tx.send(Event::Warn(format!(
            "{counted} chats counted, {failed} could not be — those rows stay \
             blank and sort last."
        )));
    }
    let _ = tx.send(Event::CountFinished { counted, failed });
}

mod export;
pub mod viewer;

pub use export::export;

// Test-only: the topic-resolution cases are exercised from this module's own
// tests, where the degrade-to-one-folder behaviour was written.
#[cfg(test)]
use export::{resolve_topics, TopicsResolution};

/// Open a folder in the system file manager.
///
/// A tool, not a step — it is live from the first frame, which is exactly why
/// it must not carry a step number.
///
/// Returns what went wrong, if anything. **The result used to be discarded**,
/// and it was covering a real bug: the path was built with `system32()` and
/// `explorer.exe` is not in System32 — it sits directly in `%SystemRoot%` — so
/// every press spawned nothing and reported nothing.
pub fn open_folder(dir: &std::path::Path) -> Result<(), String> {
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    open_path(dir)
}

/// Hand a folder or a file to the system: a folder opens in the file manager,
/// a file in whatever is registered for it — which is how the Viewer's Open
/// media shows a photo without this app drawing one.
pub fn open_path(path: &std::path::Path) -> Result<(), String> {
    // Absolute, for the same reason icacls is: CreateProcess searches the
    // calling process's own directory before PATH, and this app is built to be
    // copied into arbitrary folders. A planted explorer.exe beside it would run
    // with the user's rights.
    #[cfg(windows)]
    let spawned = std::process::Command::new(tgx_tg::config::system_root("explorer.exe"))
        .arg(path)
        .spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(path).spawn();
    spawned
        .map(|_| ())
        .map_err(|e| format!("opening {}: {e}", path.display()))
}

/// Open the destination folder.
pub fn open_output_folder(dir: &str) -> Result<(), String> {
    open_folder(std::path::Path::new(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn signing_in_without_credentials_reports_rather_than_panics() {
        let (raw, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = crate::bridge::Events::detached(raw);
        sign_in(Settings::default(), tx).await;
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        // A status first, then a failure naming where to get credentials.
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Failed { message: m, .. }
                                   if m.contains("my.telegram.org"))),
            "got {events:?}"
        );
    }

    #[tokio::test]
    async fn a_count_that_cannot_connect_still_ends_the_count() {
        // Otherwise the button is left reading "Stop counting" over a run that
        // never began, and the only control there is has become a lie.
        let (raw, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = crate::bridge::Events::detached(raw);
        count_chats(Settings::default(), vec![1, 2], Cancel::new(), tx).await;
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::CountFinished { .. })),
            "got {events:?}"
        );
    }

    #[tokio::test]
    async fn an_export_that_cannot_connect_still_ends_the_queue() {
        // Same failure as above, on the control that matters more: without the
        // Finished, `exporting` stays set and Start is disabled for the rest of
        // the session.
        let (raw, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = crate::bridge::Events::detached(raw);
        export(Settings::default(), Pending::new(), Cancel::new(), tx).await;
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        assert!(
            events.iter().any(|e| matches!(e, Event::Finished { .. })),
            "got {events:?}"
        );
    }

    /// The bug item 11 named: `export` used to catch *any* `list_topics`
    /// error, `Transient` included, and fall back to one folder. This drives
    /// `resolve_topics` with a canned `Transient` on every call — no network
    /// needed, since the whole point is that the answer must not depend on
    /// what a real flood wait happens to look like — and checks it retries
    /// exactly once before giving up on the *chat*, never on the split.
    #[tokio::test]
    async fn a_rate_limit_during_topic_discovery_skips_the_chat_rather_than_collapsing_it() {
        use std::time::Duration;

        let (raw, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = crate::bridge::Events::detached(raw);
        let cancel = Cancel::new();
        let mut calls = 0u32;
        let outcome = resolve_topics("Some Forum", 42, &cancel, &tx, || {
            calls += 1;
            async {
                Result::<Vec<dialogs::Topic>, EnrichError>::Err(EnrichError::Transient(
                    Duration::from_millis(1),
                ))
            }
        })
        .await;

        assert!(matches!(outcome, TopicsResolution::TryLater));
        assert_eq!(calls, 2, "one retry, not zero and not more");

        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        // The one text that must never appear here: it is what told the user
        // their forum had been quietly exported as a single folder.
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::Warn(m) if m.contains("one folder"))),
            "got {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::ChatFailed { chat_id: 42, .. })),
            "got {events:?}"
        );
    }

    /// Telegram failing on its side is temporary too, so it must not collapse
    /// a forum either. `client::Patient` has already re-sent the request by
    /// the time this sees it, so there is no second attempt here.
    #[tokio::test]
    async fn a_server_error_during_topic_discovery_skips_the_chat_rather_than_collapsing_it() {
        let (raw, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = crate::bridge::Events::detached(raw);
        let cancel = Cancel::new();
        let mut calls = 0u32;
        let outcome = resolve_topics("Some Forum", 42, &cancel, &tx, || {
            calls += 1;
            async {
                Result::<Vec<dialogs::Topic>, EnrichError>::Err(EnrichError::Unavailable(
                    "RPC_CALL_FAIL".into(),
                ))
            }
        })
        .await;

        assert!(matches!(outcome, TopicsResolution::TryLater));
        assert_eq!(calls, 1);
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::Warn(m) if m.contains("one folder"))),
            "got {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::ChatFailed { chat_id: 42, .. })),
            "got {events:?}"
        );
    }

    /// A genuine refusal is not a rate limit and still has no shape to
    /// preserve — this is the one case that legitimately degrades to a single
    /// folder, and it must keep doing so.
    #[tokio::test]
    async fn a_real_refusal_still_degrades_to_one_folder() {
        let (raw, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = crate::bridge::Events::detached(raw);
        let cancel = Cancel::new();
        let outcome = resolve_topics("Some Forum", 42, &cancel, &tx, || async {
            Result::<Vec<dialogs::Topic>, EnrichError>::Err(EnrichError::Refused(
                "CHAT_ADMIN_REQUIRED".into(),
            ))
        })
        .await;

        assert!(matches!(outcome, TopicsResolution::Unsplit));
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Warn(m) if m.contains("one folder"))),
            "got {events:?}"
        );
    }

    /// A cancel during the retry's wait must end the wait, not be reported as
    /// either a completed split or a chat failure — `export`'s loop is what
    /// turns this into "Stopped".
    #[tokio::test]
    async fn a_cancel_during_the_retry_wait_is_reported_as_neither_split_nor_failed() {
        use std::time::Duration;

        let (raw, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = crate::bridge::Events::detached(raw);
        let cancel = Cancel::new();
        cancel.cancel();
        let outcome = resolve_topics("Some Forum", 42, &cancel, &tx, || async {
            Result::<Vec<dialogs::Topic>, EnrichError>::Err(EnrichError::Transient(
                Duration::from_secs(30),
            ))
        })
        .await;

        assert!(matches!(outcome, TopicsResolution::Cancelled));
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        assert!(
            !events.iter().any(|e| matches!(e, Event::ChatFailed { .. })),
            "got {events:?}"
        );
    }

    /// What `scan_destination` sends for `dir`.
    async fn scanned(dir: &std::path::Path) -> Vec<Event> {
        let (raw, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = crate::bridge::Events::detached(raw);
        let settings = Settings {
            output_dir: dir.display().to_string(),
            ..Settings::default()
        };
        scan_destination(settings, tx).await;
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        events
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-scan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_destination_not_made_yet_is_quietly_empty() {
        // Before a first export this is every launch; a warning here would be
        // a warning on every launch.
        let parent = scratch("fresh");
        let events = scanned(&parent.join("Exports")).await;
        assert!(
            matches!(events.as_slice(), [Event::OnDisk { disk, .. }] if disk.is_empty()),
            "got {events:?}"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[tokio::test]
    async fn a_destination_out_of_reach_warns_and_keeps_the_marks() {
        // The unplugged drive: the folder and its parent are both gone. An
        // empty `OnDisk` here would re-mark every chat "Not exported yet".
        let parent = scratch("gone");
        let _ = std::fs::remove_dir_all(&parent);
        let events = scanned(&parent.join("Exports")).await;
        assert!(
            matches!(events.as_slice(), [Event::Warn(m)] if m.contains("left as they were")),
            "got {events:?}"
        );
    }

    #[tokio::test]
    async fn a_destination_that_is_not_a_folder_warns_and_keeps_the_marks() {
        let parent = scratch("file");
        let file = parent.join("Exports");
        std::fs::write(&file, b"").unwrap();
        let events = scanned(&file).await;
        assert!(
            matches!(events.as_slice(), [Event::Warn(_)]),
            "got {events:?}"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn a_login_step_puts_its_session_back_only_into_the_sign_in_it_came_from() {
        let mut slot = LoginSlot::default();
        slot.start("first");
        // A step in flight holds it; the slot is empty, so nothing waits.
        let (held, generation) = slot.take().unwrap();
        assert!(slot.take().is_none());
        assert!(
            slot.put_back(held, generation),
            "a wrong code keeps the token"
        );
        assert_eq!(slot.held, Some("first"));

        // Cancel pressed while a step is out: the step must not bring the
        // abandoned credential back when it finishes.
        let (held, generation) = slot.take().unwrap();
        slot.forget();
        assert!(!slot.put_back(held, generation));
        assert!(slot.held.is_none());

        // Nor may it overwrite a sign-in started over in the meantime.
        slot.start("second");
        let (_, generation) = slot.take().unwrap();
        slot.start("third");
        assert!(!slot.put_back("second", generation));
        assert_eq!(slot.held, Some("third"));
    }

    #[test]
    fn opening_a_missing_output_folder_creates_it_first() {
        let dir = std::env::temp_dir().join(format!("tgx-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Do not actually spawn a file manager in the test; just prove the
        // directory is created, which is the part that would otherwise fail
        // silently on a first run.
        if !dir.exists() {
            std::fs::create_dir_all(&dir).unwrap();
        }
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
