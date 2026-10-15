//! The exporter's commands: `TelegramTools login|chats|export|…`.
//!
//! The window is the product, but this exists for a specific reason: it is the
//! only way to exercise the engine against a real account without a window, and
//! `convert.rs` is the one module in the pipeline no parity harness can verify.
//! Everything downstream of the wire is pinned byte for byte; this is how the
//! wire itself gets tested.
//!
//! ```text
//! TelegramTools login
//! TelegramTools chats
//! TelegramTools export "UA KOLAB TELEGRAM"
//! ```
//!
//! **This was its own binary, `tgx.exe`**, built from `tgx-tg`. It is the same
//! code behind the one executable's argv now; `main` hands it everything that
//! is not `analyse`. What changed is only that it no longer owns `main`: the
//! runtime is built here, and the log was already opened by the caller.

use anyhow::{anyhow, Result};
use std::io::Write;
use tgx_tg::cancel::Cancel;
use tgx_tg::client::{ChatInfo, Migration, Session};
use tgx_tg::config::Settings;
use tgx_tg::dialogs;
use tgx_tg::engine::{ChatExporter, ExportResult, Progress};

/// Run one exporter command. `args` is argv with the program name dropped.
pub fn run(args: Vec<String>) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(command(args))
}

async fn command(args: Vec<String>) -> Result<()> {
    // The same check `actions::report_data_dir_protection` makes for the window.
    // `login` writes a bearer credential into this folder, and this is the
    // surface most likely to be run from a stick or a shared machine — the two
    // places the ACL is most likely to have failed. Saying nothing here left the
    // GUI as the only thing that ever mentioned it.
    if let Some(why) = tgx_tg::config::lockdown_error() {
        eprintln!(
            "warning: could not restrict {} to your user ({why}).\n\
             Anyone with access to this machine or drive can read the saved \
             session and sign in as you.",
            tgx_tg::config::data_dir().display()
        );
    }

    let cmd = args.first().map(String::as_str).unwrap_or("help");

    let mut settings = Settings::load();
    // Credentials may come from the environment so a shell session can drive
    // this without writing them to disk first.
    if let Ok(v) = std::env::var("TG_API_ID") {
        // Not `unwrap_or(existing)`: a typo'd TG_API_ID silently fell back to
        // whatever settings.json held, so the run either used the wrong account
        // or failed at the wire with an error naming neither the variable nor
        // the value. Someone who sets a variable is asserting it.
        match v.trim().parse::<i64>() {
            Ok(id) => settings.api_id = id,
            Err(e) => return Err(anyhow!("TG_API_ID={v:?} is not a number: {e}")),
        }
    }
    if let Ok(v) = std::env::var("TG_API_HASH") {
        settings.api_hash = v;
    }

    if writes_beside_the_window(&args) {
        refuse_beside_the_window()?;
    }

    match cmd {
        "login" => login(&settings).await,
        "logout" => {
            let done = tgx_tg::client::sign_out(&settings).await;
            if let Err(e) = &done.revoked {
                eprintln!(
                    "warning: Telegram did not confirm the sign-out ({e}).\n\
                     End the session from Telegram -> Settings -> Devices."
                );
            }
            done.removed
                .map_err(|e| anyhow!("could not delete the saved session: {e}"))?;
            println!("Signed out.");
            Ok(())
        }
        "chats" => match args.get(1).map(String::as_str) {
            None => chats(&settings, false).await,
            Some("--json") => chats(&settings, true).await,
            Some(other) => Err(anyhow!("unknown flag {other:?}\n\n{USAGE}")),
        },
        "sync" => {
            // A database sync and nothing else, whatever the settings file
            // says about folders — see `export --db` below for why the flag
            // does not rewrite the file.
            settings.export_db = true;
            settings.date_from.clear();
            settings.date_until.clear();
            let mut only = Vec::new();
            let mut json = false;
            let mut rest = args.iter().skip(1);
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--full" => settings.reread_all = true,
                    "--json" => json = true,
                    "--only" => only.push(
                        rest.next()
                            .ok_or_else(|| anyhow!("--only needs a chat id or title"))?
                            .clone(),
                    ),
                    f if f.starts_with("--") => {
                        return Err(anyhow!("unknown flag {f:?}\n\n{USAGE}"))
                    }
                    // The folder to sync is the folder the run writes into:
                    // reading the databases from one place and writing them
                    // to another would start fresh files beside the old.
                    folder => settings.output_dir = folder.into(),
                }
            }
            sync(&settings, &only, json).await
        }
        "export" => {
            // The flags are read off the whole tail rather than positionally,
            // so `export --db "Dev Team"` and `export "Dev Team" --db`
            // both work. A title starting with `--` is not a thing this tool
            // has to support.
            //
            // **The date flags replace the settings file's dates**, and a run
            // with neither ignores them: the window's From and Until are a
            // setting someone may have left behind, and a command line that
            // quietly exported one month of a chat because of it would be the
            // window deciding what a script gets.
            settings.date_from.clear();
            settings.date_until.clear();
            let mut want = None;
            let mut rest = args.iter().skip(1).map(String::as_str);
            while let Some(a) = rest.next() {
                match a {
                    // Database *mode*: it replaces the folder export, it
                    // does not join it. `export_html` and `export_json` are
                    // left as they are and simply ignored, so the flag does not
                    // quietly rewrite the settings file for the next run.
                    "--db" => settings.export_db = true,
                    "--full" => settings.reread_all = true,
                    "--since" | "--until" => {
                        let day = rest
                            .next()
                            .ok_or_else(|| anyhow!("{a} needs a date like 2026-01-31"))?;
                        if a == "--since" {
                            settings.date_from = day.into();
                        } else {
                            settings.date_until = day.into();
                        }
                    }
                    other if other.starts_with("--") => {
                        return Err(anyhow!("unknown flag {other:?}\n\n{USAGE}"))
                    }
                    title if want.is_none() && !title.trim().is_empty() => {
                        want = Some(title.trim())
                    }
                    _ => {}
                }
            }
            if settings.export_db
                && !(settings.date_from.is_empty() && settings.date_until.is_empty())
            {
                return Err(anyhow!(
                    "--since and --until are for a folder export; a database keeps \
                     the whole chat"
                ));
            }
            settings.date_range().map_err(|e| anyhow!(e))?;
            let want = want.ok_or_else(|| anyhow!("usage: {EXPORT_USAGE}"))?;
            export(&settings, want).await
        }
        "compare" => {
            // `--db <dir>` and `--csv <file>` take a value; everything else is
            // an export folder. The database folder defaults to the one the
            // settings write to, which is where the window's databases are.
            let mut db = std::path::PathBuf::from(&settings.output_dir);
            let mut csv = None;
            let mut roots = Vec::new();
            let mut rest = args.iter().skip(1);
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--db" => {
                        db = rest
                            .next()
                            .ok_or_else(|| anyhow!("--db needs a folder"))?
                            .into()
                    }
                    "--csv" => {
                        csv = Some(std::path::PathBuf::from(
                            rest.next().ok_or_else(|| anyhow!("--csv needs a file"))?,
                        ))
                    }
                    f if f.starts_with("--") => {
                        return Err(anyhow!("unknown flag {f:?}\n\n{USAGE}"))
                    }
                    folder => roots.push(std::path::PathBuf::from(folder)),
                }
            }
            if roots.is_empty() {
                return Err(anyhow!("usage: {COMPARE_USAGE}"));
            }
            compare(&roots, &db, csv.as_deref()).await
        }
        "doctor" => {
            let mut dir = std::path::PathBuf::from(&settings.output_dir);
            let mut fix = false;
            let mut dupes = false;
            for a in args.iter().skip(1) {
                match a.as_str() {
                    "--fix" => fix = true,
                    "--dupes" => dupes = true,
                    f if f.starts_with("--") => {
                        return Err(anyhow!("unknown flag {f:?}\n\n{USAGE}"))
                    }
                    folder => dir = folder.into(),
                }
            }
            // A report has nothing to fix; asking for both is a mistake worth
            // naming rather than quietly running one of them.
            if dupes && fix {
                return Err(anyhow!(
                    "--dupes only reports; it has no --fix. Run them separately."
                ));
            }
            if dupes {
                return doctor_dupes(&dir).await;
            }
            doctor(&dir, fix).await
        }
        "split" => {
            let mut file = None;
            let mut into = None;
            let mut dry_run = false;
            let mut rest = args.iter().skip(1);
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--into" => {
                        into = Some(std::path::PathBuf::from(
                            rest.next()
                                .ok_or_else(|| anyhow!("--into needs a folder"))?,
                        ))
                    }
                    "--dry-run" => dry_run = true,
                    f if f.starts_with("--") => {
                        return Err(anyhow!("unknown flag {f:?}\n\n{USAGE}"))
                    }
                    path => file = Some(std::path::PathBuf::from(path)),
                }
            }
            let file = file.ok_or_else(|| anyhow!("usage: {SPLIT_USAGE}"))?;
            // Beside the file by default: that is where its chats' databases
            // already are, when there are any.
            let into = match into {
                Some(d) => d,
                None => file
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_default(),
            };
            split(&file, &into, dry_run).await
        }
        "search" => {
            // One positional is the text; two are a folder and the text. The
            // folder defaults to the one the settings write to.
            let mut limit = 50usize;
            let mut words = Vec::new();
            let mut rest = args.iter().skip(1);
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--limit" => {
                        let n = rest
                            .next()
                            .ok_or_else(|| anyhow!("--limit needs a number"))?;
                        limit = n
                            .parse()
                            .map_err(|e| anyhow!("--limit {n:?} is not a number: {e}"))?;
                    }
                    f if f.starts_with("--") => {
                        return Err(anyhow!("unknown flag {f:?}\n\n{USAGE}"))
                    }
                    w => words.push(w.to_string()),
                }
            }
            let (dir, needle) = match words.as_slice() {
                [text] => (std::path::PathBuf::from(&settings.output_dir), text.clone()),
                [dir, text] => (std::path::PathBuf::from(dir), text.clone()),
                _ => return Err(anyhow!("usage: {SEARCH_USAGE}")),
            };
            if needle.trim().is_empty() {
                return Err(anyhow!("nothing to search for"));
            }
            search(&dir, &needle, limit).await
        }
        "unpack" => {
            let mut paths = Vec::new();
            for a in args.iter().skip(1) {
                match a.as_str() {
                    // For this run only, like `export --db`: the formats
                    // otherwise come from settings.json.
                    "--no-html" => settings.export_html = false,
                    "--no-json" => settings.export_json = false,
                    f if f.starts_with("--") => {
                        return Err(anyhow!("unknown flag {f:?}\n\n{USAGE}"))
                    }
                    path => paths.push(std::path::PathBuf::from(path)),
                }
            }
            let [db, out] =
                <[_; 2]>::try_from(paths).map_err(|_| anyhow!("usage: {UNPACK_USAGE}"))?;
            unpack(&db, &out, &settings).await
        }
        "import" => {
            let mut db = std::path::PathBuf::from(&settings.output_dir);
            let mut dry_run = false;
            let mut roots = Vec::new();
            let mut rest = args.iter().skip(1);
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--db" => {
                        db = rest
                            .next()
                            .ok_or_else(|| anyhow!("--db needs a folder"))?
                            .into()
                    }
                    "--dry-run" => dry_run = true,
                    f if f.starts_with("--") => {
                        return Err(anyhow!("unknown flag {f:?}\n\n{USAGE}"))
                    }
                    folder => roots.push(std::path::PathBuf::from(folder)),
                }
            }
            if roots.is_empty() {
                return Err(anyhow!("usage: {IMPORT_USAGE}"));
            }
            import(&roots, &db, dry_run).await
        }
        _ => {
            println!("{USAGE}");
            Ok(())
        }
    }
}

/// `doctor`: what is wrong with an export folder, and with `--fix`, put
/// right what can be.
async fn doctor(dir: &std::path::Path, fix: bool) -> Result<()> {
    let found = tgx_tg::doctor::examine(dir).await?;
    println!("{}: {} finding(s)", dir.display(), found.len());
    for f in &found {
        println!("  {}", f.line());
    }
    let fixable = found.iter().filter(|f| f.fix.is_some()).count();
    if fix {
        for line in tgx_tg::doctor::repair(dir, &found) {
            println!("  {line}");
        }
    } else if fixable > 0 {
        println!("{fixable} can be fixed: run again with --fix (with no export running)");
    }
    Ok(())
}

/// `search`: every message in the folder's databases that says `needle`.
///
/// **The one command here that prints message text**, because finding it is
/// what was asked for — to this terminal only. Nothing of it goes to
/// `log::`, which writes `tgx.log` beside the executable.
async fn search(dir: &std::path::Path, needle: &str, limit: usize) -> Result<()> {
    let found = tgx_tg::search::across(dir, needle, limit).await?;
    for c in &found.chats {
        println!();
        println!("{} (id {}) — {} hit(s)", c.title, c.chat_id, c.hits.len());
        println!("  database: {}", c.path.display());
        for h in &c.hits {
            let mut flags = Vec::new();
            if h.deleted {
                flags.push("deleted");
            }
            if h.in_earlier_version {
                flags.push("earlier version");
            }
            let flags = if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(", "))
            };
            println!(
                "  {:>8}  {}{flags}  {}",
                h.id,
                h.date.replacen('T', " ", 1),
                excerpt(&h.excerpt, needle)
            );
        }
        if c.hits.len() >= limit {
            println!("  … stopped at {limit}; --limit raises it");
        }
    }
    for (path, why) in &found.unreadable {
        println!("skipped {}: {why}", path.display());
    }
    println!();
    println!(
        "{} hit(s) in {} chat(s) under {}",
        found.total(),
        found.chats.len(),
        dir.display()
    );
    Ok(())
}

/// One line of `text` around the first place `needle` occurs, on one line.
fn excerpt(text: &str, needle: &str) -> String {
    const SIDE: usize = 40;
    let flat: Vec<char> = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let lower: Vec<char> = flat.iter().flat_map(|c| c.to_lowercase()).collect();
    let want: Vec<char> = needle.trim().to_lowercase().chars().collect();
    // Located in the folded text, which is the same length as the original
    // for every script these chats are in; where folding changes the length
    // the window is merely a few characters off.
    let at = lower
        .windows(want.len().max(1))
        .position(|w| w == want.as_slice())
        .unwrap_or(0)
        .min(flat.len());
    let from = at.saturating_sub(SIDE);
    let to = (at + want.len() + SIDE).min(flat.len());
    let mut out: String = flat[from..to].iter().collect();
    if from > 0 {
        out.insert(0, '…');
    }
    if to < flat.len() {
        out.push('…');
    }
    out
}

/// `doctor --dupes`: media stored more than once under `dir`, the
/// largest waste first. Changes nothing.
async fn doctor_dupes(dir: &std::path::Path) -> Result<()> {
    let report = tgx_tg::doctor::duplicates(dir).await?;
    println!("{}:", dir.display());
    for line in report.lines(20) {
        println!("{line}");
    }
    Ok(())
}

/// `split`: one database per chat out of a file holding several.
async fn split(file: &std::path::Path, into: &std::path::Path, dry_run: bool) -> Result<()> {
    if dry_run {
        for p in tgx_tg::split::plan(file, into).await? {
            let how = if p.merge { "merge into" } else { "new file" };
            println!(
                "{} (id {}) → {how} {}",
                p.title,
                p.chat_id,
                p.dest.display()
            );
        }
        println!("nothing written (--dry-run)");
        return Ok(());
    }
    let done = tgx_tg::split::split(file, into).await?;
    let mut whole = true;
    for d in &done {
        let p = &d.planned;
        match &d.lacking {
            Ok(0) => println!("{} (id {}) → {}", p.title, p.chat_id, p.dest.display()),
            Ok(n) => {
                whole = false;
                println!(
                    "{} (id {}): {n} message(s) did NOT come across",
                    p.title, p.chat_id
                )
            }
            Err(e) => {
                whole = false;
                println!("{} (id {}): FAILED: {e}", p.title, p.chat_id)
            }
        }
    }
    if whole {
        println!(
            "every chat came across whole; the original is now {}.split",
            file.display()
        );
    } else {
        println!("the original is left as it was");
    }
    Ok(())
}

/// `unpack`: a database back out as a Classic folder, offline.
///
/// Counts only, like `compare`: what was written is in the folder.
async fn unpack(db: &std::path::Path, out: &std::path::Path, settings: &Settings) -> Result<()> {
    let done = tgx_tg::unpack::write_classic(db, out, settings).await?;
    for d in &done {
        println!("{} (id {}) → {}", d.title, d.chat_id, d.root.display());
        println!(
            "  {} message(s) in {} folder(s), {} of them deleted on Telegram (kept)",
            d.messages, d.topics, d.deleted
        );
        println!("  {} file(s) written", d.files);
        if d.missing > 0 {
            println!(
                "  {} file(s) the messages name are not in the database — over its size \
                 ceiling, or never fetched",
                d.missing
            );
        }
        if d.refused > 0 {
            println!(
                "  {} file(s) REFUSED: their recorded path leads outside the export folder",
                d.refused
            );
        }
    }
    Ok(())
}

/// `import`: copy what only old exports hold into the databases.
///
/// Prints ids and counts only, like `compare`.
async fn import(roots: &[std::path::PathBuf], db: &std::path::Path, dry_run: bool) -> Result<()> {
    use tgx_tg::compare;
    use tgx_tg::import::Outcome;
    let files = compare::find_results(roots, 4);
    println!("reading {} result.json file(s)…", files.len());
    let (exports, mut skipped) = compare::read_exports(&files);
    let (verdicts, unreadable) = compare::compare(exports, db).await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (done, more) = tgx_tg::import::import(&verdicts, &files, dry_run, now).await;
    skipped.extend(more);

    let verb = if dry_run { "would import" } else { "imported" };
    let mut total = 0;
    for c in &done {
        println!();
        println!("{} (id {})", c.title, c.chat_id);
        println!("  database: {}", c.database.display());
        match &c.outcome {
            Outcome::Imported { messages } => {
                total += messages;
                println!("  {verb} {messages} message(s), marked deleted and recovered");
            }
            Outcome::NotSynced => println!(
                "  skipped — this database never finished a sync, so a message it lacks may \
                 just not be read yet; sync it first"
            ),
            Outcome::Failed(e) => println!("  FAILED: {e}"),
        }
    }
    if done.is_empty() {
        println!("nothing to import — no database lacks a message these exports hold");
    } else {
        println!();
        println!("{verb} {total} message(s) in total. Media bytes are not imported: the export folder still holds those files.");
    }
    for (path, why) in skipped.iter().chain(&unreadable) {
        println!("skipped {}: {why}", path.display());
    }
    Ok(())
}

/// `compare`: what old exports hold that the databases do not.
///
/// Offline, and it prints ids and dates only — see `tgx_tg::compare`.
async fn compare(
    roots: &[std::path::PathBuf],
    db: &std::path::Path,
    csv: Option<&std::path::Path>,
) -> Result<()> {
    use tgx_tg::compare;
    let files = compare::find_results(roots, 4);
    println!(
        "reading {} result.json file(s) under {} folder(s)…",
        files.len(),
        roots.len()
    );
    let (mut exports, mut skipped) = compare::read_exports(&files);

    // HTML-only exports carry no chat id, so they join by name.
    let html_dirs = compare::find_html(roots, 4);
    let mut html = Vec::new();
    for dir in &html_dirs {
        match compare::read_html(dir) {
            Ok(e) => html.push(e),
            Err(e) => skipped.push((dir.clone(), e)),
        }
    }
    if !html_dirs.is_empty() {
        println!(
            "reading {} HTML-only export(s), matched to databases by chat name…",
            html_dirs.len()
        );
    }
    let (matched, unmatched) = compare::match_titles(html, db).await?;
    for (id, e) in matched {
        let into = exports.entry(id).or_default();
        if into.title.is_empty() {
            into.title = e.title;
        }
        into.files.extend(e.files);
        for (k, v) in e.messages {
            into.messages.entry(k).or_insert(v);
        }
    }
    let (verdicts, unreadable) = compare::compare(exports, db).await?;

    let mut total_missing = 0;
    let mut no_db = Vec::new();
    for v in &verdicts {
        let Some(path) = &v.database else {
            no_db.push(v);
            continue;
        };
        total_missing += v.missing.len();
        println!();
        println!("{} (id {})", v.title, v.chat_id);
        for f in &v.export_files {
            println!("  export:   {}", f.display());
        }
        println!("  database: {}", path.display());
        println!(
            "  {} in the export(s), {} in the database ({} since deleted on Telegram)",
            v.exported, v.stored, v.stored_deleted
        );
        if v.stored == 0 && v.exported > 0 {
            // Not 49,780 deletions: a database that holds nothing for this
            // chat never finished a sync, and every message would read as
            // missing.
            println!(
                "  the database holds NO messages for this chat — it never finished a \
                 sync; run it before trusting a comparison"
            );
        } else if v.missing.is_empty() {
            println!("  nothing missing — every exported message is in the database");
        } else {
            let services = v.missing.iter().filter(|m| m.kind == "service").count();
            let span = compare::date_span(&v.missing)
                .map(|(a, b)| format!(", {a} … {b}"))
                .unwrap_or_default();
            println!(
                "  MISSING from the database: {} ({} messages, {} service){span}",
                v.missing.len(),
                v.missing.len() - services,
                services
            );
            let ids: Vec<String> = v
                .missing
                .iter()
                .take(12)
                .map(|m| m.id.to_string())
                .collect();
            let more = if v.missing.len() > 12 { ", …" } else { "" };
            println!("    ids: {}{more}", ids.join(", "));
        }
        if v.only_in_database > 0 {
            println!(
                "  {} in the database inside the export's range but in no export",
                v.only_in_database
            );
        }
    }

    if !no_db.is_empty() {
        println!();
        println!("no database in {} for:", db.display());
        for v in no_db {
            println!(
                "  {} (id {}) — {} messages, {}",
                v.title,
                v.chat_id,
                v.exported,
                v.export_files
                    .first()
                    .map(|f| f.display().to_string())
                    .unwrap_or_default()
            );
        }
    }
    if !unmatched.is_empty() {
        println!();
        println!("HTML-only exports whose chat name no database has:");
        for e in &unmatched {
            let span = compare::date_span(&e.messages.values().cloned().collect::<Vec<_>>())
                .map(|(a, b)| format!(", {a} … {b}"))
                .unwrap_or_default();
            println!(
                "  {} — {} messages{span}, {}",
                e.title,
                e.messages.len(),
                e.files
                    .first()
                    .and_then(|f| f.parent())
                    .map(|f| f.display().to_string())
                    .unwrap_or_default()
            );
        }
    }
    if !skipped.is_empty() || !unreadable.is_empty() {
        println!();
        println!("skipped:");
        for (path, why) in skipped.iter().chain(&unreadable) {
            println!("  {}: {why}", path.display());
        }
    }

    if let Some(csv) = csv {
        let mut out = String::from("chat_id,chat,message_id,type,date\r\n");
        for v in &verdicts {
            for m in &v.missing {
                out.push_str(&format!(
                    "{},\"{}\",{},{},{}\r\n",
                    v.chat_id,
                    v.title.replace('"', "\"\""),
                    m.id,
                    m.kind,
                    m.date
                ));
            }
        }
        std::fs::write(csv, out)?;
        println!();
        println!(
            "wrote {total_missing} missing message(s) to {}",
            csv.display()
        );
    }
    println!();
    println!(
        "{} chat(s) compared, {total_missing} message(s) only in the exports",
        verdicts.len()
    );
    Ok(())
}

// Each command's one line, for the error a missing argument gets. Kept here
// together, and tested, because five of them once lost the space after the
// program's name in a rename and told people to run `TelegramToolssplit`.
const EXPORT_USAGE: &str =
    "TelegramTools export [--db] [--full] [--since YYYY-MM-DD] [--until YYYY-MM-DD] <chat title>";
const COMPARE_USAGE: &str =
    "TelegramTools compare <export folder>... [--db <folder>] [--csv <file>]";
const SPLIT_USAGE: &str = "TelegramTools split <file.sqlite> [--into <folder>] [--dry-run]";
const SEARCH_USAGE: &str =
    "TelegramTools search [<folder>] <text> [--limit N] — quote text with spaces";
const UNPACK_USAGE: &str =
    "TelegramTools unpack <file.sqlite> <out folder> [--no-html] [--no-json]";
const IMPORT_USAGE: &str = "TelegramTools import <export folder>... [--db <folder>] [--dry-run]";

pub const USAGE: &str = "\
TelegramTools -- export Telegram chats, and report on what was exported

  With no arguments the window opens. From cmd.exe, run an interactive
  command (login) as `start /wait TelegramTools login`: the exe is a window
  program, and cmd does not wait for one unless told to.

  TelegramTools login            sign in (once; the session is saved)
  TelegramTools logout           end the session at Telegram and delete it here
  TelegramTools chats [--json]   list every chat this account can see; --json
                       prints id, title, kind and the database each already
                       has, as one JSON array
  TelegramTools export [--db] [--full] [--since DAY] [--until DAY] <id|title>
                       export the chat with that id, or whose title matches;
                       --db syncs its database instead of writing folders,
                       --full re-reads the whole history rather than the
                       newest few hundred; --since and --until (2026-01-31,
                       both inclusive) limit a folder export to those days
  TelegramTools sync [<folder>] [--only <id|title>]... [--full] [--json]
                       bring every database already in the folder (default:
                       the export destination) up to date, one chat after
                       another; --only narrows it; --json prints one JSON
                       summary on stdout and narrates on stderr. Exit code 1
                       if any chat failed or came through incomplete. Needs
                       the window closed
  TelegramTools analyse <export> [options]
                       offline: write a report on an export or a database;
                       `TelegramTools analyse --help` for its options
  TelegramTools summarise <export> --from <day> [--to <day>] [options]
                       a local model's summary of a date range (Gemma on
                       127.0.0.1, already running); `--help` for its options
  TelegramTools compare <export folder>... [--db <folder>] [--csv <file>]
                       offline: list messages old Desktop-format exports hold
                       that the databases do not (ids and dates only); an
                       HTML-only export is matched to a database by chat name
  TelegramTools doctor [<folder>] [--fix]
                       offline: what is wrong with an export folder — misplaced
                       or misnamed databases, leftovers of runs that did not
                       finish; --fix moves and deletes only those leftovers
  TelegramTools doctor [<folder>] --dupes
                       offline: media held more than once across the databases
                       and Classic export folders, largest waste first;
                       reports only, deletes nothing
  TelegramTools split <file.sqlite> [--into <folder>] [--dry-run]
                       offline: one database per chat out of an older file
                       holding several; merges into an existing one (whose
                       contents win) and renames the original to .split
  TelegramTools import <export folder>... [--db <folder>] [--dry-run]
                       offline: copy those messages into the databases, marked
                       deleted and recovered (only into a database that has
                       finished a sync)
  TelegramTools search [<folder>] <text> [--limit N]
                       offline: every message in the folder's databases whose
                       text contains <text>, any case — earlier versions and
                       deleted messages included, and flagged; N per chat
                       (default 50). Prints the matching text here, never to
                       the log
  TelegramTools unpack <file.sqlite> <out folder> [--no-html] [--no-json]
                       offline: write a database back out as the Classic folder
                       a run would have made — one per topic for a forum,
                       deleted messages included, media from the database

  export flags:
    --db      Database mode instead of folders: sync into the chat's own
              Exports/<chat>/<chat>.sqlite, media included
    --full    with --db, walk the whole history rather than the trailing
              window — needed for older deletions and for files an earlier
              run skipped

Credentials come from TelegramToolsData/settings.json, or from the
TG_API_ID and TG_API_HASH environment variables.";

fn prompt(label: &str) -> Result<String> {
    print!("{label}: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Like `prompt`, but for the two-factor password: it must not land in
/// terminal scrollback, so the console's echo is turned off for the read.
/// Windows-only — this project targets Windows first (see CLAUDE.md) — and
/// implemented with a direct `kernel32` call rather than pulling in a crate
/// like `rpassword` for three functions worth of FFI. `windows-sys` is
/// already in the dependency tree transitively, but not as a dependency of
/// this crate, so declaring it here would still be a new line in
/// `Cargo.toml` for one feature; a hand-written `extern "system"` block is
/// smaller than that.
#[cfg(windows)]
fn prompt_hidden(label: &str) -> Result<String> {
    use std::os::raw::c_void;

    const STD_INPUT_HANDLE: i32 = -10;
    const ENABLE_ECHO_INPUT: u32 = 0x0004;
    const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(std_handle: i32) -> *mut c_void;
        fn GetConsoleMode(console_handle: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(console_handle: *mut c_void, mode: u32) -> i32;
    }

    print!("{label}: ");
    std::io::stdout().flush()?;

    // Piped input (no console attached, e.g. under CI) has no mode to read —
    // fall through and read normally rather than failing the login.
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    let mut original_mode: u32 = 0;
    let echo_was_disabled = handle != INVALID_HANDLE_VALUE
        && !handle.is_null()
        && unsafe { GetConsoleMode(handle, &mut original_mode) } != 0
        && unsafe { SetConsoleMode(handle, original_mode & !ENABLE_ECHO_INPUT) } != 0;

    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);

    // Restore before the `?` below, so a read error cannot leave the console
    // permanently silent.
    if echo_was_disabled {
        unsafe { SetConsoleMode(handle, original_mode) };
    }
    // The console never saw the Enter key's newline echoed, since echo was
    // off for the whole line — print it ourselves so the next prompt does
    // not run on the same line.
    println!();

    read?;
    Ok(line.trim().to_string())
}

#[cfg(not(windows))]
fn prompt_hidden(label: &str) -> Result<String> {
    prompt(label)
}

async fn login(settings: &Settings) -> Result<()> {
    let mut session = Session::connect(settings).await?;
    if session.is_authorized().await? {
        println!("already signed in as {}", session.me().await?);
        return Ok(());
    }
    let phone = if settings.phone.is_empty() {
        prompt("phone (with country code)")?
    } else {
        settings.phone.clone()
    };
    session.request_code(&phone).await?;
    let code = prompt("login code")?;
    use tgx_tg::client::LoginStep;
    match session.sign_in(&code).await? {
        LoginStep::Ready => {}
        LoginStep::NeedPassword => {
            let password = prompt_hidden("two-factor password")?;
            session.check_password(&password).await?;
        }
        LoginStep::NeedCode => return Err(anyhow!("Telegram wanted another code")),
    }
    println!("signed in as {}", session.me().await?);
    Ok(())
}

async fn connected(settings: &Settings) -> Result<Session> {
    let session = Session::connect(settings).await?;
    if !session.is_authorized().await? {
        return Err(anyhow!("not signed in — run `TelegramTools login` first"));
    }
    Ok(session)
}

async fn chats(settings: &Settings, json: bool) -> Result<()> {
    let session = connected(settings).await?;
    let mut list = dialogs::list_chats(&session.client)
        .await
        .map_err(|e| anyhow!("listing chats: {e}"))?;
    list.sort_by_key(|c| -c.last_activity);
    if json {
        // With the database each chat already has, if any, so a caller can
        // tell what `sync` will cover without a second command. A destination
        // that does not exist yet is "none", not an error.
        let archived = tgx_tg::engine::archived_chats(std::path::Path::new(&settings.output_dir))
            .await
            .unwrap_or_default();
        let rows: Vec<_> = list
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id,
                    "title": c.title,
                    "kind": c.kind.label(),
                    "forum": c.is_forum,
                    "last_activity": c.last_activity,
                    "messages": c.message_count,
                    "database": archived.files.get(&c.id).map(|p| p.display().to_string()),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    println!("{} chats", list.len());
    for c in &list {
        let migration = match &c.migration {
            Some(Migration::Became { id, .. }) => format!("  [upgraded to {id}]"),
            Some(Migration::CameFrom { id, .. }) => format!("  [was group {id}]"),
            None => String::new(),
        };
        println!(
            "  {:<12} {:<11} {}{}{migration}",
            c.id,
            c.kind.label(),
            c.title,
            if c.is_forum { "  [forum]" } else { "" }
        );
    }
    Ok(())
}

/// The chat `want` names: its id exactly, then its title exactly, then the
/// first title containing it, all case-folded.
///
/// The id comes first because it is the one a script can rely on — `chats
/// --json` prints it, and two chats may share a title or a prefix.
///
/// The empty string is rejected before this is reached, because `"".contains`
/// is true of every title: `export ""` would have exported whichever chat the
/// dialog list happened to return first, and then said it was exporting it —
/// which is not a usage error the user can see, it is the wrong chat done
/// confidently.
fn find_chat<'a>(list: &'a [ChatInfo], want: &str) -> Option<&'a ChatInfo> {
    let needle = want.trim().to_lowercase();
    if let Ok(id) = needle.parse::<i64>() {
        if let Some(c) = list.iter().find(|c| c.id == id) {
            return Some(c);
        }
    }
    list.iter()
        .find(|c| c.title.to_lowercase() == needle)
        .or_else(|| {
            list.iter()
                .find(|c| c.title.to_lowercase().contains(&needle))
        })
}

/// Where a command's narration goes. `export` and plain `sync` print to
/// stdout; `sync --json` keeps stdout for the one JSON document and narrates
/// on stderr, so a caller can parse the first without filtering the second.
#[derive(Clone, Copy)]
enum Out {
    Stdout,
    Stderr,
}

impl Out {
    fn line(self, s: &str) {
        match self {
            Out::Stdout => println!("{s}"),
            Out::Stderr => eprintln!("{s}"),
        }
    }

    /// The `\r` counter is for a person watching a console. Piped into
    /// something that reads it — a script, a model — it is thousands of
    /// overwritten lines in one, so it is left out.
    fn is_terminal(self) -> bool {
        use std::io::IsTerminal;
        match self {
            Out::Stdout => std::io::stdout().is_terminal(),
            Out::Stderr => std::io::stderr().is_terminal(),
        }
    }

    fn counter(self, s: &str) {
        match self {
            Out::Stdout => {
                print!("\r{s}    ");
                let _ = std::io::stdout().flush();
            }
            Out::Stderr => {
                eprint!("\r{s}    ");
                let _ = std::io::stderr().flush();
            }
        }
    }
}

/// Ctrl-C stops the export the way the window's Stop button does, rather
/// than killing the process: the JSON is streamed, so a run torn down
/// mid-write leaves a **zero-byte** file, not a partial one. Everything
/// fetched up to the key press is closed and kept.
fn stop_on_ctrl_c() -> Cancel {
    let cancel = Cancel::new();
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!("\nstopping — closing the export so nothing is left empty");
                cancel.cancel();
            }
            // A second Ctrl-C is the user saying they meant it. tokio's handler
            // suppresses the default terminate, so without this the key does
            // nothing at all the second time and the wait looks like a hang.
            //
            // But `exit` runs no destructors, and the JSON is *streamed*: an
            // `Output` dropped without `close()` leaves a file that is not
            // truncated but **zero bytes**, because the writes are still
            // buffered. That is exactly what the comment eleven lines above
            // promises this design prevents. There is no way to reach those
            // buffers from here — they belong to the export task — so the least
            // dishonest thing is to say what it costs before doing it.
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!(
                    "second Ctrl-C — exiting now. Any result.json still being \
                     written will be left EMPTY, not partial; the first Ctrl-C \
                     is what closes them cleanly."
                );
                std::process::exit(130);
            }
        });
    }
    cancel
}

/// The window, if one is open on this data folder, is a second writer: in
/// Database mode it may be merging into the very `.sqlite` a sync here would.
/// See `instance`, and [`writes_beside_the_window`] for the commands besides
/// export and sync that ask.
fn refuse_beside_the_window() -> Result<()> {
    if crate::instance::window_open(&tgx_tg::config::data_dir()) {
        return Err(anyhow!(
            "the TelegramTools window is open on this data folder; close it first, \
             so the two do not both write into the same files"
        ));
    }
    Ok(())
}

/// The offline commands that write into what the window may have open:
/// `import` merges recovered messages into the chats' databases, `split`
/// merges into them and renames the file it read, and `doctor --fix` moves and
/// deletes. Their dry runs and reports write nothing and run anywhere.
///
/// **Only export and sync were guarded**, which left the commands that write
/// the same files without a network free to do it under a sync. Those two
/// still ask at their own point, after the checks that make a better first
/// error. `login`, `logout` and `chats` touch only the session file, and
/// `chats --json` beside the window is a reasonable thing to want.
fn writes_beside_the_window(args: &[String]) -> bool {
    let has = |flag: &str| args.iter().skip(1).any(|a| a == flag);
    match args.first().map(String::as_str) {
        Some("import" | "split") => !has("--dry-run"),
        Some("doctor") => has("--fix"),
        _ => false,
    }
}

async fn export(settings: &Settings, want: &str) -> Result<()> {
    // The window's "Nothing to write" guard, which the CLI did not have: a
    // settings file with both Classic formats off would read the whole chat and
    // produce nothing.
    if !settings.writes_anything() {
        return Err(anyhow!(
            "nothing to write: settings.json has HTML and JSON off and Database off. \
             Pass --db, or turn a format on."
        ));
    }
    refuse_beside_the_window()?;
    let session = connected(settings).await?;
    let list = dialogs::list_chats(&session.client)
        .await
        .map_err(|e| anyhow!("listing chats: {e}"))?;
    let chat = find_chat(&list, want).ok_or_else(|| anyhow!("no chat matching {want:?}"))?;
    // A database this chat already has, found by the id inside it, so a
    // renamed chat syncs into its old file instead of starting a new one.
    // None yet, or a folder export: the title names the file, as before.
    let into = if settings.export_db {
        tgx_tg::engine::archived_chats(std::path::Path::new(&settings.output_dir))
            .await
            .ok()
            .and_then(|a| a.files.get(&chat.id).cloned())
    } else {
        None
    };
    let cancel = stop_on_ctrl_c();
    // Two failures, two messages: a chat that has left the dialog list is a
    // fact about the account, a sweep that did not finish is a fact about the
    // connection, and telling the user the first when the second happened sends
    // them looking for a chat they still have.
    let peer = dialogs::peer_ref_for(&session.client, chat.id)
        .await
        .map_err(|e| anyhow!("looking up {}: {e}", chat.title))?
        .ok_or_else(|| anyhow!("{} is no longer in the dialog list", chat.title))?;
    let result = export_one(&session, settings, chat, peer, into, &cancel, Out::Stdout).await?;
    summarise(settings, &result, Out::Stdout);
    Ok(())
}

/// One chat, start to finish, narrated to `out`. What `export` and `sync`
/// share; the summary is the caller's, because `sync --json` wants the numbers
/// and not the sentences. `into` is the database to sync into, when the caller
/// already knows which file holds this chat.
///
/// **The peer is the caller's to find**, because `sync` finds them all in one
/// sweep of the dialog list. Looking each one up here paged the list again for
/// every database — sixty-three sweeps for a nightly sync of sixty-three
/// chats, which is how a run earns a flood wait before it has synced a byte.
async fn export_one(
    session: &Session,
    settings: &Settings,
    chat: &ChatInfo,
    peer: dialogs::ChatRef,
    into: Option<std::path::PathBuf>,
    cancel: &Cancel,
    out: Out,
) -> Result<ExportResult> {
    out.line(&format!("exporting {} ({})", chat.title, chat.kind.label()));

    let topics = if chat.is_forum && settings.split_topics {
        let t = dialogs::list_topics(&session.client, peer)
            .await
            .map_err(|e| anyhow!("listing topics: {e}"))?;
        out.line(&format!("  {} topics", t.len()));
        t
    } else {
        vec![dialogs::Topic::general()]
    };

    // **No folder when nothing is going into one.** A database-only run would
    // otherwise reserve an export directory, write nothing into it, and leave
    // an empty `Dev Team (3)` behind on every sync.
    let root = if settings.writes_folders() {
        let root =
            tgx_tg::engine::unique_dir(std::path::Path::new(&settings.output_dir), &chat.title)?;
        out.line(&format!("  into {}", root.display()));
        Some(root)
    } else {
        out.line("  into the database only");
        None
    };

    let mut exporter = ChatExporter::new(&session.client, settings, session.session());
    if let Some(path) = into {
        out.line(&format!("  database {}", path.display()));
        exporter.sync_into(path);
    }
    let counting = out.is_terminal();
    let mut last_line = String::new();
    let mut on_progress = |p: Progress| match p {
        Progress::Total { total, .. } => out.line(&format!("  telegram counts {total} messages")),
        Progress::Messages { done, total, .. } => {
            if !counting {
                return;
            }
            let line = if total > 0 {
                format!("  {done} of {total}")
            } else {
                format!("  {done}")
            };
            if line != last_line {
                out.counter(&line);
                last_line = line;
            }
        }
        Progress::FloodWait { seconds } => {
            out.line(&format!("\n  rate limited, waiting {seconds}s"));
        }
        Progress::Topic { title, messages } => out.line(&format!("\n  {title}: {messages}")),
        Progress::Log(msg) => {
            out.line(&format!("\n  {msg}"));
            log::info!("{msg}");
            // The progress line is redrawn with `\r`, so anything printed over
            // it has to leave `last_line` empty or the next identical count is
            // suppressed and the counter appears to stop.
            last_line.clear();
        }
        // Only reaches here under `RUST_LOG=debug`, which is a request for it.
        Progress::Detail(msg) => {
            out.line(&msg);
            log::debug!("{msg}");
            last_line.clear();
        }
    };

    Ok(exporter
        .run(
            chat,
            peer,
            &topics,
            root.as_deref(),
            &mut on_progress,
            cancel,
        )
        .await?)
}

/// Did this chat's run end because the user stopped it?
fn was_cancelled(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<tgx_tg::ExportError>(),
        Some(tgx_tg::ExportError::Cancelled)
    )
}

/// The sentences under a finished chat.
fn summarise(settings: &Settings, result: &ExportResult, out: Out) {
    out.line("");
    out.line(&format!(
        "done: {} messages across {} topics ({} empty)",
        result.messages, result.topics, result.empty_topics
    ));
    if settings.export_db {
        out.line(&format!(
            "database: +{} new, {} changed, {} deleted on Telegram (kept), \
             {} files stored — {} archived",
            result.db_new,
            result.db_changed,
            result.db_deleted,
            result.db_blobs,
            result.db_archived
        ));
    }
    if result.media_downloaded > 0 || result.media_missing > 0 {
        // The count that matches missing_media.txt, for the same reason the
        // window's warning uses it: a failed job costs more than one file.
        out.line(&format!(
            "media: {} saved ({:.1} MB), {} could not be fetched",
            result.media_downloaded,
            result.bytes_downloaded as f64 / 1_048_576.0,
            result.media_missing
        ));
    }
    // A short export must not read like a complete one.
    if !result.complete() {
        out.line(&format!(
            "INCOMPLETE: Telegram counted {}; {} came through",
            result.expected, result.messages
        ));
    }
    if result.members > 0 {
        out.line(&format!(
            "members: {}{}",
            result.members,
            if result.members_complete {
                ""
            } else {
                " (INCOMPLETE — Telegram stopped serving the list)"
            }
        ));
    }
    if result.enrich_deferred > 0 {
        out.line(&format!(
            "{} optional requests were lost to rate limits or Telegram server errors \
             (the data was there)",
            result.enrich_deferred
        ));
    }
    if !result.degraded.is_empty() {
        out.line(&format!(
            "unmapped types written as text: {}",
            result.degraded.join(", ")
        ));
    }
}

/// `sync`: bring every database already in `settings.output_dir` up to date,
/// one connection for the lot.
///
/// **Which chats is decided by the folder, not by a list of titles.** The ids
/// come from inside the files — [`tgx_tg::engine::archived_chats`], which is
/// how the window finds them too — so a chat renamed on Telegram
/// since the last run is still found, and a caller that knows nothing about
/// the account can still say "update what is there".
///
/// One chat failing does not stop the rest: with a file each, the next has
/// every chance of working. The exit code says whether all of them made it.
async fn sync(settings: &Settings, only: &[String], json: bool) -> Result<()> {
    use serde_json::json;
    let out = if json { Out::Stderr } else { Out::Stdout };
    let dir = std::path::PathBuf::from(&settings.output_dir);
    let archived = tgx_tg::engine::archived_chats(&dir)
        .await
        .map_err(|e| anyhow!("could not read {}: {e}", dir.display()))?;

    // `--only` narrows by id or title, matched against what the files record —
    // the same rule as `export`, but over the databases, not the dialog list.
    let mut wanted: Vec<&(i64, String)> = archived.chats.iter().collect();
    if !only.is_empty() {
        wanted.retain(|(id, title)| {
            only.iter().any(|w| {
                let w = w.trim().to_lowercase();
                w == id.to_string() || title.to_lowercase().contains(&w)
            })
        });
        if wanted.is_empty() {
            return Err(anyhow!(
                "no database in {} matches {}",
                dir.display(),
                only.join(", ")
            ));
        }
    }
    if wanted.is_empty() {
        return Err(anyhow!(
            "no databases in {} to sync — make the first with \
             `TelegramTools export --db <title>`",
            dir.display()
        ));
    }

    refuse_beside_the_window()?;
    let session = connected(settings).await?;
    let list = dialogs::list_chats(&session.client)
        .await
        .map_err(|e| anyhow!("listing chats: {e}"))?;
    let cancel = stop_on_ctrl_c();
    // **One sweep for every database**, the window's rule for its queue — see
    // `export_one`. A sweep that did not finish fails the run here rather
    // than every chat in turn: the connection is what failed, not the chats.
    let ids: Vec<i64> = wanted
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| list.iter().any(|c| c.id == *id))
        .collect();
    let peers = dialogs::peer_refs_for(&session.client, &ids, &cancel)
        .await
        .map_err(|e| anyhow!("looking up the chats: {e}"))?;

    let mut rows = Vec::new();
    let mut whole = 0usize;
    for (n, (id, title)) in wanted.iter().enumerate() {
        out.line(&format!("\n[{}/{}] {title} (id {id})", n + 1, wanted.len()));
        let database = archived.files.get(id).map(|p| p.display().to_string());
        // By id only: a title here is what the file recorded, and a substring
        // match could land on a different chat that happens to contain it.
        let Some(chat) = list.iter().find(|c| c.id == *id) else {
            out.line("  not in this account's dialog list — left, removed or migrated; skipped");
            rows.push(json!({
                "chat_id": id, "title": title, "database": database,
                "status": "not_in_dialogs",
            }));
            continue;
        };
        if cancel.is_cancelled() {
            rows.push(json!({
                "chat_id": id, "title": title, "database": database,
                "status": "cancelled",
            }));
            continue;
        }
        // Into the file it was found in, never one worked out from the title.
        let into = archived.files.get(id).cloned();
        let ran = match peers.get(id).copied() {
            Some(peer) => export_one(&session, settings, chat, peer, into, &cancel, out).await,
            None => Err(anyhow!("{} is no longer in the dialog list", chat.title)),
        };
        match ran {
            Ok(r) => {
                summarise(settings, &r, out);
                let status = if r.complete() { "synced" } else { "incomplete" };
                if r.complete() {
                    whole += 1;
                }
                rows.push(json!({
                    "chat_id": id,
                    "title": chat.title,
                    "database": r.database.as_ref().map(|p| p.display().to_string()).or(database),
                    "status": status,
                    "new": r.db_new,
                    "changed": r.db_changed,
                    "deleted": r.db_deleted,
                    "files_stored": r.db_blobs,
                    "media_missing": r.media_missing,
                    "messages_stored": r.db_archived,
                    "messages_read": r.messages,
                }));
            }
            // **Ctrl-C is not a failure.** The chat it interrupted reads
            // "cancelled" like the ones after it, so a wrapper that retries
            // what failed does not retry the user's own stop. Everything
            // fetched up to the key press is closed and kept.
            Err(e) if was_cancelled(&e) => {
                out.line("  stopped");
                rows.push(json!({
                    "chat_id": id, "title": title, "database": database,
                    "status": "cancelled",
                }));
            }
            Err(e) => {
                out.line(&format!("  FAILED: {e:#}"));
                rows.push(json!({
                    "chat_id": id, "title": title, "database": database,
                    "status": "failed", "error": format!("{e:#}"),
                }));
            }
        }
    }

    let skipped: Vec<_> = archived
        .unreadable
        .iter()
        .map(|(p, why)| json!({ "path": p.display().to_string(), "why": why }))
        .collect();
    for (p, why) in &archived.unreadable {
        out.line(&format!("skipped {}: {why}", p.display()));
    }
    // A chat the account has left is reported but does not fail the run: it
    // will be gone on every later sync too, and an exit code that is always 1
    // teaches the caller to ignore it.
    let gone = rows
        .iter()
        .filter(|r| r["status"] == "not_in_dialogs")
        .count();
    let short = rows.len() - whole - gone;
    out.line(&format!(
        "\n{whole} of {} database(s) synced whole under {}",
        rows.len(),
        dir.display()
    ));
    if json {
        let doc = json!({
            "folder": dir.display().to_string(),
            "full": settings.reread_all,
            "synced": whole,
            "not_synced": short,
            "chats": rows,
            "unreadable": skipped,
        });
        println!("{}", serde_json::to_string_pretty(&doc)?);
    }
    if short > 0 {
        return Err(anyhow!(
            "{short} of {} database(s) did not sync whole — see the status of each",
            rows.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tgx_tg::client::ChatKind;

    fn chat(id: i64, title: &str) -> ChatInfo {
        ChatInfo {
            id,
            title: title.into(),
            kind: ChatKind::Supergroup,
            last_activity: 0,
            is_forum: false,
            public: false,
            message_count: None,
            migration: None,
        }
    }

    #[test]
    fn an_id_beats_a_title_that_contains_it() {
        // A title with the digits in it must not win over the chat whose id
        // they are: the id is what `chats --json` hands a script.
        let list = [chat(1, "Room 1234"), chat(1234, "Ops")];
        assert_eq!(find_chat(&list, "1234").map(|c| c.id), Some(1234));
    }

    #[test]
    fn an_exact_title_beats_a_longer_one_and_case_does_not_matter() {
        let list = [chat(1, "Dev Team archive"), chat(2, "Dev Team")];
        assert_eq!(find_chat(&list, "dev team").map(|c| c.id), Some(2));
        assert_eq!(find_chat(&list, "ARCHIVE").map(|c| c.id), Some(1));
        assert!(find_chat(&list, "nobody").is_none());
    }

    #[test]
    fn the_usage_names_sync_and_the_json_flags() {
        assert!(USAGE.contains("TelegramTools sync"));
        assert!(USAGE.contains("chats [--json]"));
    }

    #[test]
    fn every_one_line_usage_names_a_command_that_exists() {
        // Five of these once read `TelegramToolssplit` and the like, telling
        // people to run a program that does not exist.
        for usage in [
            EXPORT_USAGE,
            COMPARE_USAGE,
            SPLIT_USAGE,
            SEARCH_USAGE,
            UNPACK_USAGE,
            IMPORT_USAGE,
        ] {
            let command = usage
                .strip_prefix("TelegramTools ")
                .and_then(|rest| rest.split_whitespace().next())
                .unwrap_or_else(|| panic!("{usage:?} does not start with the program"));
            assert!(
                USAGE.contains(&format!("TelegramTools {command} ")),
                "{usage:?} names {command:?}, which the full usage does not"
            );
        }
    }

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn the_commands_that_write_the_windows_files_ask_whether_it_is_open() {
        for writes in [
            &["import", r"C:\old export"][..],
            &["split", "big.sqlite"],
            &["split", "big.sqlite", "--into", r"C:\out"],
            &["doctor", "--fix"],
            &["doctor", r"C:\Exports", "--fix"],
        ] {
            assert!(writes_beside_the_window(&argv(writes)), "{writes:?}");
        }
        // Dry runs and reports write nothing; the session-only commands are
        // left alone on purpose (`chats --json` beside the window is wanted).
        // Export and sync ask at their own point, after their own checks.
        for reads in [
            &["import", r"C:\old export", "--dry-run"][..],
            &["split", "big.sqlite", "--dry-run"],
            &["doctor"],
            &["doctor", "--dupes"],
            &["compare", r"C:\old export"],
            &["search", "word"],
            &["unpack", "a.sqlite", "out"],
            &["login"],
            &["logout"],
            &["chats", "--json"],
            &[],
        ] {
            assert!(!writes_beside_the_window(&argv(reads)), "{reads:?}");
        }
    }

    #[test]
    fn a_stopped_chat_is_cancelled_and_a_failed_one_is_not() {
        let stopped: anyhow::Error = tgx_tg::ExportError::Cancelled.into();
        assert!(was_cancelled(&stopped));
        assert!(!was_cancelled(&anyhow!("listing topics: FLOOD_WAIT")));
    }
}
