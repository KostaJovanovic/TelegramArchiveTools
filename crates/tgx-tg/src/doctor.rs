//! **What is wrong with an export folder, and what can be put right.**
//!
//! Offline, and read-only unless asked: [`examine`] looks and reports, and
//! only [`repair`] changes anything, and only what a finding offers to fix.
//! [`duplicates`] is a report of media stored more than once, and has no
//! repair at all.
//! Every fix is a move or a removal of something this program made — a flat
//! database from an earlier build, a scratch tree a crashed run left, the
//! journal of a database that is no longer there — never a message.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Something a run will get wrong, or has.
    Problem,
    /// Worth knowing; nothing will go wrong because of it.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// Move a database (and its `-wal`/`-shm`) to `<dir>/<name>/<name>.sqlite`.
    IntoFolder { from: PathBuf, name: String },
    /// Rename a database's folder and file to `name`, the one a sync of its
    /// chat will look for.
    Rename { folder: PathBuf, name: String },
    /// Delete a file or folder this program made and nothing reads.
    Remove(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub path: PathBuf,
    pub what: String,
    pub fix: Option<Fix>,
}

impl Finding {
    /// One line for a terminal or the window's log.
    pub fn line(&self) -> String {
        let tag = match self.severity {
            Severity::Problem => "PROBLEM",
            Severity::Note => "note",
        };
        let fix = match &self.fix {
            Some(Fix::IntoFolder { name, .. }) => format!(" — fix: move it into {name}\\"),
            Some(Fix::Rename { name, .. }) => format!(" — fix: rename it to {name}"),
            Some(Fix::Remove(_)) => " — fix: delete it".to_string(),
            None => String::new(),
        };
        format!("{tag}: {}: {}{fix}", self.path.display(), self.what)
    }
}

/// Look at every database and export folder directly under `dir`.
pub async fn examine(dir: &Path) -> std::io::Result<Vec<Finding>> {
    let ext = tgx_archive::FILE_EXT;
    let mut out = Vec::new();
    let mut databases: Vec<(PathBuf, Option<String>)> = Vec::new();

    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    entries.sort();

    for path in &entries {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            if name == ".tgx-scratch" {
                out.push(Finding {
                    severity: Severity::Note,
                    path: path.clone(),
                    what: "media scratch left by a database run that did not finish (safe to \
                           delete when no export is running)"
                        .into(),
                    fix: Some(Fix::Remove(path.clone())),
                });
                continue;
            }
            let inner = path.join(format!("{name}.{ext}"));
            if inner.is_file() {
                databases.push((inner, Some(name.clone())));
            }
            orphan_journals(path, &mut out);
            for result in [path.join("result.json")]
                .into_iter()
                .chain(subfolders(path).into_iter().map(|d| d.join("result.json")))
            {
                if std::fs::metadata(&result).is_ok_and(|m| m.is_file() && m.len() == 0) {
                    out.push(Finding {
                        severity: Severity::Problem,
                        path: result,
                        what: "empty result.json — a Classic run ended without closing it; \
                               export the chat again"
                            .into(),
                        fix: None,
                    });
                }
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            databases.push((path.clone(), None));
        }
    }
    orphan_journals(dir, &mut out);

    let mut owners: HashMap<i64, Vec<PathBuf>> = HashMap::new();
    for (path, folder) in &databases {
        let chats = match tgx_archive::Store::chats_in(path).await {
            Ok(c) => c,
            Err(e) => {
                out.push(Finding {
                    severity: Severity::Problem,
                    path: path.clone(),
                    what: format!("cannot be read: {e}"),
                    fix: None,
                });
                continue;
            }
        };
        if chats.len() > 1 {
            out.push(Finding {
                severity: Severity::Problem,
                path: path.clone(),
                what: format!(
                    "holds {} chats — from before one database per chat; no run writes into \
                     it. Split it with: TelegramTools split \"{}\"",
                    chats.len(),
                    path.display()
                ),
                fix: None,
            });
            continue;
        }
        let Some(chat) = chats.into_iter().next() else {
            out.push(Finding {
                severity: Severity::Note,
                path: path.clone(),
                what: "records no chat".into(),
                fix: None,
            });
            continue;
        };
        owners.entry(chat.id).or_default().push(path.clone());
        let base = tgx_media::topics::sanitize_component(&chat.title, "chat");
        let distinct = format!("{base} ({})", chat.id);
        match folder {
            None => {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let taken = dir.join(&stem).join(format!("{stem}.{ext}")).exists();
                out.push(Finding {
                    severity: Severity::Note,
                    path: path.clone(),
                    what: "a database outside a folder of its own, from an earlier build \
                           (the next sync of its chat moves it anyway)"
                        .into(),
                    fix: (!taken).then(|| Fix::IntoFolder {
                        from: path.clone(),
                        name: stem,
                    }),
                });
            }
            Some(name) if *name != base && *name != distinct => {
                let taken = dir.join(&base).exists();
                out.push(Finding {
                    severity: Severity::Problem,
                    path: path.clone(),
                    what: format!(
                        "named {name:?}, but a sync of {:?} looks for {base:?} and would start \
                         a new database there instead of adding to this one",
                        chat.title
                    ),
                    fix: (!taken).then(|| Fix::Rename {
                        folder: dir.join(name),
                        name: base.clone(),
                    }),
                });
            }
            Some(_) => {}
        }
        if chat.synced.is_none() {
            out.push(Finding {
                severity: Severity::Note,
                path: path.clone(),
                what: "has never finished a sync — Update databases will read the chat whole"
                    .into(),
                fix: None,
            });
        }
    }
    let mut dupes: Vec<_> = owners.into_iter().filter(|(_, v)| v.len() > 1).collect();
    dupes.sort();
    for (id, paths) in dupes {
        out.push(Finding {
            severity: Severity::Problem,
            path: paths[0].clone(),
            what: format!(
                "chat {id} has {} databases; only one is synced. The others: {}",
                paths.len(),
                paths[1..]
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            fix: None,
        });
    }
    out.sort_by(|a, b| a.severity.cmp(&b.severity).then(a.path.cmp(&b.path)));
    Ok(out)
}

fn subfolders(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|e| {
            e.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// A `-wal` or `-shm` whose database is gone. SQLite would apply a stray WAL
/// to a **new** file of the same name, which is the one thing worse than
/// leaving it there.
fn orphan_journals(dir: &Path, out: &mut Vec<Finding>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let suffixes = [
        format!(".{}-wal", tgx_archive::FILE_EXT),
        format!(".{}-shm", tgx_archive::FILE_EXT),
    ];
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for suffix in &suffixes {
            if let Some(stem) = name.strip_suffix(suffix.as_str()) {
                let db = dir.join(format!("{stem}.{}", tgx_archive::FILE_EXT));
                if !db.exists() {
                    out.push(Finding {
                        severity: Severity::Problem,
                        path: path.clone(),
                        what: "journal of a database that is not there".into(),
                        fix: Some(Fix::Remove(path.clone())),
                    });
                }
            }
        }
    }
}

/// Apply every fix the findings offer. Returns one line per fix: what was
/// done, or why it was not.
pub fn repair(dir: &Path, findings: &[Finding]) -> Vec<String> {
    let ext = tgx_archive::FILE_EXT;
    let mut out = Vec::new();
    for f in findings {
        let Some(fix) = &f.fix else { continue };
        let done = match fix {
            Fix::IntoFolder { name, .. } => crate::engine::archive_path(dir, name)
                .map(|to| format!("moved to {}", to.display())),
            Fix::Rename { folder, name } => (|| {
                let old = folder
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                for suffix in ["", "-wal", "-shm"] {
                    let from = folder.join(format!("{old}.{ext}{suffix}"));
                    if from.exists() {
                        std::fs::rename(&from, folder.join(format!("{name}.{ext}{suffix}")))?;
                    }
                }
                let to = dir.join(name);
                std::fs::rename(folder, &to)?;
                Ok(format!("renamed to {}", to.display()))
            })(),
            Fix::Remove(path) => if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            }
            .map(|()| "deleted".to_string()),
        };
        out.push(match done {
            Ok(what) => format!("{}: {what}", f.path.display()),
            Err(e) => format!("{}: NOT fixed: {e}", f.path.display()),
        });
    }
    out
}

// --- duplicate media ---------------------------------------------------------
//
// **A report, and nothing more.** Which copy of a file is the one to keep is
// not a question this program can answer: a blob is what a database's
// messages point at, a file in a Classic folder is what that export's pages
// link to, and removing either breaks something that worked. So this says
// where the space went and leaves the deciding to whoever reads it.

/// The folders a Classic export keeps media in — every folder
/// `tgx_media::names::layout` can name. A file anywhere else in an export is
/// the export's own chrome (`result.json`, the pages, `css/`, `js/`,
/// `images/`), identical across every export by design and nobody's media.
pub const MEDIA_FOLDERS: &[&str] = &[
    "photos",
    "video_files",
    "voice_messages",
    "round_video_messages",
    "stickers",
    "thumbnails",
    "files",
];

/// Where one copy of a file is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Place {
    /// A row in a database's `blobs` table.
    Blob {
        db: PathBuf,
        file_id: i64,
        kind: String,
        file_name: Option<String>,
    },
    /// A file inside a Classic export folder.
    File(PathBuf),
}

impl Place {
    /// One line for a terminal or the window's log. Names a file, never
    /// what is in it.
    pub fn line(&self) -> String {
        match self {
            Place::Blob {
                db,
                file_id,
                kind,
                file_name,
            } => {
                let name = file_name
                    .as_deref()
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default();
                format!("{}: {kind} {file_id}{name}", db.display())
            }
            Place::File(path) => path.display().to_string(),
        }
    }
}

/// Copies of one file: the same bytes, compared byte for byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DupeGroup {
    pub size: u64,
    pub copies: Vec<Place>,
}

impl DupeGroup {
    /// What the copies past the first take up.
    pub fn wasted(&self) -> u64 {
        self.size
            .saturating_mul(self.copies.len().saturating_sub(1) as u64)
    }
}

#[derive(Debug, Clone, Default)]
pub struct DupeReport {
    /// Largest waste first.
    pub groups: Vec<DupeGroup>,
    pub wasted_bytes: u64,
    /// Files looked at in Classic export folders.
    pub scanned_files: usize,
    /// Blobs looked at across every database.
    pub scanned_blobs: usize,
    /// Databases that could not be read, and why. Their blobs are not in the
    /// report, which is therefore a floor.
    pub unreadable: Vec<(PathBuf, String)>,
}

impl DupeReport {
    /// The totals, as one line.
    pub fn summary(&self) -> String {
        let human = |n: u64| crate::engine::human_bytes(i64::try_from(n).unwrap_or(i64::MAX));
        format!(
            "{} file(s) in export folders and {} blob(s) in databases: {} held more than once, \
             {} taken up by the extra copies",
            self.scanned_files,
            self.scanned_blobs,
            self.groups.len(),
            human(self.wasted_bytes),
        )
    }

    /// The summary, then the `top` most wasteful groups with every place
    /// each copy is, then the databases that could not be read.
    pub fn lines(&self, top: usize) -> Vec<String> {
        let human = |n: u64| crate::engine::human_bytes(i64::try_from(n).unwrap_or(i64::MAX));
        let mut out = vec![self.summary()];
        for g in self.groups.iter().take(top) {
            out.push(format!(
                "  {} copies of {}, {} extra:",
                g.copies.len(),
                human(g.size),
                human(g.wasted())
            ));
            out.extend(g.copies.iter().map(|p| format!("    {}", p.line())));
        }
        if self.groups.len() > top {
            out.push(format!(
                "  … and {} smaller group(s)",
                self.groups.len() - top
            ));
        }
        out.extend(
            self.unreadable
                .iter()
                .map(|(p, why)| format!("  not read: {}: {why}", p.display())),
        );
        out
    }
}

/// **Which media is stored more than once under `dir`**, across every
/// database and every Classic export folder. Changes nothing.
///
/// Three passes, each over fewer files than the last. **By size first**, from
/// the `blobs.size` column and the file system, which reads no file at all —
/// two files of different lengths cannot be equal, and most sizes are held
/// once. Then a hash of each file in a size more than one shares, read one
/// at a time so the memory is one file's worth however large the folder.
/// Then, since a 64-bit hash can collide, **a byte comparison** of every
/// file in a hash bucket against the first of its kind: a group in this report
/// is the same bytes, not the same hash.
///
/// A database `archived_chats` declines because it holds several chats — the
/// one-file build's `telegram.sqlite` — is read anyway: after a `tgx split`
/// it is the likeliest place for a second copy of everything.
pub async fn duplicates(dir: &Path) -> std::io::Result<DupeReport> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;

    let archived = crate::engine::archived_chats(dir).await?;
    let mut report = DupeReport::default();
    let mut dbs: Vec<PathBuf> = archived.files.into_values().collect();
    for (path, why) in archived.unreadable {
        // Another schema, or not ours, still fails here; several chats does not.
        if tgx_archive::Store::open_read(&path).await.is_ok() {
            dbs.push(path);
        } else {
            report.unreadable.push((path, why));
        }
    }
    dbs.sort();
    dbs.dedup();

    let mut stores: HashMap<PathBuf, tgx_archive::Store> = HashMap::new();
    let mut by_size: HashMap<u64, Vec<Place>> = HashMap::new();
    for path in dbs {
        let store = match tgx_archive::Store::open_read(&path).await {
            Ok(s) => s,
            Err(e) => {
                report.unreadable.push((path, e.to_string()));
                continue;
            }
        };
        let sizes = match store.blob_sizes().await {
            Ok(s) => s,
            Err(e) => {
                report.unreadable.push((path, e.to_string()));
                continue;
            }
        };
        for (file_id, kind, size, file_name) in sizes {
            report.scanned_blobs += 1;
            // Every empty file equals every other, and costs nothing.
            if size > 0 {
                by_size.entry(size).or_default().push(Place::Blob {
                    db: path.clone(),
                    file_id,
                    kind,
                    file_name,
                });
            }
        }
        stores.insert(path, store);
    }

    let mut folders: Vec<PathBuf> = crate::engine::exported_folders(dir)?
        .into_values()
        .flatten()
        .collect();
    folders.sort();
    folders.dedup();
    let mut files = Vec::new();
    for folder in &folders {
        media_files(folder, &mut files);
    }
    for (path, size) in files {
        report.scanned_files += 1;
        if size > 0 {
            by_size.entry(size).or_default().push(Place::File(path));
        }
    }

    for (size, places) in by_size {
        if places.len() < 2 {
            continue;
        }
        let mut by_hash: HashMap<u64, Vec<Place>> = HashMap::new();
        for place in places {
            let Some(bytes) = read_copy(&place, size, &stores).await else {
                continue;
            };
            let mut h = DefaultHasher::new();
            h.write(&bytes);
            by_hash.entry(h.finish()).or_default().push(place);
        }
        for (_, bucket) in by_hash {
            if bucket.len() < 2 {
                continue;
            }
            // One representative's bytes per distinct content — nearly always
            // one, since a collision is what this pass exists to rule out.
            let mut kinds: Vec<(Vec<u8>, Vec<Place>)> = Vec::new();
            for place in bucket {
                let Some(bytes) = read_copy(&place, size, &stores).await else {
                    continue;
                };
                match kinds.iter_mut().find(|(rep, _)| *rep == bytes) {
                    Some((_, copies)) => copies.push(place),
                    None => kinds.push((bytes, vec![place])),
                }
            }
            for (_, mut copies) in kinds {
                if copies.len() > 1 {
                    copies.sort();
                    report.groups.push(DupeGroup { size, copies });
                }
            }
        }
    }
    report
        .groups
        .sort_by(|a, b| b.wasted().cmp(&a.wasted()).then(a.copies.cmp(&b.copies)));
    report.wasted_bytes = report.groups.iter().map(DupeGroup::wasted).sum();
    report.unreadable.sort();
    Ok(report)
}

/// One copy's bytes. `None` for a copy that vanished, or changed length,
/// since it was sized: it is left out rather than guessed about.
async fn read_copy(
    place: &Place,
    size: u64,
    stores: &HashMap<PathBuf, tgx_archive::Store>,
) -> Option<Vec<u8>> {
    let bytes = match place {
        Place::Blob {
            db, file_id, kind, ..
        } => stores.get(db)?.blob(*file_id, kind).await.ok().flatten(),
        Place::File(path) => std::fs::read(path).ok(),
    }?;
    (bytes.len() as u64 == size).then_some(bytes)
}

/// Every file under `dir` whose folder is one of [`MEDIA_FOLDERS`], with its
/// length. Recursive, because a forum split into topics keeps its media a
/// level down; symlinks are not followed.
fn media_files(dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let in_media = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| MEDIA_FOLDERS.contains(&n));
    for entry in entries.filter_map(Result::ok) {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if entry.file_name() != ".tgx-scratch" {
                media_files(&entry.path(), out);
            }
        } else if kind.is_file() && in_media {
            if let Ok(meta) = entry.metadata() {
                out.push((entry.path(), meta.len()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tgx-doctor-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn db(path: &Path, chats: &[(i64, &str)], synced: bool) {
        let store = tgx_archive::Store::open(path).await.unwrap();
        for (id, title) in chats {
            store
                .upsert_chat(*id, title, "private_group", 1)
                .await
                .unwrap();
            if synced {
                let run = store
                    .start_run(*id, "database", None, "{}", 1)
                    .await
                    .unwrap();
                store.finish_run(run, 0, 0, 0, 0, true, 2).await.unwrap();
            }
        }
    }

    fn has(findings: &[Finding], needle: &str) -> bool {
        findings.iter().any(|f| f.line().contains(needle))
    }

    #[tokio::test]
    async fn every_kind_of_trouble_is_found_and_the_fixable_ones_are_fixed() {
        let dir = tmp("all");
        db(&dir.join("Good").join("Good.sqlite"), &[(1, "Good")], true).await;
        db(&dir.join("Flat.sqlite"), &[(2, "Flat")], true).await;
        db(
            &dir.join("Moved").join("Moved.sqlite"),
            &[(3, "Real")],
            true,
        )
        .await;
        db(
            &dir.join("Never").join("Never.sqlite"),
            &[(4, "Never")],
            false,
        )
        .await;
        db(&dir.join("Both.sqlite"), &[(5, "A"), (6, "B")], true).await;
        db(
            &dir.join("Good (1)").join("Good (1).sqlite"),
            &[(1, "Good")],
            true,
        )
        .await;
        std::fs::create_dir_all(dir.join(".tgx-scratch").join("x")).unwrap();
        std::fs::write(dir.join("Gone.sqlite-wal"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("Classic")).unwrap();
        std::fs::write(dir.join("Classic").join("result.json"), b"").unwrap();

        let found = examine(&dir).await.unwrap();
        assert!(has(&found, "outside a folder of its own"));
        assert!(has(&found, "would start a new database"));
        assert!(has(&found, "never finished a sync"));
        assert!(has(&found, "holds 2 chats"));
        assert!(has(&found, "chat 1 has 2 databases"));
        assert!(has(&found, "media scratch"));
        assert!(has(&found, "journal of a database that is not there"));
        assert!(has(&found, "empty result.json"));
        // A healthy database is not a finding of its own; it is named only as
        // the first of the two that share chat 1.
        let good = dir.join("Good").join("Good.sqlite");
        assert!(found
            .iter()
            .filter(|f| f.path == good)
            .all(|f| f.what.contains("2 databases")));
        // The problems sort first.
        assert_eq!(found[0].severity, Severity::Problem);

        let done = repair(&dir, &found);
        assert!(done.iter().all(|l| !l.contains("NOT fixed")), "{done:?}");
        assert!(dir.join("Flat").join("Flat.sqlite").is_file());
        assert!(dir.join("Real").join("Real.sqlite").is_file());
        assert!(!dir.join(".tgx-scratch").exists());
        assert!(!dir.join("Gone.sqlite-wal").exists());

        let after = examine(&dir).await.unwrap();
        assert!(after.iter().all(|f| f.fix.is_none()), "{after:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A database at `<dir>/<name>/<name>.sqlite` holding one chat and the
    /// given blobs, as `(file_id, bytes)`.
    async fn db_with_blobs(dir: &Path, name: &str, chat: i64, blobs: &[(i64, &[u8])]) -> PathBuf {
        let path = dir.join(name).join(format!("{name}.sqlite"));
        let store = tgx_archive::Store::open(&path).await.unwrap();
        store
            .upsert_chat(chat, name, "private_group", 1)
            .await
            .unwrap();
        for (id, bytes) in blobs {
            store
                .put_blob(*id, "photos", "image/jpeg", None, bytes, 1)
                .await
                .unwrap();
        }
        path
    }

    #[tokio::test]
    async fn the_same_bytes_are_one_group_wherever_they_are_and_same_size_is_not_enough() {
        let dir = tmp("dupes");
        let photo = b"the same photo, twice over".as_slice();
        // Same length as `photo`, different bytes: the size pass groups it
        // with them, and the hash and the comparison must take it out again.
        let other = b"a different photo, same sz".as_slice();
        assert_eq!(photo.len(), other.len());
        let a = db_with_blobs(&dir, "A", 1, &[(10, photo), (11, other)]).await;
        let b = db_with_blobs(&dir, "B", 2, &[(20, photo)]).await;

        // A Classic export of a third chat holding the photo as well, plus
        // chrome that is identical in every export and is not media.
        let classic = dir.join("C");
        std::fs::create_dir_all(classic.join("photos")).unwrap();
        std::fs::create_dir_all(classic.join("css")).unwrap();
        std::fs::write(
            classic.join("result.json"),
            "{\n \"id\": 3,\n \"messages\": []\n}",
        )
        .unwrap();
        std::fs::write(classic.join("photos").join("photo_1.jpg"), photo).unwrap();
        std::fs::write(classic.join("photos").join("photo_2.jpg"), b"unique").unwrap();
        std::fs::write(classic.join("css").join("style.css"), photo).unwrap();

        let report = duplicates(&dir).await.unwrap();
        assert_eq!(report.scanned_blobs, 3);
        assert_eq!(report.scanned_files, 2, "css/ is chrome, not media");
        assert_eq!(report.groups.len(), 1, "{:#?}", report.groups);
        let group = &report.groups[0];
        assert_eq!(group.size, photo.len() as u64);
        assert_eq!(
            group.copies,
            vec![
                Place::Blob {
                    db: a,
                    file_id: 10,
                    kind: "photos".into(),
                    file_name: None
                },
                Place::Blob {
                    db: b,
                    file_id: 20,
                    kind: "photos".into(),
                    file_name: None
                },
                Place::File(classic.join("photos").join("photo_1.jpg")),
            ]
        );
        assert_eq!(report.wasted_bytes, 2 * photo.len() as u64);
        let lines = report.lines(20);
        assert!(lines[0].contains("1 held more than once"), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("photo_1.jpg")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
