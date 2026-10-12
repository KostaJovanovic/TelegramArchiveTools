//! The reader, against a real export rather than a fixture.
//!
//! Unit tests prove the coercions; they cannot prove that this reads the same
//! export the Python original reads. Only a real folder does that, and the
//! numbers below are the Python analyser's own, taken from
//! `reference/ua-kolab.stats.json`'s `export` branch.
//!
//! **The skip is deliberate and it is loud.** libtest discards stdout for a
//! passing test, so a corpus test that quietly does nothing is a corpus test
//! nobody knows stopped running — that is how three green parity legs in
//! telegram_rust missed four missing features. Set `TGA_REQUIRE_CORPUS=1` and
//! a missing export becomes a failure; the machine that owns the drive should
//! always have it set.

use std::path::PathBuf;

/// Same default as the Python side's `save.bat REFEXPORT`. Lives on an
/// external drive, so it is absent more often than not.
const DEFAULT_EXPORT: &str = r"N:\telegram export\UA KOLAB TELEGRAM";

fn corpus() -> Option<PathBuf> {
    let path = std::env::var("TGA_EXPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_EXPORT));
    if path.is_dir() {
        return Some(path);
    }
    let required = std::env::var("TGA_REQUIRE_CORPUS").as_deref() == Ok("1");
    assert!(
        !required,
        "TGA_REQUIRE_CORPUS=1 but the export is not readable at {}",
        path.display()
    );
    eprintln!(
        "SKIP: no export at {} — set TGA_EXPORT to point at one",
        path.display()
    );
    None
}

#[test]
fn the_reference_export_reads_the_way_python_reads_it() {
    let Some(root) = corpus() else { return };
    let export = tga_read::load(&root, None).expect("load");

    // Every one of these is from the Python analyser's own --stats dump.
    assert_eq!(export.name, "UA KOLAB TELEGRAM");
    assert_eq!(export.topics.len(), 4, "topics");
    assert_eq!(export.msgs.len(), 6_643, "messages");
    assert_eq!(
        export.msgs.iter().filter(|m| m.service).count(),
        63,
        "service messages"
    );
    // This export carries no participants.json, and an absent roster is not a
    // roster known to be short.
    assert_eq!(export.members_count, None);
    assert_eq!(export.roster_complete, None);
}

#[test]
fn topic_order_matches_the_index_every_figure_is_keyed_on() {
    let Some(root) = corpus() else { return };
    let export = tga_read::load(&root, None).expect("load");

    // Python sorts WindowsPath case-insensitively, which is why "ćaskanje"
    // sorts last: U+0107 is above 'f'. Get this wrong and every per-topic
    // figure is attributed to the wrong topic while the totals still agree.
    let names: Vec<&str> = export.topics.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        ["bitno pročitaj", "editorijal", "foto video", "ćaskanje"]
    );
}

#[test]
fn the_forum_root_reply_is_dropped_before_any_metric_sees_it() {
    let Some(root) = corpus() else { return };
    let export = tga_read::load(&root, None).expect("load");

    // The Python figure is 3,518 replies after the drop. Counted literally,
    // 2,702 more would be here — every top-level message answering the one
    // that opened its topic.
    let replies = export.said().filter(|m| m.reply_to.is_some()).count();
    assert_eq!(replies, 3_518, "replies after the topic-root drop");
}
