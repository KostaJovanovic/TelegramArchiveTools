//! Enforces the dependency rules CLAUDE.md and the README both describe as
//! "enforced by the build" — which, until this file existed, they were not.
//! Since the merge into TelegramTools it holds the analyser's rules too.
//! `cargo test --all` reads every crate's `Cargo.toml` as plain TOML text and
//! fails loudly, naming the rule, if a forbidden dependency has crept in.
//!
//! This lives in `tgx-parity` because it is the harness crate: it already
//! depends on nothing that would make it awkward to also carry the checks
//! that keep the *other* crates honest.

use std::path::Path;

fn manifest(crate_dir: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(crate_dir)
        .join("Cargo.toml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Deliberately not a real TOML parser — pulling in one just for two
/// dependency checks would be a heavier fix than the problem it solves. Every
/// section whose header ends in `dependencies]` is one Cargo actually reads
/// as a dependency table (`[dependencies]`, `[dev-dependencies]`,
/// `[build-dependencies]`, and the `[target.'cfg(...)'.*-dependencies]` forms
/// this workspace uses for `winresource`), so tracking "am I inside one of
/// those" line by line is enough to catch every place a crate name can
/// appear as a key.
fn dependency_names(manifest_toml: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_deps_section = false;
    for raw_line in manifest_toml.lines() {
        let line = raw_line.trim();
        if let Some(header) = line.strip_prefix('[') {
            let header = header.trim_end_matches(']');
            in_deps_section = header.ends_with("dependencies");
            continue;
        }
        if !in_deps_section {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            let key = key.trim().trim_matches('"');
            if !key.is_empty() {
                names.push(key.to_string());
            }
        }
    }
    names
}

/// `tgx-html` writes Desktop's markup from serialised maps and MUST NOT know
/// about Telegram's wire types — that is what lets the parity harness replay
/// a recorded `result.json` through it with no connection at all. A
/// `grammers-*` dependency here would mean the writer had started depending
/// on wire shapes instead of the JSON schema, and the html leg would no
/// longer be proving what it claims to prove.
#[test]
fn tgx_html_does_not_depend_on_grammers() {
    let names = dependency_names(&manifest("tgx-html"));
    let offenders: Vec<&String> = names.iter().filter(|n| n.starts_with("grammers")).collect();
    assert!(
        offenders.is_empty(),
        "tgx-html/Cargo.toml depends on {offenders:?} — tgx-html must not depend on \
         grammers-tl-types (or any grammers-* crate). It writes Desktop's pages from \
         serialised maps with no knowledge of Telegram's wire types; that separation is \
         what lets the parity harness replay a recorded result.json through it with no \
         network connection. See CLAUDE.md's Architecture section."
    );
}

/// `tgx-archive` is the database output. It takes the message payload as a
/// serialised map — exactly what `result.json` carries, minus `_p` — and knows
/// nothing of Telegram's wire types, for the same reason `tgx-html` does not:
/// **the store must be replayable by the harness from recorded data.** The
/// archive leg merges a real Desktop export into a store twice and reads
/// Desktop's own bytes back out, and it can only do that offline if nothing in
/// the store needs a connection to exist.
#[test]
fn tgx_archive_does_not_depend_on_grammers() {
    let names = dependency_names(&manifest("tgx-archive"));
    let offenders: Vec<&String> = names.iter().filter(|n| n.starts_with("grammers")).collect();
    assert!(
        offenders.is_empty(),
        "tgx-archive/Cargo.toml depends on {offenders:?} — the store must not depend on \
         grammers-tl-types (or any grammers-* crate). It takes the same serialised map the \
         JSON and HTML writers take, so the harness can replay a recorded result.json \
         through it with no network connection; that is what the archive leg proves. \
         See CLAUDE.md's Architecture section."
    );
}

/// The other half of the rule above, and the one that is easier to break by
/// accident: `tgx-tg` owns the engine, the client and every typed error, so
/// reaching for it from the store is the obvious shortcut the moment the store
/// wants a `Settings` or a `Progress`. It would also make the store
/// unreachable from `tgx-parity`, which must never depend on `tgx-tg` — so the
/// archive leg would stop compiling, which is a late and confusing way to
/// learn the layer was crossed.
#[test]
fn tgx_archive_does_not_depend_on_tgx_tg() {
    let names = dependency_names(&manifest("tgx-archive"));
    assert!(
        !names.iter().any(|n| n == "tgx-tg"),
        "tgx-archive/Cargo.toml depends on tgx-tg — the store sits *below* the engine, not \
         beside it. Anything it needs from a run is passed in as a value. Depending on \
         tgx-tg would also put it out of reach of tgx-parity, which may not depend on \
         tgx-tg, and the archive leg would have nowhere to live. See CLAUDE.md's \
         Architecture section."
    );
}

/// `tgx-app` is the window. It reaches Telegram only through `tgx-tg`, which
/// owns the client, the engine and every typed error the UI is allowed to see.
/// A direct `grammers-*` dependency lets a wire type reach the widgets, and the
/// seam that keeps GPUI on the main thread and tokio on its own stops being the
/// only way across.
///
/// This assertion was missing while `tgx-app/Cargo.toml` carried an unused
/// `grammers-client`, so the manifest and CLAUDE.md disagreed and the test that
/// exists to catch exactly that did not look.
#[test]
fn tgx_app_does_not_depend_on_grammers() {
    let names = dependency_names(&manifest("tgx-app"));
    let offenders: Vec<&String> = names.iter().filter(|n| n.starts_with("grammers")).collect();
    assert!(
        offenders.is_empty(),
        "tgx-app/Cargo.toml depends on {offenders:?} — the window must reach Telegram only \
         through tgx-tg, which owns the client and the typed errors the UI may see. A direct \
         grammers-* dependency lets a wire type reach the widgets. See CLAUDE.md's \
         Architecture section."
    );
}

/// CLAUDE.md says "tgx-app: the window. Depends only on tgx-ui + tgx-tg", and
/// until this test existed that was a sentence rather than a rule.
///
/// It is worth a check of its own because the pressure to break it is real and
/// looks harmless every time: the window needs one constant or one type from a
/// lower crate — the database file name, say — and adding the path dependency
/// is a one-line change that compiles. Then the window knows about the store's
/// vocabulary, and the seam that keeps every Telegram and storage concern
/// behind `tgx-tg` has a second hole in it. Whatever the window needs, `tgx-tg`
/// re-exports or does on its behalf.
#[test]
fn tgx_app_depends_only_on_tgx_ui_and_tgx_tg() {
    let names = dependency_names(&manifest("tgx-app"));
    let ours: Vec<&String> = names.iter().filter(|n| n.starts_with("tgx-")).collect();
    let strays: Vec<&&String> = ours
        .iter()
        .filter(|n| n.as_str() != "tgx-ui" && n.as_str() != "tgx-tg")
        .collect();
    assert!(
        strays.is_empty(),
        "tgx-app/Cargo.toml depends on {strays:?} — the window may name only tgx-ui and \
         tgx-tg. Anything it needs from a lower crate comes through tgx-tg, which owns the \
         client, the engine, the store and every typed error the UI is allowed to see. \
         See CLAUDE.md's Architecture section."
    );
}

// -- the analyser's rules ------------------------------------------------------
//
// These were prose in TelegramAnalyser's CLAUDE.md until the two repositories
// merged into TelegramTools. In one workspace a forbidden dependency is a single
// path line away, so they are checked like the exporter's.

/// Every crate directory in the workspace whose name starts with `prefix`.
fn crates(prefix: &str) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut names: Vec<String> = std::fs::read_dir(&root)
        .expect("reading crates/")
        .flatten()
        .filter(|e| e.path().join("Cargo.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix))
        .collect();
    names.sort();
    names
}

/// **The analyser works on an export with no network, ever.** That is what
/// lets it run on a machine with no account signed in, and what keeps a report
/// something that can be regenerated from files alone. A `grammers-*` or
/// `tgx-tg` dependency anywhere in `tga-*` would end both.
#[test]
fn no_analyser_crate_reaches_telegram() {
    let analyser = crates("tga-");
    assert!(analyser.len() >= 8, "found only {analyser:?}");
    for name in analyser {
        let offenders: Vec<String> = dependency_names(&manifest(&name))
            .into_iter()
            .filter(|n| n.starts_with("grammers") || n == "tgx-tg" || n == "tgx-app")
            .collect();
        assert!(
            offenders.is_empty(),
            "{name}/Cargo.toml depends on {offenders:?} — the analyser reads an export that \
             is already on disk and never talks to Telegram. See CLAUDE.md's Architecture \
             section."
        );
    }
}

/// **The exporter's oracle must not move when the analyser does.** The four
/// replay legs prove the exporter's writers byte for byte; if one of them
/// depended on an analyser crate, a change made for a report could shift a
/// byte of `result.json` and the leg would be measuring both at once.
#[test]
fn no_exporter_crate_depends_on_the_analyser() {
    for name in crates("tgx-") {
        let offenders: Vec<String> = dependency_names(&manifest(&name))
            .into_iter()
            .filter(|n| n.starts_with("tga-"))
            .collect();
        assert!(
            offenders.is_empty(),
            "{name}/Cargo.toml depends on {offenders:?} — the exporter's crates sit below \
             the analyser and must not reach up into it. See CLAUDE.md's Architecture \
             section."
        );
    }
}

/// `tga-report` renders from a `tga_stats::Stats` and nothing else, so a
/// recorded dump replays through it with no export on disk — which is what the
/// golden test and `--from-stats` both rest on.
#[test]
fn tga_report_depends_on_neither_the_reader_nor_the_metrics() {
    let names = dependency_names(&manifest("tga-report"));
    for banned in ["tga-read", "tga-metrics", "tga-db", "tga-docs"] {
        assert!(
            !names.iter().any(|n| n == banned),
            "tga-report/Cargo.toml depends on {banned} — the writer renders from a Stats \
             alone, so a recorded dump can replay through it with no export on disk."
        );
    }
}

/// `tga-notes` carries the `Event` type `tga-report` renders, so the report's
/// rule reaches it at one remove: a dependency on the reader here would reach
/// the writer transitively. The `Export -> digest::Row` mapping lives in the
/// caller for exactly this reason.
#[test]
fn tga_notes_depends_on_neither_the_reader_nor_the_documents() {
    let names = dependency_names(&manifest("tga-notes"));
    for banned in ["tga-read", "tga-docs", "tga-db", "tga-metrics"] {
        assert!(
            !names.iter().any(|n| n == banned),
            "tga-notes/Cargo.toml depends on {banned} — tga-report depends on tga-notes, so \
             this would reach the report writer. The caller maps an Export to digest rows."
        );
    }
}

/// `tga-docs` links a zip reader, an XML parser and a PDF parser that panics
/// on bad input, and `tga-db` links the store. Neither belongs anywhere near
/// the report, so the one crate that drives a run is the only one that may
/// reach them.
#[test]
fn only_the_analysers_front_end_reaches_documents_and_databases() {
    for name in crates("") {
        if name == "tga-app" || name == "tga-db" {
            continue;
        }
        let names = dependency_names(&manifest(&name));
        for heavy in ["tga-docs", "tga-db"] {
            assert!(
                !names.iter().any(|n| n == heavy),
                "{name}/Cargo.toml depends on {heavy} — only tga-app may. See CLAUDE.md's \
                 Architecture section."
            );
        }
    }
}

/// `tgx-parity` is the oracle: it replays *recorded* data — a real Desktop
/// export, or the corpus cut from one — through our own writers and diffs
/// the result. If it depended on `tgx-tg` it could reach for a live client
/// instead of replaying fixtures, and the harness would stop being something
/// that runs offline, deterministically, on a machine with no signed-in
/// account.
#[test]
fn tgx_parity_does_not_depend_on_tgx_tg() {
    let names = dependency_names(&manifest("tgx-parity"));
    assert!(
        !names.iter().any(|n| n == "tgx-tg"),
        "tgx-parity/Cargo.toml depends on tgx-tg — the oracle must only ever replay \
         recorded data (a real Desktop export, or the corpus cut from one) through our \
         writers. Depending on tgx-tg would let it reach for a live client instead of a \
         fixture, and the harness would no longer run offline and deterministically. \
         See CLAUDE.md's Architecture section."
    );
}
