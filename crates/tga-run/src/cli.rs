//! `TelegramTools.exe analyse <export folder> [options]` — the report, from a
//! command line.
//!
//! Point it at a finished export and it writes one `report.html` beside it;
//! nothing here reaches the network.
//!
//! **This was a second executable twice over** — `tga.exe`, then the
//! analyser's own exe with argv deciding between window and command. It is a
//! subcommand of TelegramTools now, and `args` here is what follows the word
//! `analyse`. Every option below behaves exactly as it did when it had its own
//! exe, which is what lets `save.bat baseline` keep comparing byte for byte
//! across both changes.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

pub const USAGE: &str = "\
TelegramTools analyse -- a report on an export

    TelegramTools analyse <export folder> [options]
    TelegramTools analyse <telegram.sqlite> [options]
    TelegramTools analyse <exporter output folder> --refresh [--force] [options]
    TelegramTools analyse --from-stats <stats.json> --out <report.html> [options]

    `report` and `analyze` are the same command.

    An export is a folder of result.json files, or the exporter's database
    output -- a .sqlite, named directly or found in or one folder below the
    folder given. Both produce the same report from the same figures.

Options
    --refresh         re-analyse every database under the folder -- the
                      exporter writes one per chat, <title>\\<title>.sqlite --
                      whose report is missing or older than the database or
                      its notes. Each report goes beside its database.
    --force           with --refresh, redo every report, changed or not
    --out PATH        where to write the report (default: report.html beside
                      the export)
    --chat ID|TITLE   which chat to read out of a database that holds more
                      than one. Default: the largest, with the others named.
    --digest          also write analysis/digest.jsonl and analysis/EVENTS.md,
                      which is what a model needs to map events onto the
                      timeline
    --no-fonts        do not embed the fonts; smaller file, needs Geist
                      installed to look right
    --handles         show people by @handle rather than by name. For your
                      contacts an export holds the name *you* saved them as;
                      the handle is the one they chose. Somebody with no
                      handle keeps the name in the export.
    --stats PATH      write every computed figure as JSON
    --from-stats PATH re-render from a recorded --stats dump instead of
                      reading an export. No folder is needed and none is
                      opened: `tga-report` depends on neither the reader nor
                      the metrics, so a dump is the whole input. Use --notes to
                      annotate it.
    --notes PATH      read the events file from here rather than from beside
                      the export
    --stamp TEXT      the date the notes section prints, instead of today's.
                      The one value in the report that comes from the clock, so
                      pinning it is what makes two runs on different days
                      comparable byte for byte -- which is what `save.bat
                      baseline` rests on.
    --quiet           no per-file progress
";

/// Run as a command. `args` is what followed `analyse` on the command line.
pub fn run(args: Vec<String>) -> Result<()> {
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print!("{USAGE}");
        return Ok(());
    }

    // The folder is positional and optional: `--from-stats` needs no export,
    // and requiring a placeholder path to something that is never opened would
    // be a lie in the command line.
    let folder: Option<PathBuf> = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .map(PathBuf::from);
    let flags_from = if folder.is_some() { 1 } else { 0 };
    let mut out: Option<PathBuf> = None;
    let mut from_stats: Option<PathBuf> = None;
    let mut notes_path: Option<PathBuf> = None;
    let mut digest = false;
    let mut embed_fonts = true;
    let mut stats_out: Option<PathBuf> = None;
    let mut quiet = false;
    let mut stamp: Option<String> = None;
    let mut chat: Option<String> = None;
    let mut refresh = false;
    let mut force = false;
    let mut handles = false;

    let mut rest = args[flags_from..].iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--out" => match rest.next() {
                Some(path) => out = Some(PathBuf::from(path)),
                None => bail!("--out needs a path"),
            },
            "--chat" => match rest.next() {
                Some(which) => chat = Some(which.clone()),
                None => bail!("--chat needs a chat id or title"),
            },
            "--digest" => digest = true,
            "--no-fonts" => embed_fonts = false,
            "--stats" => match rest.next() {
                Some(path) => stats_out = Some(PathBuf::from(path)),
                None => bail!("--stats needs a path"),
            },
            "--from-stats" => match rest.next() {
                Some(path) => from_stats = Some(PathBuf::from(path)),
                None => bail!("--from-stats needs a path"),
            },
            "--notes" => match rest.next() {
                Some(path) => notes_path = Some(PathBuf::from(path)),
                None => bail!("--notes needs a path"),
            },
            "--stamp" => match rest.next() {
                Some(text) => stamp = Some(text.clone()),
                None => bail!("--stamp needs a date"),
            },
            "--quiet" => quiet = true,
            "--refresh" => refresh = true,
            "--force" => force = true,
            "--handles" => handles = true,
            other => bail!("Unknown option: {other}\n\n{USAGE}"),
        }
    }

    // Built once and shared by both render paths, so `--from-stats` and a
    // straight read cannot drift apart in what they pass the writer.
    let options = tga_report::Options {
        embed_fonts,
        stamp: stamp.unwrap_or_else(tga_report::today_stamp),
        ..Default::default()
    };

    if force && !refresh {
        bail!("--force only means something with --refresh");
    }
    if refresh {
        // Every one of these names a single file or a single chat, and a
        // refresh writes one report per database: there is no one place for
        // any of them to mean.
        if out.is_some() || stats_out.is_some() || chat.is_some() || notes_path.is_some() {
            bail!("--refresh writes one report beside each database; --out, --stats, --chat and --notes name a single one");
        }
        if from_stats.is_some() {
            bail!("--refresh reads databases; --from-stats reads none");
        }
        let Some(folder) = folder else {
            bail!("--refresh needs the folder the exporter writes its databases into");
        };
        return refresh_all(&folder, force, digest, handles, quiet, &options);
    }

    // The notes come from wherever they were named, or from beside the export.
    // A `--notes` path that is not there is an error rather than a shrug: the
    // ordinary "no annotations" case is not passing the flag at all, so naming
    // a file and getting silence would hide a typo.
    let notes = match (&notes_path, &folder) {
        (Some(path), _) => {
            if !path.is_file() {
                bail!("Not a file: {}", path.display());
            }
            tga_notes::load_file(path)
        }
        (None, Some(folder)) => tga_notes::load(folder),
        (None, None) => tga_notes::Notes::default(),
    };

    // -- re-render from a recorded dump, with no export on disk -------------
    if let Some(dump) = &from_stats {
        // A dump already holds whatever names it was recorded with; there is
        // nothing left to swap a handle in from.
        if handles {
            bail!("--handles applies when reading an export; a --from-stats dump keeps the names it was recorded with");
        }
        if stats_out.is_some() || digest {
            bail!(
                "--from-stats has no export to read, so --stats and --digest have nothing to write"
            );
        }
        let Some(out) = out else {
            bail!("--from-stats needs --out: there is no export folder to write beside");
        };
        let text = std::fs::read_to_string(dump)
            .map_err(|e| anyhow::anyhow!("{}: {e}", dump.display()))?;
        // A dump that no longer matches the shape now fails here, by name,
        // instead of rendering a report full of zeroes.
        let stats: tga_stats::Stats =
            serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", dump.display()))?;
        let html = tga_report::render(
            &stats,
            &tga_report::names_from_stats(&stats),
            &notes,
            &options,
        );
        write_utf8(&out, &html)?;
        println!(
            "{} people, {} topics{} (from {})",
            stats.people.speakers,
            stats.export.topics,
            if notes.events.is_empty() {
                String::new()
            } else {
                format!(", {} events", notes.events.len())
            },
            dump.display()
        );
        println!("{}", out.display());
        return Ok(());
    }

    let Some(folder) = folder else {
        bail!(
            "Nothing to read.

{USAGE}"
        );
    };
    // A database is a file, so `is_dir` cannot be the gate any more: the
    // exporter writes `<output_dir>/telegram.sqlite`, and either the folder or
    // the file itself is a reasonable thing to be handed.
    let database = tga_db::find(&folder);
    if database.is_none() && !folder.is_dir() {
        bail!("Not a folder: {}", folder.display());
    }
    // The exporter's output folder, one database per chat below it: not one
    // export, and reading it as a folder export would end on "No result.json".
    if database.is_none() && tga_read::find_results(&folder, 3).is_empty() {
        let all = tga_db::find_all(&folder);
        if all.len() > 1 {
            bail!(
                "{} holds {} chat databases, not one export. Name one, or pass --refresh to report on each.",
                folder.display(),
                all.len()
            );
        }
    }
    // Beside the database, not inside it. A report written next to the file is
    // where somebody handed the folder would look for it either way.
    let beside = match &database {
        Some(db) => db.parent().unwrap_or(&folder).to_path_buf(),
        None => folder.clone(),
    };
    let out = out.unwrap_or_else(|| beside.join("report.html"));

    let mut progress = |done: usize, total: usize, name: &str| {
        if !quiet {
            println!("  read {done}/{total}  {name}");
        }
    };
    let mut export = match &database {
        Some(db) => read_database(db, chat.as_deref(), quiet, &mut progress)?,
        None => tga_read::load(&folder, Some(&mut progress))?,
    };
    if export.msgs.is_empty() {
        bail!("That export has no messages in it.");
    }
    if handles {
        export.handles_only();
    }

    let (stats, people) = tga_metrics::analyse(&export);

    if let Some(path) = &stats_out {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, tga_stats::write(&stats)?)?;
        println!("  {}", path.display());
    }

    if digest {
        let target = folder.join("analysis");
        let rows = digest_rows(&export, &people);
        println!("  {}", tga_notes::write_digest(&rows, &target)?.display());
        println!("  {}", tga_notes::write_brief(&target)?.display());
    }

    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let html = tga_report::render(
        &stats,
        &tga_report::names_from_stats(&stats),
        &notes,
        &options,
    );
    write_utf8(&out, &html)?;

    println!(
        "{} messages, {} people, {} topics{}",
        thousands(export.msgs.len()),
        stats.people.speakers,
        export.topics.len(),
        if notes.events.is_empty() {
            String::new()
        } else {
            format!(", {} events", notes.events.len())
        }
    );
    println!("{}", out.display());
    Ok(())
}

/// Read a chat out of a database export, saying what the format knows and the
/// folder format never could.
///
/// The two extra lines are not decoration. A message Telegram has dropped is
/// still in the figures — that is the point of the format — so a reader who is
/// not told is looking at counts they cannot reconcile with the live chat.
fn read_database(
    db: &Path,
    wanted: Option<&str>,
    quiet: bool,
    progress: tga_read::Progress<'_>,
) -> anyhow::Result<tga_read::Export> {
    let chats = tga_db::chats(db)?;
    let export = tga_db::load(db, wanted, Some(progress))?;

    if !quiet {
        println!("  database {}", db.display());
        let id = export.topics.first().and_then(|t| t.chat_id);
        if let Some(id) = id {
            let deleted = tga_db::deleted(db, id).unwrap_or(0);
            let (edited, versions) = tga_db::edits(db, id).unwrap_or((0, 0));
            if deleted > 0 {
                println!("  {deleted} deleted on Telegram, kept, and counted here");
            }
            if edited > 0 {
                println!("  {edited} edited, {versions} earlier versions kept (not yet read)");
            }
        }
        // The exporter is moving to one database per chat; until then, a file
        // with several is read as its largest and the rest are named rather
        // than silently passed over.
        if chats.len() > 1 {
            println!(
                "  this database holds {} chats; the others were not read:",
                chats.len()
            );
            for other in chats.iter().filter(|c| Some(c.id) != id) {
                println!(
                    "    --chat {}  {}  ({} messages)",
                    other.id, other.title, other.messages
                );
            }
        }
    }
    Ok(export)
}

/// `--refresh`: a report beside every database under `folder` that has moved on
/// since its last one.
///
/// One that fails is named and the rest carry on — sixteen chats should not
/// hang on the one whose file is mid-sync — but the exit status still says so,
/// because a scheduled refresh that half-worked must not read as one that did.
fn refresh_all(
    folder: &Path,
    force: bool,
    digest: bool,
    handles: bool,
    quiet: bool,
    options: &tga_report::Options,
) -> Result<()> {
    let plan = crate::refresh::plan(folder);
    if plan.is_empty() {
        bail!(
            "No .sqlite in {} or the folders directly inside it.",
            folder.display()
        );
    }
    let (mut written, mut current, mut failed) = (0, 0, 0);
    for target in &plan {
        let name = target.name();
        if !target.stale && !force {
            current += 1;
            if !quiet {
                println!("  {name}  up to date");
            }
            continue;
        }
        let mut progress = |done: usize, total: usize, topic: &str| {
            if !quiet {
                println!("  {name}  read {done}/{total}  {topic}");
            }
        };
        match crate::refresh::one(target, digest, handles, options, Some(&mut progress)) {
            Ok(done) => {
                written += 1;
                println!(
                    "  {name}  {} messages, {} people, {} topics{} -> {}",
                    thousands(done.messages),
                    done.people,
                    done.topics,
                    if done.events == 0 {
                        String::new()
                    } else {
                        format!(", {} events", done.events)
                    },
                    done.out.display()
                );
                if let Some(warning) = done.warning {
                    println!("  {name}  {warning}");
                }
            }
            Err(e) => {
                failed += 1;
                eprintln!("  {name}  failed: {e:#}");
            }
        }
    }
    println!("{written} written, {current} up to date, {failed} failed");
    if failed > 0 {
        bail!(
            "{failed} of {} databases could not be reported on",
            plan.len()
        );
    }
    Ok(())
}

/// The digest's view of the export.
///
/// This mapping lives here rather than in `tga-notes`, because that crate must
/// not depend on `tga-read` — it carries the `Event` type that `tga-report`
/// renders, and `tga-report` must not reach the reader. Four lines is the whole
/// price of the rule.
///
/// `pub(crate)` because the window writes digests too; it used to keep its own
/// identical copy, which is what two crates cost and one does not.
pub(crate) fn digest_rows(
    export: &tga_read::Export,
    people: &tga_metrics::People,
) -> Vec<tga_notes::Row> {
    export
        .msgs
        .iter()
        .map(|msg| tga_notes::Row {
            id: msg.id,
            t: msg.when.format("%Y-%m-%d %H:%M").to_string(),
            topic: msg.topic,
            who: if msg.sender.is_empty() && msg.name.is_empty() {
                String::new()
            } else {
                people.name_of(&people.key_of(msg))
            },
            service: if msg.service {
                Some(msg.action.clone())
            } else {
                None
            },
            text: tga_notes::digest::squeeze(&msg.text),
            re: msg.reply_to,
            media: msg.media.clone(),
            doc: attachment(export, msg),
            reactions: if msg.reactions.is_empty() {
                None
            } else {
                Some(msg.reaction_total())
            },
        })
        .collect()
}

/// Read the document a message carries, if it carries one this build can open.
///
/// Sits here for the same reason `digest_rows` does, one rule further out:
/// `tga-notes` may not depend on `tga-docs` either, because `tga-report`
/// depends on `tga-notes` and has no business linking a PDF parser. So the
/// caller does the reading and hands over strings.
///
/// The cost is only paid on the `--digest` path. Reading seven hundred
/// documents is not something a report should do -- it does not use them, and
/// `--from-stats` renders one with no export on disk at all.
fn attachment(export: &tga_read::Export, msg: &tga_read::Msg) -> Option<tga_notes::Attachment> {
    if msg.file.is_empty() || !tga_docs::is_document(&msg.file_name) {
        return None;
    }
    // `file` is relative to the topic's own folder, not to the export root.
    let path = export.topics.get(msg.topic)?.folder.join(&msg.file);
    let posted = msg.when.date();

    // A document that could not be opened still gets a row: the name and the
    // date are most of the value, and a scanned PDF -- a photograph of a page,
    // with no text layer -- is common enough that dropping it would quietly
    // hide a whole class of shared paperwork.
    let doc = tga_docs::read(&path, &msg.file_name, posted);
    let dates = doc
        .as_ref()
        .map(|d| d.dates.clone())
        .unwrap_or_else(|| tga_docs::dates::infer(&msg.file_name, "", posted));

    let day = |d: Option<chrono::NaiveDate>| d.map(|d| d.to_string()).unwrap_or_default();
    Some(tga_notes::Attachment {
        name: msg.file_name.clone(),
        date: day(dates.best),
        date_src: dates.src.as_str().to_string(),
        from: day(dates.from),
        to: day(dates.to),
        n: dates.n,
        text: doc.as_ref().map(|d| d.text.clone()).unwrap_or_default(),
        cut: doc.as_ref().is_some_and(|d| d.cut),
    })
}

/// Write UTF-8 with `\n` line endings, as the Python's `newline="\n"` does.
///
/// `std::fs::write` already does exactly this on every platform — the note is
/// here because the Python had to ask for it, and somebody comparing the two
/// will look for the equivalent.
fn write_utf8(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
