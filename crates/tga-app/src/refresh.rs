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

/// Everything the window can analyse under `root`: its databases, or — when
/// it holds none — the folder export it is.
///
/// Databases first, for the reason in `cli::run`: a folder that holds one may
/// have no `result.json` in it at all.
pub fn library(root: &Path) -> Vec<Target> {
    let databases = plan(root);
    if !databases.is_empty() {
        return databases;
    }
    if root.is_dir() && !tga_read::find_results(root, 3).is_empty() {
        return vec![target(Source::Folder(root.to_path_buf()))];
    }
    Vec::new()
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
/// is byte for byte the one `TelegramAnalyser <folder>` would have written.
pub fn one(
    target: &Target,
    digest: bool,
    options: &tga_report::Options,
    progress: Option<tga_read::Progress<'_>>,
) -> Result<Written> {
    let export = match &target.source {
        Source::Database(db) => tga_db::load(db, None, progress)?,
        Source::Folder(dir) => tga_read::load(dir, progress)?,
    };
    if export.msgs.is_empty() {
        bail!("no messages in it");
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
    std::fs::write(&target.out, html)
        .map_err(|e| anyhow::anyhow!("{}: {e}", target.out.display()))?;

    let mut warning = None;
    if digest {
        let dir = folder.join("analysis");
        let rows = crate::cli::digest_rows(&export, &people);
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
