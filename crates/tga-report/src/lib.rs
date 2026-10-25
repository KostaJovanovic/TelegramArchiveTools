//! Assemble one self-contained `report.html`.
//!
//! Self-contained means exactly that: one file, no folder beside it, no request
//! to anything. The fonts are base64'd into the stylesheet, the charts are
//! inline SVG, and the script only filters and pans what is already drawn. An
//! archive is something you keep, and a report that needs a CDN to render is a
//! report that stops working the year the CDN does.
//!
//! The stylesheet and the script live in `assets/`, pulled in with
//! `include_str!`. They are copied into the report verbatim, so `.gitattributes`
//! pins their line endings for the same reason it pins the golden's.
//!
//! The visual language is quiet: each chart sits in a low panel with one
//! plain sentence above it saying what it shows, and the method behind a (?)
//! that opens on a tap. Bars are a muted grey and only the mark a chart points
//! at is red. Numbers are set in Geist Mono, and section headings are
//! letterspaced uppercase micro-type. The page is read on a phone as often as
//! on a desk, so every mark whose label is the point is laid out by the
//! browser rather than scaled inside an SVG; see `charts::html`.
//!
//! **This crate depends on neither `tga-read` nor `tga-metrics.`** It renders
//! from a [`tga_stats::Stats`], which is exactly what `--stats` dumps, and that
//! is what lets `tests/golden.rs` replay a recorded fixture through the writer
//! with no export on disk. See `Cargo.toml`.

use tga_notes::Notes;
use tga_stats::Stats;

pub mod assets;
pub mod charts;
pub mod palette;
pub mod sections;
pub mod stats;

pub use charts::esc;
pub use sections::{Names, WIDTH};

// ---------------------------------------------------------------------------
// the stylesheet
// ---------------------------------------------------------------------------

/// The custom properties, as one `<html>` class block.
///
/// **The report is dark only.** One block, and no control to switch away from
/// it: a second appearance is a second design to keep in step, and this one has
/// two colours and a red to keep in step already.
fn tokens_css() -> String {
    let pairs: String = palette::tokens()
        .iter()
        .map(|(k, v)| format!("--{k}:{v};"))
        .collect();
    let ramp: String = palette::ramp()
        .iter()
        .enumerate()
        .map(|(i, c)| format!("--r{}:{c};", i + 1))
        .collect();
    format!(
        "html.dark{{{pairs}{ramp}--self:{};}}",
        palette::token("rule")
    )
}

/// The stylesheet, in `assets/report.css`.
///
/// **A `/* */` comment in that file is emitted into every report**, because the
/// whole file is. Keep them to the ones a reader of the report would want.
///
/// It was two constants until the move: the second half was split off so that a
/// frozen reproduction of an older report could omit it. That render is gone.
/// Merging them let the one rule that existed only to override another go with
/// it -- the `details[open]` marker shipped a C1 control character followed by
/// an ASCII `2`, which is what a minus sign becomes after a bad encoding round
/// trip and which a browser draws as a stray `2`. It is a real minus now.
const CSS: &str = include_str!("../assets/report.css");

/// The whole of the interactivity, in `assets/report.js`.
///
/// Every view on the page is already rendered into the SVG; this only filters
/// and highlights what is drawn. A report that renders blank with scripting off
/// has stopped being an archive.
const JS: &str = include_str!("../assets/report.js");

// ---------------------------------------------------------------------------
// entry point
// ---------------------------------------------------------------------------

pub const SOURCE: &str = "Telegram Export Analyser";

/// Today, as the notes section stamps it: `5 September 2025`.
pub fn today_stamp() -> String {
    let text = chrono::Local::now().format("%d %B %Y").to_string();
    text.trim_start_matches('0').to_string()
}

#[derive(Debug, Clone)]
pub struct Options {
    pub embed_fonts: bool,
    /// Who the notes section says wrote the file.
    pub source: String,
    /// The date the notes section stamps.
    ///
    /// Injected rather than read from the clock inside `render`, which is what
    /// makes the whole document reproducible: it is the only value on the page
    /// that would otherwise differ between two runs on the same export. The
    /// golden and `save.bat baseline` both rest on that, and `--stamp` is how
    /// the command line pins it.
    pub stamp: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            embed_fonts: true,
            source: SOURCE.to_string(),
            stamp: today_stamp(),
        }
    }
}

/// Peer key -> display name, built from the stats themselves.
///
/// The two charts that label by key — the who-answers-whom matrix and the
/// contact graph — need a name for an arbitrary peer key. `people.rows` carries
/// one for every key that can reach them: anyone who spoke has a row, anyone
/// who only ever reacted gets one through `votes_given`, and a roster member
/// who never posted gets one as `silent`. Building the lookup from the stats
/// rather than taking `tga_metrics::People` is what keeps this crate off
/// `tga-metrics` and lets a recorded `stats.json` render on its own.
pub fn names_from_stats(stats: &Stats) -> Names {
    stats
        .people
        .rows
        .iter()
        .map(|row| (row.key.clone(), row.name.clone()))
        .collect()
}

/// The whole report, as one string.
pub fn render(stats: &Stats, names: &Names, notes: &Notes, options: &Options) -> String {
    let title = format!("{} — archive report", stats.export.name);

    // The churn section's heading is written here rather than inside `churn`,
    // because that function returns nothing at all for an export with no
    // dated months and the `<section>` still has to close. A private chat has
    // no comings and goings to speak of, and gets no section.
    let churn = if sections::is_duo(stats) {
        String::new()
    } else {
        format!(
            "{}{}</section>",
            sections::head("Coming and going", "", "churn"),
            sections::churn(stats)
        )
    };
    let people_title = if sections::is_duo(stats) {
        "The two of you"
    } else {
        "People"
    };
    let parts: [(&str, &str, String); 15] = [
        ("whole", "Timeline", sections::timeline(stats, notes, names)),
        ("rhythm", "Rhythm", sections::rhythm(stats)),
        ("trend", "Over time", sections::trend(stats)),
        ("people", people_title, sections::people(stats)),
        ("talk", "Conversation", sections::conversation(stats, names)),
        ("between", "Between people", sections::between(stats)),
        ("said", "What was said", sections::said(stats)),
        ("words", "Words", sections::words(stats)),
        ("reactions", "Reactions", sections::reactions(stats)),
        ("polls", "Polls", sections::polls(stats)),
        ("churn", "Coming and going", churn),
        ("history", "Group history", sections::history(stats)),
        ("topics", "Topics", sections::topics(stats)),
        ("records", "Records", sections::records(stats)),
        (
            "notes",
            "Notes",
            sections::notes(stats, &notes.source, &options.source, &options.stamp),
        ),
    ];
    // The nav is built from what was written, so no link scrolls nowhere.
    let nav: Vec<(&str, &str)> = parts
        .iter()
        .filter(|(_, _, html)| !html.is_empty())
        .map(|(anchor, title, _)| (*anchor, *title))
        .collect();
    let mut body = sections::masthead(stats, &nav);
    for (_, _, html) in &parts {
        body.push_str(html);
    }
    body.push_str(&sections::focus_data(stats));

    // The compact presence rows leave their bucket labels out of every cell —
    // that omission is most of the size budget — so the labels are written into
    // the document once, here, and the tooltip is composed from the pointer's
    // position.
    let axis = shared_axis(stats);
    let kind_rules = kind_css(notes);

    format!(
        "<!doctype html>\n<html lang=\"en\" class=\"{}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{}</title><style>{}{}{CSS}{kind_rules}</style></head><body>{body}{axis}\
         <div id=\"tip\" role=\"status\"></div><script>{JS}</script></body></html>\n",
        palette::DEFAULT,
        esc(&title),
        assets::font_css(options.embed_fonts),
        tokens_css(),
    )
}

/// Every presence row's bucket labels, written once for the document.
///
/// Every row is bucketed identically — same date range, same width, same
/// `bucket_days` — so one list serves all 260 of them. Emitting it per cell is
/// what made the presence rows 64% of a large report.
fn shared_axis(stats: &Stats) -> String {
    let series = &stats.activity.per_day;
    if series.is_empty() {
        return String::new();
    }
    let labels = charts::bucket_labels(series, sections::PRESENCE_WIDTH);
    format!(
        "<div id=\"axis\" hidden data-days=\"{}\"></div>",
        esc(&labels.join("|"))
    )
}

/// The show/hide rule for each kind the notes file actually used.
///
/// Generated rather than fixed at four, because the vocabulary belongs to
/// whoever wrote the file. A file with three kinds gets three rules and no dead
/// fourth; a file with six gets six.
fn kind_css(notes: &Notes) -> String {
    let kinds = notes.kinds();
    if kinds.len() < 2 {
        return String::new();
    }
    (0..kinds.len())
        .map(|k| format!("html.hide-k{k} .ev.k{k},html.hide-k{k} .events li.k{k}{{display:none}}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tga_stats::{Activity, Count, Dynamics, Tenure, TenureRow};

    /// The smallest stats value that exercises every branch's absence.
    ///
    /// Almost all of it is `Default`, which is the point: the writer has to
    /// produce a whole document from a run that found nothing, and every
    /// section has to decide for itself what to say about having no data.
    fn bare() -> Stats {
        Stats {
            export: tga_stats::ExportInfo {
                name: "Nothing".into(),
                root: "C:\\x".into(),
                ..Default::default()
            },
            activity: Activity {
                empty: true,
                ..Default::default()
            },
            conversation: tga_stats::Conversation {
                session_gap: 1800,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Five days of activity, so the timeline has an axis to draw on.
    fn dated() -> Activity {
        Activity {
            empty: false,
            first: "2025-01-01".into(),
            last: "2025-01-05".into(),
            span_days: 5,
            active_days: 2,
            per_day: vec![
                Count::new("2025-01-01", 3),
                Count::new("2025-01-02", 0),
                Count::new("2025-01-03", 1),
                Count::new("2025-01-04", 0),
                Count::new("2025-01-05", 2),
            ],
            per_month: vec![Count::new("2025-01", 6)],
            ..Default::default()
        }
    }

    fn options() -> Options {
        Options {
            stamp: "5 September 2025".into(),
            ..Default::default()
        }
    }

    #[test]
    fn an_export_with_nothing_in_it_still_renders_a_whole_document() {
        // The report is what somebody gets back from a folder that turned out
        // to be empty. Failing here would leave them with no output and no
        // explanation.
        let html = render(&bare(), &Names::new(), &Notes::default(), &options());
        assert!(html.starts_with("<!doctype html>\n"));
        assert!(html.ends_with("</body></html>\n"));
        assert!(html.contains("No dated messages."));
        assert!(html.contains("no dated messages"));
    }

    #[test]
    fn every_section_closes_the_tag_it_opened() {
        let html = render(&bare(), &Names::new(), &Notes::default(), &options());
        assert_eq!(
            html.matches("<section").count(),
            html.matches("</section>").count(),
            "unbalanced sections"
        );
    }

    #[test]
    fn the_report_is_dark_and_offers_no_choice_about_it() {
        let html = render(&bare(), &Names::new(), &Notes::default(), &options());
        assert!(html.contains("<html lang=\"en\" class=\"dark\">"));
        assert!(html.contains("html.dark{"));
        assert!(html.contains("--r5:"));
        assert!(html.contains("--self:"));

        // One block, one switch fewer. A light block nothing can reach is dead
        // weight in every report ever written; a switch to it with no block is
        // a page with no colours.
        assert!(!html.contains("html.light{"), "the surface emits one block");
        assert!(
            !html.contains("id=\"theme\""),
            "and no control to change it"
        );
    }

    /// A `dynamics` branch whose people have stopped posting.
    ///
    /// The golden fixture cannot reach this: its archive is seven days long, so
    /// nobody in it can be thirty days dormant and the section correctly renders
    /// the "everybody posted in the last month" line instead. The populated
    /// table is real markup and needs a case of its own.
    fn drifted() -> Stats {
        Stats {
            dynamics: Some(Dynamics {
                empty: false,
                answer: tga_stats::Answer {
                    minimum: 20,
                    cap: 86400,
                    ..Default::default()
                },
                tenure: Tenure {
                    as_of: "2025-06-30".into(),
                    active: 0,
                    fading: 1,
                    gone: 1,
                    active_within: 30,
                    fading_within: 90,
                    rows: vec![
                        TenureRow {
                            key: "user1".into(),
                            name: "Ana".into(),
                            messages: 400,
                            first: "2025-01-01".into(),
                            last: "2025-05-20".into(),
                            span_days: 140,
                            active_days: 40,
                            density: 0.2857142857142857,
                            dormant_days: 41,
                            status: "fading".into(),
                        },
                        TenureRow {
                            key: "user2".into(),
                            // `&` and `<` on purpose: this row is also what
                            // pins the escaping on the way into the table.
                            name: "Bob & Co <the second>".into(),
                            messages: 12,
                            first: "2025-01-01".into(),
                            last: "2025-01-04".into(),
                            span_days: 4,
                            active_days: 2,
                            density: 0.5,
                            dormant_days: 177,
                            status: "gone".into(),
                        },
                    ],
                },
                retention: tga_stats::Retention {
                    people: 2,
                    ..Default::default()
                },
                depth: tga_stats::Depth {
                    cap: 8,
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..bare()
        }
    }

    #[test]
    fn the_people_who_stopped_posting_are_ranked_by_the_silence() {
        let html = render(&drifted(), &Names::new(), &Notes::default(), &options());
        // Longest silence first, which is the one ordering the People table
        // cannot give — it ranks by message count, and there Ana comes first.
        let bob = html.find("Bob &amp; Co").expect("the quiet one is listed");
        let ana = html.find(">Ana<").expect("the fading one is listed");
        assert!(
            bob < ana,
            "the table is ranked by dormancy, not by messages"
        );
        assert!(html.contains("<td class=\"n\" data-l=\"Days since\">177</td>"));
        assert!(html.contains("30 Jun 2025, the last day in this archive"));
        // Escaped like every other name that came out of an export.
        assert!(!html.contains("Bob & Co <the second>"));
    }

    #[test]
    fn the_pair_table_heads_its_directions_with_arrows_not_entity_names() {
        let mut stats = drifted();
        if let Some(dynamics) = stats.dynamics.as_mut() {
            dynamics.pairs.rows = vec![tga_stats::Pair {
                a: "user1".into(),
                b: "user2".into(),
                a_name: "Ana".into(),
                b_name: "Bob".into(),
                a_to_b: 3,
                b_to_a: 2,
                both: 2,
                replies: 5,
                balance: 0.8,
            }];
        }
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains(
            "<th class=\"n\" data-sort=\"n\">\u{2192}</th><th class=\"n\" data-sort=\"n\">\u{2190}</th>"
        ));
        assert!(!html.contains("&amp;rarr;") && !html.contains("&amp;larr;"));
    }

    #[test]
    fn who_came_back_is_read_from_an_older_dump_as_well() {
        // An older dump has no `came_back`; it is what is left of `active`
        // once the new and the people who stayed are taken out.
        let mut stats = drifted();
        if let Some(dynamics) = stats.dynamics.as_mut() {
            dynamics.retention = tga_stats::Retention {
                months: vec!["2025-01".into(), "2025-02".into(), "2025-03".into()],
                active: vec![2, 2, 3],
                new: vec![2, 1, 0],
                returning: vec![0, 1, 2],
                lost: vec![0, 1, 0],
                people: 3,
                ..Default::default()
            };
        }
        let old = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(old.contains(
            "<th class=\"n\" data-sort=\"n\">Stayed</th><th class=\"n\" data-sort=\"n\">Came back</th>"
        ));
        assert!(old.contains(
            "<td data-l=\"Month\">2025-03</td><td class=\"n\" data-l=\"Spoke\">3</td>\
             <td class=\"n\" data-l=\"New\">0</td><td class=\"n\" data-l=\"Stayed\">2</td>\
             <td class=\"n\" data-l=\"Came back\">1</td>"
        ));
        if let Some(dynamics) = stats.dynamics.as_mut() {
            dynamics.retention.came_back = vec![0, 0, 1];
        }
        let new = render(&stats, &Names::new(), &Notes::default(), &options());
        assert_eq!(old, new, "the stored series and the derived one agree");
    }

    #[test]
    fn everybody_quiet_has_a_row_and_the_right_population() {
        let person = |key: &str, messages: i64, listed: bool| tga_stats::Person {
            key: key.into(),
            name: key.into(),
            messages,
            listed,
            ..Default::default()
        };
        let mut stats = bare();
        stats.activity = dated();
        stats.people.rows = vec![
            person("user1", 6, true),
            person("user2", 0, true),
            person("user9", 0, false),
        ];
        stats.people.speakers = 1;
        stats.people.known_members = 2;
        stats.people.members_who_spoke = 1;
        stats.people.silent_members = 1;
        stats.people.quiet_unlisted = 1;
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        // One table: the speaker, the silent member and the reactor each have
        // a row, and the quiet two are filed under Never posted.
        assert!(html.contains("<tr class=\"posted\" data-who=\"user1\">"));
        assert!(html.contains("<tr class=\"silent\" data-who=\"user2\">"));
        assert!(html.contains("<tr class=\"silent\" data-who=\"user9\">"));
        assert!(
            html.contains("data-f=\"silent\" aria-pressed=\"false\">Never posted <span>2</span>")
        );
        assert!(
            !html.contains("id=\"members\""),
            "Members is part of People now"
        );

        // With no member list the method says where the quiet rows came from.
        stats.people.known_members = 0;
        stats.people.silent_members = 0;
        stats.people.quiet_unlisted = 2;
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains("This export carries no member list."));
        assert!(html.contains("<tr class=\"silent\" data-who=\"user9\">"));
    }

    #[test]
    fn a_private_chat_gets_the_side_by_side_layout() {
        let person = |key: &str, messages: i64| tga_stats::Person {
            key: key.into(),
            name: key.into(),
            messages,
            listed: true,
            ..Default::default()
        };
        let mut stats = bare();
        stats.activity = dated();
        stats.people.rows = vec![person("ana", 40), person("bob", 10)];
        stats.people.speakers = 2;
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains("A private chat between ana and bob"));
        assert!(html.contains("class=\"chart ribbon mirror\""));
        assert!(html.contains("<table class=\"vs\">"));
        assert!(
            !html.contains("class=\"rows people-table\""),
            "no ranked table of two"
        );
        assert!(!html.contains("id=\"churn\""), "no comings and goings");
        assert!(
            !html.contains("id=\"focus-data\""),
            "nothing to focus between two"
        );
    }

    #[test]
    fn a_name_cannot_close_the_focus_script() {
        let mut stats = bare();
        stats.activity = dated();
        stats.people.rows = ["a", "b", "c"]
            .iter()
            .map(|k| tga_stats::Person {
                key: (*k).into(),
                name: "</script><img src=x>".into(),
                messages: 3,
                ..Default::default()
            })
            .collect();
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains("id=\"focus-data\""));
        assert!(!html.contains("</script><img"));
    }

    #[test]
    fn the_award_caption_states_the_floor_the_metrics_used() {
        let mut stats = bare();
        stats.awards = vec![tga_stats::Award {
            title: "Night owl".into(),
            name: "Ana".into(),
            key: "user1".into(),
            value: "50%".into(),
            note: String::new(),
            messages: 80,
        }];
        stats.people.rows = vec![tga_stats::Person {
            key: "user1".into(),
            name: "Ana".into(),
            messages: 80,
            ..Default::default()
        }];
        stats.activity = dated();
        stats.award_minimum = Some(25);
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains("Restricted to people with 25 messages or more"));
        stats.award_minimum = None;
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains("Restricted to people who posted often enough to mean it"));
    }

    #[test]
    fn the_burst_gap_is_the_one_the_metrics_recorded() {
        let mut stats = bare();
        stats.conversation.session_gap = 45 * 60;
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains("no gap longer than 45 min,"));
        assert!(!html.contains("half an hour"));
    }

    #[test]
    fn a_report_rendered_from_a_dump_without_dynamics_is_still_whole() {
        // Every `--from-stats` dump recorded before the branch existed has this
        // shape, and a re-render of one must not lose a section or a nav entry
        // it never had.
        let html = render(&bare(), &Names::new(), &Notes::default(), &options());
        assert!(!html.contains("id=\"between\""));
        assert!(!html.contains("#between"));
        assert!(html.ends_with("</body></html>\n"));
    }

    /// All six newer branches, populated, with a hostile string in every
    /// place one can come from the export.
    fn filled() -> Stats {
        use tga_stats::*;
        let hostile = "<b>x</b> & co";
        let months = vec!["2025-01".to_string(), "2025-02".to_string()];
        Stats {
            activity: dated(),
            polls: Some(Polls {
                rows: vec![PollRow {
                    id: 7,
                    date: "2025-01-02 10:00".into(),
                    key: "user1".into(),
                    name: "Ana".into(),
                    question: format!("{hostile}?"),
                    voters: 9,
                    answers: vec![Count::new("petak", 6), Count::new(hostile, 3)],
                    ..Default::default()
                }],
                total: 1,
                shown: 1,
                votes: 9,
                median_voters: 9,
                per_month: vec![Count::new("2025-01", 1), Count::new("2025-02", 0)],
                ..Default::default()
            }),
            voice: Some(Voice {
                voice_notes: 3,
                voice_seconds: 200,
                median_seconds: 60,
                per_month: vec![Count::new("2025-01", 200), Count::new("2025-02", 0)],
                people: vec![VoiceRow {
                    key: "user1".into(),
                    name: "Ana".into(),
                    voice_notes: 3,
                    seconds: 200,
                    share: 0.5,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            history: Some(History {
                events: vec![
                    HistoryEvent {
                        date: "2025-01-03 12:00".into(),
                        action: "edit_group_title".into(),
                        name: "Ana".into(),
                        title: hostile.into(),
                        ..Default::default()
                    },
                    HistoryEvent {
                        date: "2025-01-04 12:00".into(),
                        action: "pin_message".into(),
                        name: "Bob".into(),
                        pinned: Some(3),
                        pinned_who: "Ana".into(),
                        pinned_text: hostile.into(),
                        ..Default::default()
                    },
                ],
                total: 2,
                shown: 2,
                kinds: vec![
                    Count::new("edit_group_title", 1),
                    Count::new("pin_message", 1),
                ],
            }),
            // Three months, so the latency panel has two readings to draw and
            // one month it could not read between them.
            trends: Some(Trends {
                months: vec!["2025-01".into(), "2025-02".into(), "2025-03".into()],
                messages: vec![4, 2, 3],
                voices: vec![2, 1, 2],
                top3_share: vec![1.0, 1.0, 1.0],
                reply_share: vec![0.25, 0.5, 0.0],
                latency_median: vec![Some(120), None, Some(60)],
                latency_minimum: 20,
                mean_words: vec![3.5, 2.0, 1.0],
                media_share: vec![0.0, 0.5, 0.0],
                reactions_per_message: vec![0.5, 0.0, 0.0],
                leaders: vec![
                    Some(Leader {
                        key: "user1".into(),
                        name: hostile.into(),
                        messages: 3,
                        share: 0.75,
                    }),
                    None,
                    None,
                ],
                ..Default::default()
            }),
            reactions: Some(Reactions {
                total: 5,
                named: 3,
                reacted: 2,
                messages: 6,
                kinds: vec![Count::new("\u{1F44D}", 4), Count::new("custom emoji", 1)],
                custom: 1,
                months: months.clone(),
                per_month: vec![5, 0],
                reacted_per_month: vec![2, 0],
                messages_per_month: vec![4, 2],
                people: vec![ReactionRow {
                    key: "user1".into(),
                    name: "Ana".into(),
                    messages: 4,
                    received: 5,
                    received_top: vec![Count::new("\u{1F44D}", 4)],
                    ..Default::default()
                }],
            }),
            words: Some(Words {
                tokens: 40,
                vocabulary: 12,
                people: vec![WordSet {
                    key: "user1".into(),
                    name: "Ana".into(),
                    tokens: 20,
                    words: vec![Count::new(hostile, 5)],
                }],
                script: Script {
                    cyrillic: 2,
                    latin: 998,
                    messages_latin: 6,
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..bare()
        }
    }

    #[test]
    fn every_newer_section_renders_and_is_in_the_nav() {
        let html = render(&filled(), &Names::new(), &Notes::default(), &options());
        for anchor in ["trend", "words", "reactions", "polls", "history"] {
            assert!(
                html.contains(&format!("<section id=\"{anchor}\">")),
                "{anchor}"
            );
            assert!(
                html.contains(&format!("href=\"#{anchor}\"")),
                "{anchor} in the nav"
            );
        }
        assert!(html.contains("<h3>Voice and video notes</h3>"));
        assert!(html.contains("<h3>How concentrated the talk was</h3>"));
        assert!(html.contains("<h3>Who led each month</h3>"));
        assert!(html.contains("renamed the group to <b>"));
        assert!(html.contains("pinned a message by Ana:"));
        // The winning answer is the bold one.
        assert!(html.contains("<b>petak</b><span class=\"num muted\"> 6</span>"));
        // An unread month is a tooltip, not a zero.
        assert!(html.contains("02 · no reading"));
        // Under the script floor, one sentence and no charts.
        assert!(
            html.contains("Written almost entirely in Latin: 0.2% of the letters are Cyrillic.")
        );
        assert!(!html.contains("<h3>Who writes in Cyrillic</h3>"));
        assert!(html.contains("of the 5 reactions here"));
    }

    #[test]
    fn nothing_from_the_export_escapes_into_the_newer_sections() {
        let html = render(&filled(), &Names::new(), &Notes::default(), &options());
        assert!(
            !html.contains("<b>x</b> & co"),
            "a raw string reached the page"
        );
        assert!(html.contains("&lt;b&gt;x&lt;/b&gt; &amp; co"));
    }

    #[test]
    fn a_chat_written_in_both_scripts_gets_both_charts() {
        // The golden's archive is 0.6% Cyrillic, under the floor, so it only
        // ever renders the one-sentence form.
        let mut stats = filled();
        if let Some(words) = stats.words.as_mut() {
            words.script = tga_stats::Script {
                cyrillic: 300,
                latin: 700,
                months: vec!["2025-01".into(), "2025-02".into()],
                month_cyrillic: vec![0.1, 0.5],
                people: vec![tga_stats::ScriptRow {
                    key: "user1".into(),
                    name: "Ana".into(),
                    cyrillic: 300,
                    latin: 100,
                    share: 0.75,
                }],
                ..Default::default()
            };
        }
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains("<h3>Who writes in Cyrillic</h3>"));
        assert!(html.contains("<b>30.0%</b> of all the letters are Cyrillic"));
        assert!(html.contains("75.0% Cyrillic"));
        assert!(html.contains("<h3>Cyrillic, month by month</h3>"));
        assert!(!html.contains("Written almost entirely in"));
    }

    #[test]
    fn a_private_chat_has_no_ranking_of_two_over_time() {
        let mut stats = filled();
        let person = |key: &str| tga_stats::Person {
            key: key.into(),
            name: key.into(),
            messages: 3,
            ..Default::default()
        };
        stats.people.rows = vec![person("a"), person("b")];
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains("<section id=\"trend\">"));
        assert!(!html.contains("How concentrated the talk was"));
        assert!(!html.contains("Who led each month"));
    }

    #[test]
    fn a_chat_without_polls_notes_or_changes_gets_none_of_those_sections() {
        let mut stats = filled();
        stats.polls = Some(tga_stats::Polls::default());
        stats.voice = Some(tga_stats::Voice::default());
        if let Some(history) = stats.history.as_mut() {
            // Joins only: Coming and going already charts those.
            history.events.clear();
            history.kinds = vec![tga_stats::Count::new("join_group_by_link", 40)];
        }
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(!html.contains("id=\"polls\"") && !html.contains("#polls"));
        assert!(!html.contains("id=\"history\"") && !html.contains("#history"));
        assert!(!html.contains("Voice and video notes"));
    }

    #[test]
    fn a_stored_light_theme_cannot_leave_the_surface_with_no_colours() {
        // The frozen script restores `tg-report-theme` from localStorage, and
        // an older report may well have written 'light' under that key. With no
        // `html.light` block and no switch, restoring it would render a page
        // with no colours and no way back. The surface script undoes it.
        let html = render(&bare(), &Names::new(), &Notes::default(), &options());
        assert!(html.contains("root.classList.remove('light')"));
        assert!(html.contains("localStorage.removeItem('tg-report-theme')"));
    }

    #[test]
    fn a_hostile_export_name_cannot_escape_into_markup() {
        // A report opens as a local file, so anything that survives as markup
        // runs with that origin. The name came from a group title.
        let mut stats = bare();
        stats.export.name = "<script>alert(1)</script>".into();
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn a_hostile_event_title_cannot_escape_either() {
        // The events file is written outside the program entirely, so it is
        // the least trusted input on the page.
        let event = tga_notes::Event {
            id: "\"><img src=x onerror=alert(1)>".into(),
            start: chrono::NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
            end: None,
            title: "<b>bold</b>".into(),
            summary: "<i>x</i>".into(),
            kind: "milestone".into(),
            topic: None,
            messages: vec![1],
            confidence: String::new(),
            time: String::new(),
            weight: String::new(),
            who: vec![],
            tags: vec![],
        };
        let mut stats = bare();
        stats.activity = dated();
        let notes = Notes {
            events: vec![event],
            source: "events.json".into(),
            ..Default::default()
        };
        let html = render(&stats, &Names::new(), &notes, &options());
        assert!(!html.contains("<img src=x"));
        assert!(!html.contains("<b>bold</b>"));
        assert!(html.contains("&lt;b&gt;bold&lt;/b&gt;"));
        assert!(html.contains("Events read from events.json."));
    }

    #[test]
    fn no_events_file_says_how_to_add_one_rather_than_saying_nothing() {
        let mut stats = bare();
        stats.activity = dated();
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains("No events file yet."));
        assert!(html.contains("analysis/digest.jsonl"));
    }

    #[test]
    fn an_export_with_dates_draws_the_hero_ribbon_and_its_axis() {
        let mut stats = bare();
        stats.activity = dated();
        let html = render(&stats, &Names::new(), &Notes::default(), &options());
        assert!(html.contains("class=\"chart ribbon hero\""));
        assert!(html.contains("class=\"taxis\""));
        assert!(html.contains("class=\"chart calendar\""));
        // The caption states the grain, which is the whole point of bucketing.
        assert!(html.contains("One bar per day; the tallest is 3 messages."));
    }

    #[test]
    fn names_are_taken_from_the_people_rows() {
        let mut stats = bare();
        stats.people.rows = vec![
            tga_stats::Person {
                key: "user1".into(),
                name: "Ana".into(),
                ..Default::default()
            },
            tga_stats::Person {
                key: "user2".into(),
                name: "Bob".into(),
                ..Default::default()
            },
        ];
        let names = names_from_stats(&stats);
        assert_eq!(names.get("user1").map(String::as_str), Some("Ana"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn no_fonts_leaves_the_document_without_a_single_font_face() {
        let html = render(
            &bare(),
            &Names::new(),
            &Notes::default(),
            &Options {
                embed_fonts: false,
                ..options()
            },
        );
        assert!(!html.contains("@font-face"));
        // ...and still names Geist in the stack, so an installed copy is used.
        assert!(html.contains("font-family:'Geist'"));
    }

    #[test]
    fn the_report_asks_nothing_of_the_network() {
        // The one property that makes this an archive rather than a page.
        let html = render(&bare(), &Names::new(), &Notes::default(), &options());
        for scheme in ["http://", "https://", "//cdn", "src=\"http"] {
            assert!(!html.contains(scheme), "the report reaches for {scheme}");
        }
    }

    #[test]
    fn the_open_disclosure_marker_is_a_minus_sign_and_nothing_else() {
        // It was a C1 control character followed by an ASCII `2` — what a minus
        // becomes after a bad encoding round trip — and a second rule further
        // down the stylesheet existed only to beat it. One rule now, and this
        // fails if either the defect or the override comes back.
        let html = render(&bare(), &Names::new(), &Notes::default(), &options());
        assert!(html.contains("details.data[open] summary::before{content:'\\2212 '}"));
        assert!(!html.contains('\u{91}'), "the C1 control character is back");
        assert_eq!(
            html.matches("details.data[open] summary::before").count(),
            1,
            "one rule, not a rule and its correction"
        );
    }
}
