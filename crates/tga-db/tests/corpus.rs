//! The database reader against a real `telegram.sqlite`.
//!
//! The unit tests build a database this crate wrote itself, which proves the
//! reader agrees with `tga-read` but not that it agrees with **the exporter**.
//! Only a file TelegramExporter produced does that, and the two disagree in
//! ways a hand-built fixture cannot show — the shipped schema already differs
//! from `DATABASE.md` in how `blobs` is keyed.
//!
//! **The skip is deliberate and it is loud**, for the reason in
//! `tga-read/tests/corpus.rs`. `TGA_REQUIRE_CORPUS=1` turns a missing database
//! into a failure.

use std::path::PathBuf;

/// `TGA_DATABASE` overrides. Lives on a removable drive, like every corpus
/// here.
///
/// **This was `L:\9 telegram export\telegram.sqlite`**, one file holding UA
/// KOLAB and Telegram's service chat. `tgx split` took it apart into one
/// database per chat, `<title>\<title>.sqlite`, and the flat file is gone. UA
/// KOLAB's own is the same chat the old file's largest was. It was renamed on
/// Telegram when it was archived, which started a second file under the new
/// name; `tgx merge` folded the first into it, so the chat's one file is now
/// the archived name's.
const DEFAULT: &str = r"L:\9 telegram export\UA KOLAB (ARHIVIRANO)";

/// The folder the exporter syncs every chat's database into.
const OUTPUT: &str = r"L:\9 telegram export";

fn corpus() -> Option<PathBuf> {
    let root = std::env::var("TGA_DATABASE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT));
    if let Some(db) = tga_db::find(&root) {
        return Some(db);
    }
    let required = std::env::var("TGA_REQUIRE_CORPUS").as_deref() == Ok("1");
    assert!(
        !required,
        "TGA_REQUIRE_CORPUS=1 but no database at {}",
        root.display()
    );
    eprintln!(
        "SKIP: no database at {} — set TGA_DATABASE to point at one",
        root.display()
    );
    None
}

#[test]
fn the_exporters_own_file_reads() {
    let Some(db) = corpus() else { return };
    let chats = tga_db::chats(&db).expect("chats");
    assert!(!chats.is_empty(), "no chats in the database");
    eprintln!(
        "database: {} chats — {}",
        chats.len(),
        chats
            .iter()
            .map(|c| format!("{} ({} messages)", c.title, c.messages))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let export = tga_db::load(&db, None, None).expect("load");
    let biggest = &chats[0];

    // The reader must account for every message the table holds, not most of
    // them: a payload that fails to parse is skipped silently, and this is
    // what would catch a schema change that made that the common case.
    assert_eq!(
        export.msgs.len(),
        biggest.messages,
        "read {} of the {} messages SQL counts",
        export.msgs.len(),
        biggest.messages
    );
    assert_eq!(
        export.name, biggest.title,
        "the export is named for the chat"
    );
    assert!(!export.topics.is_empty(), "no topics");
    assert_eq!(
        export.topics.iter().map(|t| t.messages).sum::<usize>(),
        export.msgs.len(),
        "the per-topic counts must add up to the whole"
    );

    // Every topic of one chat carries that chat's id, which is what makes the
    // "one database per chat" shape work at all.
    for topic in &export.topics {
        assert_eq!(topic.chat_id, Some(biggest.id), "{}", topic.name);
    }
}

#[test]
fn the_figures_can_be_computed_from_it() {
    let Some(db) = corpus() else { return };
    let export = tga_db::load(&db, None, None).expect("load");

    // The point of the whole crate: an `Export` from a database goes through
    // the metrics untouched, because it is the same `Export`.
    let (stats, _people) = tga_metrics::analyse(&export);
    assert_eq!(stats.export.messages, export.msgs.len());
    assert_eq!(stats.export.topics, export.topics.len());
    assert!(stats.people.speakers > 0, "nobody spoke");
    eprintln!(
        "figures: {} messages, {} speakers, {} topics",
        stats.export.messages, stats.people.speakers, stats.export.topics
    );
}

#[test]
fn every_chat_in_the_file_can_be_read_on_its_own() {
    let Some(db) = corpus() else { return };
    for chat in tga_db::chats(&db).expect("chats") {
        let by_id = tga_db::load(&db, Some(&chat.id.to_string()), None)
            .unwrap_or_else(|e| panic!("{} ({}): {e}", chat.title, chat.id));
        assert_eq!(by_id.msgs.len(), chat.messages, "{}", chat.title);
        // Reading one chat must not drag in another's messages; that is the
        // failure a shared `messages` table invites.
        for topic in &by_id.topics {
            assert_eq!(topic.chat_id, Some(chat.id), "{}", chat.title);
        }
    }
}

/// Every database the exporter's output folder holds opens, and each is one
/// chat that reads whole.
///
/// This is what a refresh walks, so a file that fails here is a report that
/// silently stops being redone. Forty chats, 36 GB on disk — nearly all of it
/// media blobs, which nothing here reads.
#[test]
fn every_database_in_the_output_folder_reads() {
    let root = std::env::var("TGA_DATABASE_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(OUTPUT));
    let all = tga_db::find_all(&root);
    if all.len() < 2 {
        let required = std::env::var("TGA_REQUIRE_CORPUS").as_deref() == Ok("1");
        assert!(
            !required,
            "TGA_REQUIRE_CORPUS=1 but no per-chat databases under {}",
            root.display()
        );
        eprintln!("SKIP: no per-chat databases under {}", root.display());
        return;
    }
    // A folder of several is not one export, and `find` must not pretend it is.
    assert_eq!(
        tga_db::find(&root),
        None,
        "find picked one of {}",
        all.len()
    );

    let mut total = 0;
    for db in &all {
        let chats = tga_db::chats(db).unwrap_or_else(|e| panic!("{}: {e:#}", db.display()));
        assert_eq!(
            chats.len(),
            1,
            "{} holds {} chats",
            db.display(),
            chats.len()
        );
        let export =
            tga_db::load(db, None, None).unwrap_or_else(|e| panic!("{}: {e:#}", db.display()));
        assert_eq!(export.msgs.len(), chats[0].messages, "{}", chats[0].title);
        total += export.msgs.len();
    }
    eprintln!("output folder: {} databases, {total} messages", all.len());
}

#[test]
fn what_the_folder_format_could_not_record_is_readable() {
    let Some(db) = corpus() else { return };
    let chats = tga_db::chats(&db).expect("chats");
    for chat in &chats {
        // Zero today on this corpus. The assertion is that the queries run and
        // the columns are where this build thinks they are — a schema bump
        // that moved them would fail here rather than silently report none.
        let deleted = tga_db::deleted(&db, chat.id).expect("deleted");
        let (edited, versions) = tga_db::edits(&db, chat.id).expect("edits");
        assert!(versions >= edited, "more edited messages than versions");
        eprintln!(
            "{}: {deleted} deleted and kept, {edited} edited over {versions} versions",
            chat.title
        );
    }
}
