//! Read, analyse, render — off the main thread.
//!
//! **The thread is not precautionary.** Measured on the Python original: 6,643
//! messages take 0.3 s and 333,582 take 24.8 s. This port does the large one in
//! about 6 s, which is still six seconds in which a single-threaded window
//! would stop repainting — and a window that stops repainting is one Windows
//! offers to close for you.
//!
//! **The worker owns no widget.** It sends [`Progress`] and the shell decides
//! what that means, which is what keeps the failure path honest: a bad folder
//! or a malformed export becomes a [`Progress::Failed`] on screen rather than a
//! panic that takes the thread with no message at all.
//!
//! The one thing it does hold is a [`Wake`]: the window's "draw a frame",
//! called after every send. An `mpsc::Receiver` can only be *polled*, so
//! without that call nothing appears until some unrelated input causes a frame
//! -- which the user experiences as having to move the mouse to make the app
//! work. It used to be a clone of the egui context itself; it is a callback
//! now so this crate has no toolkit in it, and the window passes
//! `move || ctx.request_repaint()`.
//!
//! **A run is a queue of library rows**, one export after another. One that
//! fails is reported against its own row and the rest carry on: forty chats
//! should not hang on the one whose file is mid-sync.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use crate::refresh::{Source, Target, Written};

/// What the worker tells the window.
#[derive(Debug, Clone)]
pub enum Progress {
    /// `done` exports finished, and what is happening now.
    Step {
        done: usize,
        label: String,
    },
    /// This row is the one being read.
    Started(PathBuf),
    Wrote {
        path: PathBuf,
        written: Written,
    },
    /// A row that could not be read or written. The run carries on.
    Failed {
        path: PathBuf,
        error: String,
    },
    /// The queue is empty, or Stop was pressed and the row in hand finished.
    Finished {
        stopped: bool,
    },
}

/// Everything a run needs, resolved before the thread starts.
///
/// Taken by value so the worker borrows nothing from the view: a run that
/// outlives the window it was started from must still finish writing the file
/// rather than be torn out from under itself.
#[derive(Debug, Clone)]
pub struct Request {
    pub targets: Vec<Target>,
    pub write_digest: bool,
    pub embed_fonts: bool,
    pub handles_only: bool,
}

/// What the notes section says wrote the file.
pub const SOURCE: &str = "Telegram Export Analyser";

/// A running queue: the events, and the switch that stops it.
pub struct Run {
    pub events: Receiver<Progress>,
    stop: Arc<AtomicBool>,
}

impl Run {
    /// Stop **between** exports, never inside one. A report half-written is
    /// worse than the stale one it would have replaced, and reading a database
    /// has no point at which stopping leaves anything worth keeping.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What wakes the window after a send: `request_repaint`, in the app.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Start a run. The receiver is drained by the window on the next frame.
pub fn spawn(request: Request, wake: Wake) -> Run {
    let (tx, rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let tx = Waking { tx, wake };
    let flag = stop.clone();
    std::thread::Builder::new()
        .name("tga-analyse".into())
        .spawn(move || run(request, &flag, tx))
        // A thread that cannot be spawned is a machine in trouble, and there is
        // nothing useful to fall back to — the alternative is doing 6 seconds of
        // work on the paint thread.
        .expect("the OS refused a worker thread");
    Run { events: rx, stop }
}

/// A sender that wakes the window after every message.
///
/// Wrapped rather than left to the caller because "send, then repaint" has to
/// happen at every send site and forgetting it at one produces a window that
/// is a frame behind for the rest of the run.
struct Waking<T> {
    tx: Sender<T>,
    wake: Wake,
}

impl<T> Waking<T> {
    /// Every send is ignored on failure: the window may have been closed, and a
    /// worker that panics on a closed channel loses the report it had already
    /// written.
    fn send(&self, event: T) {
        let _ = self.tx.send(event);
        (self.wake)();
    }
}

fn run(request: Request, stop: &AtomicBool, tx: Waking<Progress>) {
    let options = tga_report::Options {
        embed_fonts: request.embed_fonts,
        source: SOURCE.to_string(),
        ..Default::default()
    };
    let total = request.targets.len();
    for (index, target) in request.targets.iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            tx.send(Progress::Finished { stopped: true });
            return;
        }
        let name = target.name();
        let head = format!("{}/{total}  {name}", index + 1);
        tx.send(Progress::Started(target.path().to_path_buf()));
        tx.send(Progress::Step {
            done: index,
            label: head.clone(),
        });
        let mut report = |_done: usize, _total: usize, topic: &str| {
            tx.send(Progress::Step {
                done: index,
                label: format!("{head}: reading {topic}"),
            });
        };
        let path = target.path().to_path_buf();
        match crate::refresh::one(
            target,
            request.write_digest,
            request.handles_only,
            &options,
            Some(&mut report),
        ) {
            Ok(written) => tx.send(Progress::Wrote { path, written }),
            Err(e) => tx.send(Progress::Failed {
                path,
                error: format!("{e:#}"),
            }),
        }
    }
    tx.send(Progress::Finished { stopped: false });
}

/// What the library learns about a row once its file has been opened.
#[derive(Debug, Clone)]
pub struct Counted {
    /// Which scan asked. A count that lands after the folder changed belongs
    /// to a list that is no longer on screen.
    pub generation: u64,
    pub path: PathBuf,
    /// The chat's real title, which the file's name only approximates: the
    /// exporter sanitises it for the filesystem.
    pub title: Option<String>,
    pub messages: Option<usize>,
    /// How many chats the file holds. More than one only for a flat
    /// `telegram.sqlite` from before `tgx split`.
    pub chats: usize,
    pub error: Option<String>,
}

/// Open every database in the list and count its messages, one row at a time.
///
/// **Off the paint thread because it is not free.** It is one indexed
/// `COUNT` per file, and forty of them over 36 GB of databases on a USB drive
/// is a few seconds — long enough to freeze a window that did it inline, short
/// enough that the list is usable while the figures fill in.
///
/// A folder export is not counted: that means parsing every `result.json`,
/// which is the run itself.
pub fn count(sources: Vec<Source>, generation: u64, wake: Wake) -> Receiver<Counted> {
    let (tx, rx) = channel();
    let tx = Waking { tx, wake };
    std::thread::Builder::new()
        .name("tga-count".into())
        .spawn(move || {
            for source in sources {
                let Source::Database(db) = source else {
                    continue;
                };
                let counted = match tga_db::chats(&db) {
                    // Largest first, which is the one `tga_db::load` reads.
                    Ok(chats) => Counted {
                        generation,
                        path: db.clone(),
                        title: chats.first().map(|c| c.title.clone()),
                        messages: chats.first().map(|c| c.messages),
                        chats: chats.len(),
                        error: None,
                    },
                    // Said against the row, before anybody presses Analyse: a
                    // file from a later schema is refused here by name.
                    Err(e) => Counted {
                        generation,
                        path: db.clone(),
                        title: None,
                        messages: None,
                        chats: 0,
                        error: Some(format!("{e:#}")),
                    },
                };
                tx.send(counted);
            }
        })
        .expect("the OS refused a worker thread");
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(dir: &std::path::Path) -> Target {
        crate::refresh::library(dir)
            .into_iter()
            .next()
            .unwrap_or_else(|| Target {
                source: Source::Folder(dir.to_path_buf()),
                out: dir.join("report.html"),
                stale: true,
                changed: None,
                reported: None,
                bytes: 0,
            })
    }

    #[test]
    fn a_folder_with_no_export_in_it_fails_rather_than_writing_an_empty_report() {
        // The commonest mistake this window can be handed: a folder that is not
        // an export. It has to come back as a sentence, not as a 1 MB file
        // describing nothing.
        let dir = std::env::temp_dir().join("tga-app-empty-folder");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let out = dir.join("report.html");
        let _ = std::fs::remove_file(&out);

        let (tx, rx) = channel();
        let wake: Wake = Arc::new(|| {});
        run(
            Request {
                targets: vec![target(&dir)],
                write_digest: false,
                embed_fonts: false,
                handles_only: false,
            },
            &AtomicBool::new(false),
            Waking { tx, wake },
        );

        let seen: Vec<Progress> = rx.try_iter().collect();
        // `tga_read::load` refuses first and names what it looked for, which is
        // the more useful of the two messages — the shell prints it verbatim.
        assert!(
            seen.iter().any(
                |p| matches!(p, Progress::Failed { error, .. } if error.contains("result.json"))
            ),
            "expected a Failed naming result.json, got {seen:?}"
        );
        assert!(
            matches!(seen.last(), Some(Progress::Finished { stopped: false })),
            "a failed row still ends the run: {seen:?}"
        );
        assert!(!out.is_file(), "nothing should have been written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stop_is_honoured_before_the_next_export_starts() {
        let dir = std::env::temp_dir().join("tga-app-stopped");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let (tx, rx) = channel();
        run(
            Request {
                targets: vec![target(&dir), target(&dir)],
                write_digest: false,
                embed_fonts: false,
                handles_only: false,
            },
            &AtomicBool::new(true),
            Waking {
                tx,
                wake: Arc::new(|| {}),
            },
        );
        let seen: Vec<Progress> = rx.try_iter().collect();
        assert!(
            !seen.iter().any(|p| matches!(p, Progress::Started(_))),
            "a stopped queue started a row: {seen:?}"
        );
        assert!(matches!(
            seen.last(),
            Some(Progress::Finished { stopped: true })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
