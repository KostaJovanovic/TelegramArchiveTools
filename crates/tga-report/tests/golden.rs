//! The report against a committed copy of itself.
//!
//! **This catches drift. It does not claim correctness.** What it adds over
//! `save.bat baseline` — which compares whole reports on two real archives — is
//! that it runs on a fresh clone with no corpus and no drives, and that it names
//! the character where an edit changed the output.
//!
//! **The fixture is generated from a real archive and then scrubbed** — see
//! `tools/make_fixture.py`, which is the only thing that should ever write it.
//! It was hand-written until it wasn't, and hand-written was worse in a way
//! worth recording: every histogram in it was a number somebody invented and
//! then made self-consistent, so it was flat and short-tailed. The quantile
//! ramp exists *because* real chat activity is long-tailed, and the only
//! fixture that exercised it was one no chat could have produced.
//!
//! What it must not be is a real archive. `*.stats.json` is gitignored because
//! a dump is verbatim history from real people, and this file is committed. So
//! every display name, `@handle`, peer key and message snippet is replaced —
//! including inside map *keys*, which is where that kind of scrub usually
//! leaks — while every number, date and topic name survives untouched.
//!
//! On top of that the script injects the shapes a healthy export does not have
//! and a renderer needs tested: a name carrying `&` and `<`, an alias list, a
//! role that is not `member`, a roster the export admits is short, and one it
//! says it truncated. Those are marked in `inject()` with why.
//!
//! Two branches came free with the real data that the old seven-day fixture
//! could not reach at all — somebody fading out, and an hour with too few
//! replies to time sitting beside one with enough. The second used to require
//! lowering `dynamics.answer.minimum` from 20 to 3; the floor is the real one
//! now.
//!
//! The fonts are **not** embedded in the golden. Three base64'd faces are
//! ~270 KB of noise in a committed file and they are already covered by
//! `assets::tests`; leaving them out keeps the golden readable in a diff, which
//! is the only reason to have one.
//!
//! To re-bless after a deliberate change:
//!
//! ```text
//! TGA_BLESS=1 cargo test -p tga-report --test golden
//! ```
//!
//! and read the resulting diff before committing it. A golden updated without
//! being read is a golden that records whatever the bug did.

use std::path::{Path, PathBuf};

use tga_notes::{Coverage, Event, Notes};
use tga_stats::Stats;

fn here() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// A single annotation, so the event rail, the marker and the card all render.
///
/// Written here rather than read from a file: the events layer has its own
/// tests in `tga-notes`, and what this fixture is for is the *markup* an event
/// produces.
///
/// **The dates have to sit inside the archive's span**, which is 14 Dec 2025 to
/// 26 Aug 2026. An event outside it lands off the end of the rail, where it
/// draws nothing and pins nothing — the marker tests would then be asserting
/// against a chart with no marks on it and would still pass.
fn events() -> Vec<Event> {
    vec![
        Event {
            id: "migration".into(),
            start: chrono::NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
            end: Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 6).unwrap()),
            title: "Moved off the old group".into(),
            summary: "A span, with citations and a confidence.".into(),
            kind: "milestone".into(),
            topic: Some(0),
            messages: vec![1, 2, 17],
            confidence: "high".into(),
            // The fields only the `_timeline` notes layout carries.
            time: "20:59".into(),
            weight: "major".into(),
            who: vec!["Person 2".into(), "Nobody At All".into()],
            tags: vec!["osnivanje".into(), "kanali".into()],
        },
        Event {
            id: "quiet".into(),
            start: chrono::NaiveDate::from_ymd_opt(2026, 1, 7).unwrap(),
            end: None,
            title: "A moment, uncited & unsure".into(),
            summary: String::new(),
            // The loader defaults an absent kind to `milestone`, so an event
            // with no kind at all is a shape only a hand-built `Event`
            // reaches, and `rail` has a branch for it that nothing else covers.
            kind: String::new(),
            topic: None,
            messages: vec![],
            confidence: "low".into(),
            time: String::new(),
            weight: "minor".into(),
            who: vec![],
            tags: vec![],
        },
        // A second *named* kind, so the filter row is a real choice. With one
        // kind `kind_filters` renders nothing — a control that can only be
        // all-on or all-off is not a filter — and the golden would then pin an
        // absence rather than the feature.
        Event {
            id: "split".into(),
            start: chrono::NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(),
            end: None,
            title: "An argument".into(),
            summary: "A second kind, so the filters have something to choose between.".into(),
            kind: "drama".into(),
            topic: None,
            messages: vec![9],
            confidence: String::new(),
            time: "13:20".into(),
            weight: "medium".into(),
            who: vec!["Bob & Co <the second>".into()],
            tags: vec![],
        },
    ]
}

fn notes() -> Notes {
    Notes {
        events: events(),
        coverage: Some(Coverage {
            from: chrono::NaiveDate::from_ymd_opt(2025, 12, 14),
            to: chrono::NaiveDate::from_ymd_opt(2026, 1, 31),
            level: "turning points".into(),
            note: "Only the first seven weeks were read.".into(),
        }),
        source: "events.json, written by a test".into(),
    }
}

fn stats() -> Stats {
    serde_json::from_str(
        &std::fs::read_to_string(here().join("fixture.stats.json"))
            .expect("the fixture is committed beside this test"),
    )
    .expect("the fixture still matches the shape `analyse` returns")
}

fn rendered() -> String {
    let stats = stats();
    tga_report::render(
        &stats,
        &tga_report::names_from_stats(&stats),
        &notes(),
        &tga_report::Options {
            embed_fonts: false,
            stamp: "5 September 2025".into(),
            ..Default::default()
        },
    )
}

#[test]
fn the_report_matches_the_committed_golden() {
    check_golden("golden.html", rendered());
}

#[test]
fn every_interactive_feature_is_drawn_into_the_document() {
    // The report renders every view server-side and lets JS only filter and pan
    // what is already there, so each of these is markup rather than a class
    // name. **Markup, not CSS.** The stylesheet names `.kind-filter` and
    // `.cov-read` whether or not anything uses them, so matching those alone
    // would let a feature that renders nothing pass on the strength of its own
    // rules.
    let html = rendered();
    for (what, needle) in [
        ("the search box", "<input class=\"find\""),
        ("the shared axis", "<div id=\"axis\" hidden"),
        ("the kind filters", "<div class=\"kinds\">"),
        ("a kind toggle", "<button class=\"kind-filter on\""),
        ("the coverage band", "<rect class=\"cov-read\""),
        ("the coverage line", "<p class=\"coverage\">"),
        ("the compact presence rows", "<path class=\"c"),
        ("the `who` line", "<p class=\"who\">"),
        ("the tag line", "<p class=\"tags\">"),
        ("a shape-coded marker", "<g class=\"ev k"),
        ("the between-people section", "<section id=\"between\">"),
        ("its nav entry", "href=\"#between\""),
    ] {
        assert!(html.contains(needle), "the report lost {what}");
    }
}

fn check_golden(name: &str, ours: String) {
    let path = here().join(name);

    if std::env::var("TGA_BLESS").as_deref() == Ok("1") {
        std::fs::write(&path, &ours).expect("write golden");
        eprintln!(
            "BLESSED {} ({} bytes) — read the diff",
            path.display(),
            ours.len()
        );
        return;
    }

    let golden = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "no golden at {} — run `TGA_BLESS=1 cargo test -p tga-report --test golden`",
            path.display()
        )
    });

    if golden == ours {
        return;
    }
    let at = golden
        .char_indices()
        .zip(ours.char_indices())
        .find(|((_, a), (_, b))| a != b)
        .map(|((i, _), _)| i)
        .unwrap_or_else(|| golden.len().min(ours.len()));
    let window = |text: &str| {
        let from = at.saturating_sub(160).min(text.len());
        let to = (at + 200).min(text.len());
        text[from..to].to_string()
    };
    panic!(
        "the report drifted from the golden at character {at} \
         ({} bytes golden / {} bytes now)\n  golden: {}\n  now   : {}\n\n\
         If the change was deliberate: TGA_BLESS=1 cargo test -p tga-report --test golden",
        golden.len(),
        ours.len(),
        window(&golden),
        window(&ours)
    );
}

#[test]
fn the_golden_exercises_the_cases_it_was_built_for() {
    // A golden is only worth its bytes if it covers the awkward shapes. This
    // fails if the fixture is ever trimmed down to something tidy.
    let html = rendered();
    let mut missing: Vec<String> = Vec::new();
    for (what, needle) in [
        ("an escaped name", "Bob &amp; Co &lt;the second&gt;"),
        ("an alias list", "class=\"alias\">was Bobby, B."),
        ("a non-member role", "class=\"role\">creator"),
        ("the overflow-free people table", "class=\"rank\">1<"),
        // -- the member list ------------------------------------------------
        (
            "a silent member",
            "2 of the 43 people on the member list never posted",
        ),
        (
            "an incomplete roster",
            "export says this member list is incomplete",
        ),
        ("a capped roster", "stopped collecting members"),
        (
            "people who talked but are not listed",
            "are not on the member list at all",
        ),
        ("a handle", "class=\"handle\">@handle_1"),
        ("a member with no handle", "class=\"muted\">no handle"),
        ("a member who never posted", "class=\"muted\">never posted"),
        ("a non-ASCII topic", "ćaskanje"),
        // -- per-topic figures ----------------------------------------------
        ("a topic's own busiest day", "class=\"at\">18 Aug 2026"),
        ("the per-topic clock", "<h3>When each topic is awake</h3>"),
        (
            "a cell scaled to its own row",
            "157 of 642 at this row's peak",
        ),
        ("a string-valued superlative", "class=\"big\">47.1 MB<"),
        ("a spanning event", "ev-span"),
        // `ev k1 conf-low`: the shape class sits between the two, so match the
        // part that is the actual claim.
        ("a low-confidence event", "conf-low"),
        ("a shape-coded marker", "class=\"ev k0"),
        (
            "the coverage prose",
            "Only the first seven weeks were read.",
        ),
        (
            "a name that matches nobody",
            "class=\"unknown\">Nobody At All",
        ),
        (
            "a name that does match",
            "class=\"who\"><span>Person 2</span>",
        ),
        ("the weight", "class=\"w w-major\""),
        ("the time of day", "class=\"at\">20:59"),
        ("a tag", "class=\"tags\"><span>osnivanje"),
        ("an uncited event", "no messages cited"),
        ("an event with no summary", "No summary."),
        (
            "the hidden-node note",
            "The 16 least-connected people are left out",
        ),
        ("orphan replies", "11 replies point at a message"),
        (
            "an export with no roster dates",
            "carries no member list, so arrivals cannot be dated",
        ),
        ("skipped media", "of those attachments were over the"),
        ("a quantile caption", "Five shades, one per fifth"),
        (
            "the streak",
            "Longest unbroken run of days posted on: Bob &amp; Co &lt;the second&gt;, 9 days",
        ),
        // -- the `dynamics` branch ------------------------------------------
        //
        // These used to need a doctored `answer.minimum` of 3: on a 22-message
        // hand-written fixture every hour fell under the real floor of 20, so
        // only the "not enough replies" branch rendered. A 6,687-message
        // archive clears the floor in some hours and not others, which is the
        // shape the code was written for — the floor is the real one now, and
        // both branches below come from it rather than from a distortion.
        (
            "a correspondent pair",
            "Bob &amp; Co &lt;the second&gt; &amp; Person 4",
        ),
        ("both directions of a pair", "13 \u{2194} 10"),
        ("an hour that met the floor", "<td>2 min</td>"),
        ("an hour that did not", "<td>&#8212;</td>"),
        ("the chain-length buckets", " chains\""),
        ("the month-over-month chart", "<h3>Who came back</h3>"),
        // Nine months of real archive reaches both of these. The old fixture
        // covered seven days, in which nobody *can* fade or leave, so they were
        // pushed out to unit tests in `lib.rs`; the golden carries them now.
        ("somebody fading out", "<td>fading</td>"),
        (
            "dormancy measured against the archive",
            "26 Aug 2026, the last day in this archive",
        ),
    ] {
        if !html.contains(needle) {
            missing.push(format!("  {what}: {needle:?}"));
        }
    }
    // Collected rather than asserted one at a time. Regenerating the fixture
    // moves many of these at once, and a test that stops at the first miss
    // turns one edit into a dozen rebuild-and-look-again rounds.
    assert!(
        missing.is_empty(),
        "the golden lost {} of the cases it was built for:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn the_golden_is_a_whole_self_contained_document() {
    let html = rendered();
    assert!(html.starts_with("<!doctype html>\n"));
    assert!(html.ends_with("</body></html>\n"));
    assert_eq!(
        html.matches("<section").count(),
        html.matches("</section>").count()
    );
    for scheme in ["http://", "https://", "//cdn"] {
        assert!(!html.contains(scheme), "the golden reaches for {scheme}");
    }
}
