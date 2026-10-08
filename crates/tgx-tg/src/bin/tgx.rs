//! A command-line exporter.
//!
//! The GUI is the product, but this exists for a specific reason: it is the
//! only way to exercise the engine against a real account without a window, and
//! `convert.rs` is the one module in the pipeline no parity harness can verify.
//! Everything downstream of the wire is pinned byte for byte; this is how the
//! wire itself gets tested.
//!
//! ```text
//! tgx login
//! tgx chats
//! tgx export "UA KOLAB TELEGRAM"
//! ```

use anyhow::{anyhow, Result};
use std::io::Write;
use tgx_tg::cancel::Cancel;
use tgx_tg::client::Session;
use tgx_tg::config::Settings;
use tgx_tg::dialogs;
use tgx_tg::engine::{ChatExporter, Progress};

#[tokio::main]
async fn main() -> Result<()> {
    // The same file the window writes, for the same reason: this binary exists
    // to exercise the wire, and the wire is the one part of the pipeline with
    // no parity leg over it. `RUST_LOG=debug` here is the closest thing to a
    // trace of what Telegram actually sent.
    if let Err(e) = tgx_tg::logging::init() {
        eprintln!("could not open the log: {e}");
    }

    // The same check `actions::report_data_dir_protection` makes for the window.
    // `tgx login` writes a bearer credential into this folder, and this is the
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

    let args: Vec<String> = std::env::args().skip(1).collect();
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

    match cmd {
        "login" => login(&settings).await,
        "chats" => chats(&settings).await,
        "export" => {
            // The flags are read off the whole tail rather than positionally,
            // so `tgx export --db "Dev Team"` and `tgx export "Dev Team" --db`
            // both work. A title starting with `--` is not a thing this tool
            // has to support.
            let flags: Vec<&str> = args
                .iter()
                .skip(1)
                .map(String::as_str)
                .filter(|a| a.starts_with("--"))
                .collect();
            for flag in &flags {
                match *flag {
                    // Database *mode*: it replaces the folder export, it
                    // does not join it. `export_html` and `export_json` are
                    // left as they are and simply ignored, so the flag does not
                    // quietly rewrite the settings file for the next run.
                    "--db" => settings.export_db = true,
                    "--full" => settings.reread_all = true,
                    other => return Err(anyhow!("unknown flag {other:?}\n\n{USAGE}")),
                }
            }
            let want = args
                .iter()
                .skip(1)
                .map(String::as_str)
                .map(str::trim)
                .find(|w| !w.is_empty() && !w.starts_with("--"))
                .ok_or_else(|| anyhow!("usage: tgx export [--db] [--full] <chat title>"))?;
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
                return Err(anyhow!(
                    "usage: tgx compare <export folder>... [--db <folder>] [--csv <file>]"
                ));
            }
            compare(&roots, &db, csv.as_deref()).await
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
                return Err(anyhow!(
                    "usage: tgx import <export folder>... [--db <folder>] [--dry-run]"
                ));
            }
            import(&roots, &db, dry_run).await
        }
        _ => {
            println!("{USAGE}");
            Ok(())
        }
    }
}

/// `tgx import`: copy what only old exports hold into the databases.
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

/// `tgx compare`: what old exports hold that the databases do not.
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

const USAGE: &str = "\
tgx — Telegram Desktop-format exporter

  tgx login            sign in (once; the session is saved)
  tgx chats            list every chat this account can see
  tgx export <title>   export the chat whose title matches
  tgx compare <export folder>... [--db <folder>] [--csv <file>]
                       offline: list messages old Desktop-format exports hold
                       that the databases do not (ids and dates only); an
                       HTML-only export is matched to a database by chat name
  tgx import <export folder>... [--db <folder>] [--dry-run]
                       offline: copy those messages into the databases, marked
                       deleted and recovered (only into a database that has
                       finished a sync)

  export flags:
    --db      Database mode instead of folders: sync into the chat's own
              Exports/<chat>/<chat>.sqlite, media included
    --full    with --db, walk the whole history rather than the trailing
              window — needed for older deletions and for files an earlier
              run skipped

Credentials come from TelegramExporterData/settings.json, or from the
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
        return Err(anyhow!("not signed in — run `tgx login` first"));
    }
    Ok(session)
}

async fn chats(settings: &Settings) -> Result<()> {
    let session = connected(settings).await?;
    let mut list = dialogs::list_chats(&session.client)
        .await
        .map_err(|e| anyhow!("listing chats: {e}"))?;
    list.sort_by_key(|c| -c.last_activity);
    println!("{} chats", list.len());
    for c in &list {
        println!(
            "  {:<12} {:<11} {}{}",
            c.id,
            c.kind.label(),
            c.title,
            if c.is_forum { "  [forum]" } else { "" }
        );
    }
    Ok(())
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
    let session = connected(settings).await?;
    let list = dialogs::list_chats(&session.client)
        .await
        .map_err(|e| anyhow!("listing chats: {e}"))?;

    // Exact title first, substring second. The empty string is rejected in
    // `main` rather than here, because `"".contains` is true of every title:
    // `tgx export ""` would have exported whichever chat the dialog list
    // happened to return first, and then said it was exporting it — which is
    // not a usage error the user can see, it is the wrong chat done confidently.
    let needle = want.to_lowercase();
    let chat = list
        .iter()
        .find(|c| c.title.to_lowercase() == needle)
        .or_else(|| {
            list.iter()
                .find(|c| c.title.to_lowercase().contains(&needle))
        })
        .ok_or_else(|| anyhow!("no chat matching {want:?}"))?;

    println!("exporting {} ({})", chat.title, chat.kind.label());

    // Two failures, two messages: a chat that has left the dialog list is a
    // fact about the account, a sweep that did not finish is a fact about the
    // connection, and telling the user the first when the second happened sends
    // them looking for a chat they still have.
    let peer = dialogs::peer_ref_for(&session.client, chat.id)
        .await
        .map_err(|e| anyhow!("looking up {}: {e}", chat.title))?
        .ok_or_else(|| anyhow!("{} is no longer in the dialog list", chat.title))?;

    let topics = if chat.is_forum && settings.split_topics {
        let t = dialogs::list_topics(&session.client, peer)
            .await
            .map_err(|e| anyhow!("listing topics: {e}"))?;
        println!("  {} topics", t.len());
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
        println!("  into {}", root.display());
        Some(root)
    } else {
        println!("  into the database only");
        None
    };

    let mut exporter = ChatExporter::new(&session.client, settings, session.session());
    let mut last_line = String::new();
    let mut on_progress = |p: Progress| match p {
        Progress::Total { total, .. } => println!("  telegram counts {total} messages"),
        Progress::Messages { done, total, .. } => {
            let line = if total > 0 {
                format!("  {done} of {total}")
            } else {
                format!("  {done}")
            };
            if line != last_line {
                print!("\r{line}    ");
                let _ = std::io::stdout().flush();
                last_line = line;
            }
        }
        Progress::FloodWait { seconds } => {
            println!("\n  rate limited, waiting {seconds}s");
        }
        Progress::Topic { title, messages } => println!("\n  {title}: {messages}"),
        Progress::Log(msg) => {
            println!("\n  {msg}");
            log::info!("{msg}");
            // The progress line is redrawn with `\r`, so anything printed over
            // it has to leave `last_line` empty or the next identical count is
            // suppressed and the counter appears to stop.
            last_line.clear();
        }
        // Only reaches here under `RUST_LOG=debug`, which is a request for it.
        Progress::Detail(msg) => {
            println!("{msg}");
            log::debug!("{msg}");
            last_line.clear();
        }
    };

    // Ctrl-C stops the export the way the window's Stop button does, rather
    // than killing the process: the JSON is streamed, so a run torn down
    // mid-write leaves a **zero-byte** file, not a partial one. Everything
    // fetched up to the key press is closed and kept.
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

    let result = exporter
        .run(
            chat,
            peer,
            &topics,
            root.as_deref(),
            &mut on_progress,
            &cancel,
        )
        .await?;

    println!();
    println!(
        "done: {} messages across {} topics ({} empty)",
        result.messages, result.topics, result.empty_topics
    );
    if settings.export_db {
        println!(
            "database: +{} new, {} changed, {} deleted on Telegram (kept), \
             {} files stored — {} archived",
            result.db_new,
            result.db_changed,
            result.db_deleted,
            result.db_blobs,
            result.db_archived
        );
    }
    if result.media_downloaded > 0 || result.media_missing > 0 {
        // The count that matches missing_media.txt, for the same reason the
        // window's warning uses it: a failed job costs more than one file.
        println!(
            "media: {} saved ({:.1} MB), {} could not be fetched",
            result.media_downloaded,
            result.bytes_downloaded as f64 / 1_048_576.0,
            result.media_missing
        );
    }
    // A short export must not read like a complete one.
    if !result.complete() {
        println!(
            "INCOMPLETE: Telegram counted {}; {} came through",
            result.expected, result.messages
        );
    }
    if result.members > 0 {
        println!(
            "members: {}{}",
            result.members,
            if result.members_complete {
                ""
            } else {
                " (INCOMPLETE — Telegram stopped serving the list)"
            }
        );
    }
    if result.enrich_deferred > 0 {
        println!(
            "{} optional requests were lost to rate limits or Telegram server errors \
             (the data was there)",
            result.enrich_deferred
        );
    }
    if !result.degraded.is_empty() {
        println!(
            "unmapped types written as text: {}",
            result.degraded.join(", ")
        );
    }
    Ok(())
}
