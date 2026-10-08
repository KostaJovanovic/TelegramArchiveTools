//! **What is wrong with an export folder, and what can be put right.**
//!
//! Offline, and read-only unless asked: [`examine`] looks and reports, and
//! only [`repair`] changes anything, and only what a finding offers to fix.
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
                     it. Split it with: tgx split \"{}\"",
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
}
