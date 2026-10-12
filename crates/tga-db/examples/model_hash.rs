//! Hash the whole model a database reads into, field by field.
//!
//! `cargo run --release -p tga-db --example model_hash -- <db.sqlite>...`
//!
//! **Written to prove a change of SQLite library changed nothing.** `tga-db`
//! moved from `rusqlite` to the exporter's own `tgx-archive` reader when the two
//! repos merged, because a second SQLite in one binary fails at link time. The
//! report and the stats dump only see what the figures use; this sees every
//! field of every message, so a NULL read as `""` instead of absent, or an
//! integer that changed width, shows up here even where no figure moves.
//!
//! It hashes `{:#?}` rather than writing it out: the largest corpus chat is
//! 171,354 messages, and its dump would be a few hundred megabytes to compare a
//! line of. One hash per topic as well, so a mismatch says where to look.
//! `DefaultHasher::new()` is SipHash with fixed keys, so the number is stable
//! between two runs of one toolchain, which is all a before/after needs.

use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hasher};
use std::path::Path;

/// `fmt::Write` straight into a hasher, so the dump never exists as a string.
struct Feed(DefaultHasher);

impl std::fmt::Write for Feed {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}

fn hash(value: &impl std::fmt::Debug) -> u64 {
    let mut feed = Feed(DefaultHasher::new());
    let _ = write!(feed, "{value:#?}");
    feed.0.finish()
}

fn main() -> anyhow::Result<()> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(!paths.is_empty(), "usage: model_hash <db.sqlite>...");
    for path in paths {
        let export = tga_db::load(Path::new(&path), None, None)?;
        println!(
            "{:016x}  {} messages, {} topics, {} members  {path}",
            hash(&export),
            export.msgs.len(),
            export.topics.len(),
            export.roster.len()
        );
        for topic in &export.topics {
            let msgs: Vec<_> = export
                .msgs
                .iter()
                .filter(|m| m.topic == topic.index)
                .collect();
            println!(
                "  {:016x}  {:>7}  {}",
                hash(&(topic, &msgs)),
                msgs.len(),
                topic.name
            );
        }
        println!("  {:016x}  roster", hash(&export.roster));
    }
    Ok(())
}
