//! Refresh: re-analyse every database under a folder that has moved on since
//! its report was written.
//!
//! **This exists because the exporter stopped finishing.** A Classic export is
//! a folder written once; a database is synced in place, and the exporter's
//! "Update databases" re-syncs every chat a folder holds — one database per
//! chat, `<output_dir>/<title>/<title>.sqlite`. A report beside each goes stale
//! the moment that runs, and pointing the window at sixteen folders by hand is
//! the step nobody does twice.
//!
//! **Stale is decided by the clock on disk, not by reading the database.** A
//! report older than its database's last write — or than the notes file beside
//! it — is redone; anything else is left alone. Asking the `runs` table instead
//! was ruled out: `tgx import`, `tgx split` and the profile-picture pass all
//! write without a sync row of their own, and a report is stale after any of
//! them. The cost is the odd needless rewrite, which is the cheap direction to
//! be wrong in. `--force` is there for the change the clock cannot see: a new
//! build of this program.
//!
//! The report goes beside the database, in the chat's own folder, which is
//! where somebody handed that folder alone would have got it.
//!
//! **The window's library is built from the same plan.** One row per target,
//! and a folder export — which has no database to be stale against — is a
//! target too, dated by its newest `result.json`, so the window has one kind of
//! row whatever it was pointed at.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{bail, Result};

/// What a report is drawn from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    /// One of the exporter's databases.
    Database(PathBuf),
    /// A folder of `result.json` files, in either layout `tga-read` knows.
    Folder(PathBuf),
}

/// One export, and what analysing it would write.
#[derive(Debug, Clone)]
pub struct Target {
    pub source: Source,
    pub out: PathBuf,
    /// Whether the report is missing or older than what it was drawn from.
    pub stale: bool,
    /// The last write to the source, as [`is_stale`] compared it.
    pub changed: Option<SystemTime>,
    /// When the report beside it was written, if there is one.
    pub reported: Option<SystemTime>,
    /// How much is on disk: the database and its `-wal`, or every
    /// `result.json`. Media is not counted for either — a folder's media tree
    /// is not what the report reads.
    pub bytes: u64,
}

impl Target {
    /// What to call it before its real title is read: the file's name without
    /// `.sqlite`, which the exporter makes from the chat's title, or the
    /// folder's own name.
    pub fn name(&self) -> String {
        let path = self.path();
        let stem = match &self.source {
            Source::Database(_) => path.file_stem(),
            Source::Folder(_) => path.file_name(),
        };
        stem.map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    }

    /// The database file or the export folder: the row's identity.
    pub fn path(&self) -> &Path {
        match &self.source {
            Source::Database(p) | Source::Folder(p) => p,
        }
    }

    /// Where the report, the notes and the digest live.
    pub fn home(&self) -> PathBuf {
        match &self.source {
            Source::Database(db) => beside(db),
            Source::Folder(dir) => dir.clone(),
        }
    }
}

/// Every database under `root`, each with whether its report needs redoing.
pub fn plan(root: &Path) -> Vec<Target> {
    tga_db::find_all(root)
        .into_iter()
        .map(|db| target(Source::Database(db)))
        .collect()
}

/// Everything the window can analyse under `root`, one row per export.
///
/// - its databases;
/// - **the folder exports directly inside it**, when it is the exporter's
///   destination: a Classic run writes one folder per chat beside the
///   databases, and listing the destination as one export would read every
///   chat in it as one;
/// - or, when it holds neither, the folder export it is itself.
///
/// Databases first, for the reason in `cli::run`: a folder that holds one may
/// have no `result.json` in it at all.
pub fn library(root: &Path) -> Vec<Target> {
    let mut out = plan(root);
    if !is_export(root) {
        let mut children: Vec<PathBuf> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && is_export(p) && tga_db::find(p).is_none())
            .collect();
        children.sort();
        out.extend(children.into_iter().map(|dir| target(Source::Folder(dir))));
    }
    if out.is_empty() && root.is_dir() && !tga_read::find_results(root, 3).is_empty() {
        out.push(target(Source::Folder(root.to_path_buf())));
    }
    out
}

/// Whether `dir` is one export by itself, rather than a folder of them.
///
/// Telegram Desktop writes `export_results.html` at the root of every export,
/// and a single chat's `result.json` sits there too; the exporter's forum
/// export writes the same index page above its topic folders. A destination
/// has neither: its exports are the folders inside it.
pub fn is_export(dir: &Path) -> bool {
    dir.join("result.json").is_file() || dir.join("export_results.html").is_file()
}

/// The one export at `path` — a database file or an export folder — as a
/// target, or `None` if it is neither. What "report after export" hands the
/// analyser for the chat that has just finished.
pub fn target_for(path: &Path) -> Option<Target> {
    if path.is_file() {
        return tga_db::find(path).map(|db| target(Source::Database(db)));
    }
    (path.is_dir() && !tga_read::find_results(path, 3).is_empty())
        .then(|| target(Source::Folder(path.to_path_buf())))
}

fn target(source: Source) -> Target {
    let (changed, bytes) = match &source {
        Source::Database(db) => (tga_db::modified(db), size(db) + size(&wal(db))),
        Source::Folder(dir) => {
            // The depth `tga_read::load` reads at, so the date is of exactly
            // the files the report would be drawn from.
            let files = tga_read::find_results(dir, 3);
            (
                files.iter().filter_map(|f| modified(f)).max(),
                files.iter().map(|f| size(f)).sum(),
            )
        }
    };
    let home = match &source {
        Source::Database(db) => beside(db),
        Source::Folder(dir) => dir.clone(),
    };
    let out = home.join("report.html");
    let reported = modified(&out);
    let stale = is_stale(changed, reported, &home);
    Target {
        source,
        out,
        stale,
        changed,
        reported,
        bytes,
    }
}

/// The folder a database sits in, which is where its report and notes live.
pub fn beside(db: &Path) -> PathBuf {
    db.parent().unwrap_or(Path::new(".")).to_path_buf()
}

fn is_stale(changed: Option<SystemTime>, reported: Option<SystemTime>, home: &Path) -> bool {
    let Some(written) = reported else {
        return true;
    };
    // A source whose time cannot be read is redone rather than trusted: a
    // report that is needlessly rewritten costs seconds, one that is wrongly
    // kept costs somebody reading last month's figures as this month's.
    let Some(changed) = changed else {
        return true;
    };
    let notes = tga_notes::find(home).and_then(|p| modified(&p));
    changed > written || notes.is_some_and(|n| n > written)
}

fn modified(path: &Path) -> Option<SystemTime> {
    path.metadata().and_then(|m| m.modified()).ok()
}

fn size(path: &Path) -> u64 {
    path.metadata().map(|m| m.len()).unwrap_or(0)
}

fn wal(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push("-wal");
    PathBuf::from(name)
}

/// What one refreshed report holds.
#[derive(Debug, Clone)]
pub struct Written {
    pub out: PathBuf,
    pub messages: usize,
    pub topics: usize,
    pub people: usize,
    pub events: usize,
    /// The digest could not be written. Said, not failed over: the report is
    /// the thing that was asked for, as in a single run.
    pub warning: Option<String>,
}

/// Read one export and write its report — and, if asked, its digest — beside
/// it.
///
/// The same steps a single run takes, in the same order, so a refreshed report
/// is byte for byte the one `TelegramTools analyse <folder>` would have written.
///
/// `handles` is `--handles`: every name becomes an `@handle` before anything
/// is counted, digest included. See `tga_read::Export::handles_only`.
pub fn one(
    target: &Target,
    digest: bool,
    handles: bool,
    options: &tga_report::Options,
    progress: Option<tga_read::Progress<'_>>,
) -> Result<Written> {
    let mut export = match &target.source {
        Source::Database(db) => tga_db::load(db, None, progress)?,
        Source::Folder(dir) => tga_read::load(dir, progress)?,
    };
    if export.msgs.is_empty() {
        bail!("no messages in it");
    }
    if handles {
        export.handles_only();
    }
    let (stats, people) = tga_metrics::analyse(&export);
    let folder = target.home();
    let notes = tga_notes::load(&folder);

    let html = tga_report::render(
        &stats,
        &tga_report::names_from_stats(&stats),
        &notes,
        options,
    );
    // **Written beside, then renamed over.** A report is judged up to date by
    // its modification time, so one cut short — the window closed mid-write,
    // which does not wait for this thread — would otherwise read as current
    // forever. A rename is all or nothing; a half-written `.part` is not a
    // report and is never listed as one.
    let part = target.out.with_extension("html.part");
    std::fs::write(&part, html)
        .and_then(|()| std::fs::rename(&part, &target.out))
        .map_err(|e| anyhow::anyhow!("{}: {e}", target.out.display()))?;

    let mut warning = None;
    if digest {
        let dir = folder.join("analysis");
        let stored = match &target.source {
            Source::Database(db) => crate::cli::Stored::open(db, &export),
            Source::Folder(_) => None,
        };
        let rows = crate::cli::digest_rows(&export, &people, stored.as_ref());
        if let Err(e) =
            tga_notes::write_digest(&rows, &dir).and_then(|_| tga_notes::write_brief(&dir))
        {
            warning = Some(format!("the digest could not be written: {e}"));
        }
    }

    Ok(Written {
        out: target.out.clone(),
        messages: export.msgs.len(),
        topics: export.topics.len(),
        people: stats.people.speakers,
        events: notes.events.len(),
        warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn touch(path: &Path, at: SystemTime) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        let file = std::fs::File::create(path).expect("create");
        file.set_modified(at).expect("set time");
    }

    #[test]
    fn a_folder_export_is_one_row_and_is_analysed_like_a_database() {
        // The window has one kind of row whatever it is pointed at. The
        // committed synthetic export names nobody, so this never skips.
        let base = std::env::temp_dir().join("tga-app-refresh-folder");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("mkdir");
        let synthetic =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tga-report/tests/synthetic/result.json");
        std::fs::copy(&synthetic, base.join("result.json")).expect("copy");

        let rows = library(&base);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, Source::Folder(base.clone()));
        assert!(rows[0].stale && rows[0].reported.is_none());
        assert!(rows[0].bytes > 0, "dated and sized by its result.json");

        let done = one(
            &rows[0],
            false,
            false,
            &tga_report::Options {
                embed_fonts: false,
                ..Default::default()
            },
            None,
        )
        .expect("analysed");
        assert!(done.messages > 0);
        assert!(base.join("report.html").is_file());
        assert!(!library(&base)[0].stale, "a fresh report is up to date");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_destination_lists_each_chat_folder_beside_its_databases() {
        let base = std::env::temp_dir().join("tga-run-refresh-destination");
        let _ = std::fs::remove_dir_all(&base);
        let synthetic =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tga-report/tests/synthetic/result.json");
        let now = SystemTime::now();
        // A synced chat.
        touch(&base.join("Synced").join("Synced.sqlite"), now);
        // A Classic chat: result.json at its root.
        std::fs::create_dir_all(base.join("Plain")).unwrap();
        std::fs::copy(&synthetic, base.join("Plain").join("result.json")).unwrap();
        // A Classic forum: the index page at its root, a topic folder below.
        std::fs::create_dir_all(base.join("Forum").join("General")).unwrap();
        touch(&base.join("Forum").join("export_results.html"), now);
        std::fs::copy(&synthetic, base.join("Forum/General/result.json")).unwrap();
        // Not an export at all.
        std::fs::create_dir_all(base.join("Scratch")).unwrap();

        let rows: Vec<Source> = library(&base).into_iter().map(|t| t.source).collect();
        assert_eq!(
            rows,
            vec![
                Source::Database(base.join("Synced").join("Synced.sqlite")),
                Source::Folder(base.join("Forum")),
                Source::Folder(base.join("Plain")),
            ]
        );
        // Pointed at one of them, the forum is one export, not one per topic.
        let forum: Vec<Source> = library(&base.join("Forum"))
            .into_iter()
            .map(|t| t.source)
            .collect();
        assert_eq!(forum, vec![Source::Folder(base.join("Forum"))]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_database_digest_reads_a_documents_text_out_of_the_database() {
        // The case PLAN.md left open: a database has no folder for the file to
        // be in, so the document's text has to come out of `blobs`.
        let base = std::env::temp_dir().join("tga-run-refresh-stored-doc");
        let _ = std::fs::remove_dir_all(&base);
        let db = base.join("Dev").join("Dev.sqlite");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        const CHAT: i64 = 4242;
        let message = |id: i64, text: &str, file: bool| {
            let mut m = serde_json::json!({
                "id": id, "type": "message", "date": "2026-07-16T10:00:00",
                "date_unixtime": "1784196000", "from": "Ana", "from_id": "user1",
                "text": text,
            });
            if file {
                m["file"] = "files/zapisnik.txt".into();
                m["file_name"] = "zapisnik 16.07.2026.txt".into();
            }
            m
        };
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                let store = tgx_archive::Store::open(&db).await.unwrap();
                store
                    .upsert_chat(CHAT, "Dev", "private_group", 0)
                    .await
                    .unwrap();
                let head = serde_json::json!({});
                store
                    .upsert_topic(CHAT, 0, "Dev", head.as_object().unwrap(), 0)
                    .await
                    .unwrap();
                for (id, text, file) in [(1, "dobar dan", false), (2, "evo zapisnika", true)] {
                    let m = message(id, text, file);
                    store
                        .merge(CHAT, id, Some(0), m.as_object().unwrap(), 0)
                        .await
                        .unwrap();
                }
                store
                    .put_blob(
                        77,
                        "document",
                        "text/plain",
                        Some("zapisnik 16.07.2026.txt"),
                        "Prisutni: 14\nOdluka: usvojeno".as_bytes(),
                        0,
                    )
                    .await
                    .unwrap();
                store
                    .link_media(CHAT, 2, "file", 77, "document", "files/zapisnik.txt")
                    .await
                    .unwrap();
            });

        let target = target_for(&db).expect("a database");
        one(
            &target,
            true,
            false,
            &tga_report::Options {
                embed_fonts: false,
                ..Default::default()
            },
            None,
        )
        .expect("analysed");
        let digest =
            std::fs::read_to_string(base.join("Dev").join("analysis").join("digest.jsonl"))
                .expect("digest written");
        assert!(
            digest.contains("Odluka: usvojeno"),
            "the stored document's text is in the digest: {digest}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_finished_export_is_found_by_its_own_path() {
        let base = std::env::temp_dir().join("tga-run-refresh-target-for");
        let _ = std::fs::remove_dir_all(&base);
        let synthetic =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tga-report/tests/synthetic/result.json");
        std::fs::create_dir_all(base.join("Plain")).unwrap();
        std::fs::copy(&synthetic, base.join("Plain").join("result.json")).unwrap();
        let folder = target_for(&base.join("Plain")).expect("a folder export");
        assert_eq!(folder.source, Source::Folder(base.join("Plain")));
        assert_eq!(folder.out, base.join("Plain").join("report.html"));
        assert!(target_for(&base.join("Nowhere")).is_none());
        std::fs::create_dir_all(base.join("Empty")).unwrap();
        assert!(target_for(&base.join("Empty")).is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_report_is_redone_only_when_something_it_was_drawn_from_moved_on() {
        let base = std::env::temp_dir().join("tga-app-refresh-plan");
        let _ = std::fs::remove_dir_all(&base);
        let then = SystemTime::now() - Duration::from_secs(86_400);
        let hour = Duration::from_secs(3600);

        // Never reported on.
        touch(&base.join("A").join("A.sqlite"), then);
        // Reported on after its last sync.
        touch(&base.join("B").join("B.sqlite"), then);
        touch(&base.join("B").join("report.html"), then + hour);
        // Synced again since: the case the whole module is for.
        touch(&base.join("C").join("C.sqlite"), then + hour * 2);
        touch(&base.join("C").join("report.html"), then + hour);
        // Synced in WAL mode: the main file is old, the side file is not.
        touch(&base.join("D").join("D.sqlite"), then);
        touch(&base.join("D").join("D.sqlite-wal"), then + hour * 2);
        touch(&base.join("D").join("report.html"), then + hour);
        // Its notes were edited after the report.
        touch(&base.join("E").join("E.sqlite"), then);
        touch(&base.join("E").join("report.html"), then + hour);
        touch(&base.join("E").join("events.json"), then + hour * 2);

        let plan = plan(&base);
        let stale: Vec<(String, bool)> = plan.iter().map(|t| (t.name(), t.stale)).collect();
        assert_eq!(
            stale,
            vec![
                ("A".into(), true),
                ("B".into(), false),
                ("C".into(), true),
                ("D".into(), true),
                ("E".into(), true),
            ]
        );
        assert_eq!(plan[1].out, base.join("B").join("report.html"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
