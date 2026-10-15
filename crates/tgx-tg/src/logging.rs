//! A log file beside the executable.
//!
//! **Nothing in this workspace ever installed a `log` implementation.** Every
//! `log::warn!` we wrote — including the one that says Telegram asked to
//! restart the authorisation — and every `info!` inside grammers went to a
//! logger that did not exist. That is invisible right up until the question is
//! *why did signing in take so long*, which cannot be answered from inside a
//! program that records nothing. The three lines that answer it are already
//! written, in `grammers-mtsender`:
//!
//! ```text
//! connecting...
//! generating new authorization key...
//! authorization key generated successfully
//! ```
//!
//! **Elapsed seconds, not wall clock, is the first column.** The reason to open
//! this file is almost always to find out what took the time, and subtracting
//! two timestamps by eye is a step between the reader and the answer. The wall
//! clock is on the header line, once, for anyone correlating with something
//! else.
//!
//! The file is the only channel that survives a double-click: a GUI-subsystem
//! binary has no stderr unless it was launched from a terminal, which is the
//! same reason `main.rs` writes `startup-error.log`.

use crate::config::{data_dir, ensure_data_dir};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

/// This run's log.
pub const LOG_NAME: &str = "tgx.log";

/// The run before it.
///
/// One generation, kept because the log is most wanted *after* the thing went
/// wrong — and by then the app has usually been restarted, which is exactly
/// the moment a truncate-on-start log destroys the evidence.
pub const PREVIOUS_LOG_NAME: &str = "tgx.prev.log";

pub fn log_file() -> PathBuf {
    data_dir().join(LOG_NAME)
}

struct FileLogger {
    level: log::LevelFilter,
    started: Instant,
    out: Mutex<File>,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Flushed per line rather than buffered. A `BufWriter` loses exactly
        // the tail, and the tail is the half that says what the program was
        // doing when it stopped.
        if let Ok(mut out) = self.out.lock() {
            let _ = writeln!(
                out,
                "{:>9.3}  {:<5}  {}  {}",
                self.started.elapsed().as_secs_f64(),
                record.level(),
                record.target(),
                record.args()
            );
            let _ = out.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut out) = self.out.lock() {
            let _ = out.flush();
        }
    }
}

/// Read the wanted level out of `RUST_LOG`.
///
/// **A plain level, not `env_logger`'s filter grammar.** Accepting
/// `tgx_tg=debug,grammers=info` and then ignoring the half we do not implement
/// would be worse than not accepting it: the user would believe a filter was
/// applied. Anything unrecognised falls back to the default and says so in the
/// header, rather than silently logging nothing.
fn wanted_level() -> (log::LevelFilter, Option<String>) {
    let Ok(raw) = std::env::var("RUST_LOG") else {
        return (log::LevelFilter::Info, None);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" => (log::LevelFilter::Off, None),
        "error" => (log::LevelFilter::Error, None),
        "warn" => (log::LevelFilter::Warn, None),
        "info" => (log::LevelFilter::Info, None),
        "debug" => (log::LevelFilter::Debug, None),
        "trace" => (log::LevelFilter::Trace, None),
        _ => (log::LevelFilter::Info, Some(raw)),
    }
}

/// The last line of the header; every line after it is a record.
const LEGEND: &str = "seconds since start, then level, target, message";

/// The target of the records `init` writes itself.
const OWN_TARGET: &str = module_path!();

/// Whether the log at `path` holds anything past what [`init`] writes into
/// every log: the header, and its own records ("logging to …", the lockdown
/// warning), which come from this module's target.
///
/// A log that cannot be read, or has no legend line — another build's, or a
/// torn one — counts as holding something: keeping a file nobody needed costs
/// less than losing one somebody did.
fn holds_a_record(path: &std::path::Path) -> bool {
    use std::io::BufRead;
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => return e.kind() != std::io::ErrorKind::NotFound,
    };
    let own = format!("  {OWN_TARGET}  ");
    let mut past_header = false;
    let mut any = false;
    for line in std::io::BufReader::new(file).lines() {
        let Ok(line) = line else {
            return true;
        };
        any = true;
        if !past_header {
            past_header = line == LEGEND;
        } else if !line.contains(&own) {
            return true;
        }
    }
    any && !past_header
}

/// Start logging, and answer where it went.
///
/// Failing to open the log must never stop the app: it is a diagnostic, and
/// refusing to start because the diagnostic could not be written would cost
/// someone an export over a file they did not ask for. The error is returned
/// so a caller that has somewhere to put it can, and ignored otherwise.
///
/// Calling this twice is a no-op — `log::set_boxed_logger` refuses the second
/// installation, and that refusal is not an error worth surfacing.
pub fn init() -> Result<PathBuf, String> {
    ensure_data_dir().map_err(|e| format!("creating the data directory: {e}"))?;
    let path = log_file();

    // Rotate one generation. The previous run is what someone is usually
    // looking for, so it is moved rather than overwritten; two runs is enough
    // to be useful and bounded enough that a portable folder does not grow
    // without limit.
    //
    // **Only a log with something in it.** Every command starts here, so a
    // run that recorded nothing past its own header used to push the failed
    // export's log out of `tgx.prev.log` — two quick commands to look around
    // after a failure and the evidence was gone. Such a log is overwritten
    // instead: it held nothing anyone could want.
    if holds_a_record(&path) {
        let _ = std::fs::rename(&path, data_dir().join(PREVIOUS_LOG_NAME));
    }

    let file = File::create(&path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    let (level, unparsed) = wanted_level();
    let logger = FileLogger {
        level,
        started: Instant::now(),
        out: Mutex::new(file),
    };

    // The header carries what the elapsed column cannot: when zero was, and
    // what is actually being recorded.
    {
        let mut out = logger.out.lock().unwrap();
        let _ = writeln!(
            out,
            "TelegramTools {} — started {} — level {level}",
            env!("CARGO_PKG_VERSION"),
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f %:z"),
        );
        if let Some(raw) = &unparsed {
            let _ = writeln!(
                out,
                "(RUST_LOG={raw:?} is not a plain level name; using {level})"
            );
        }
        let _ = writeln!(out, "{LEGEND}");
        let _ = out.flush();
    }

    log::set_max_level(level);
    log::set_boxed_logger(Box::new(logger)).map_err(|e| e.to_string())?;
    log::info!("logging to {}", path.display());

    // `ensure_data_dir()` above is what tries to lock the folder down, and its
    // failure path calls `log::warn!` — three lines before any logger exists.
    // So the one warning in this workspace that concerns a bearer credential
    // was, every single time, written to the no-op logger. Re-stating it here
    // is what makes this module's "the app says so in the log" true.
    if let Some(why) = crate::config::lockdown_error() {
        log::warn!(
            "{} is NOT restricted to your user: {why} — \
             it holds the session key, which is a bearer credential",
            data_dir().display()
        );
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_rust_log_asks_for_info() {
        // Not `debug`: grammers logs the body of every failed deserialisation
        // at that level, and a default nobody chose should not produce a
        // megabyte per export.
        if std::env::var_os("RUST_LOG").is_none() {
            assert_eq!(wanted_level().0, log::LevelFilter::Info);
        }
    }

    #[test]
    fn the_two_log_names_are_distinct() {
        // The rotation renames one onto the other, so equal names would delete
        // the run someone is trying to read.
        assert_ne!(LOG_NAME, PREVIOUS_LOG_NAME);
        assert!(log_file().ends_with(LOG_NAME));
    }

    #[test]
    fn a_log_of_nothing_but_its_header_does_not_push_out_the_last_one() {
        let dir = std::env::temp_dir().join(format!("tgx-logrotate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join(LOG_NAME);
        assert!(!holds_a_record(&log), "no log at all");

        let own = OWN_TARGET;
        let header = format!(
            "TelegramTools 0.0.0 — started 2026-10-15 10:00:00.000 +02:00 — level INFO\n\
             {LEGEND}\n\
             \x20   0.001  INFO   {own}  logging to x\\tgx.log\n\
             \x20   0.001  WARN   {own}  x is NOT restricted to your user\n"
        );
        std::fs::write(&log, &header).unwrap();
        assert!(!holds_a_record(&log), "only what init writes");

        std::fs::write(
            &log,
            format!("{header}    2.500  INFO   grammers_mtsender  connecting...\n"),
        )
        .unwrap();
        assert!(holds_a_record(&log), "a run that did something");

        // Something that is not one of ours is kept rather than judged.
        std::fs::write(&log, "a line with no header\n").unwrap();
        assert!(holds_a_record(&log));
        std::fs::write(&log, "").unwrap();
        assert!(!holds_a_record(&log));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
