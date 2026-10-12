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
//! The visual language is Swiss/International: hairlines do the dividing,
//! corners are square, numbers are set in Geist Mono, and section headings are
//! letterspaced uppercase micro-type. There are no cards.
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

    let body = format!(
        "{}{}{}{}{}{}{}{}{}{}</section>{}{}{}",
        sections::masthead(stats),
        sections::timeline(stats, notes, names),
        sections::rhythm(stats),
        sections::people(stats),
        // Straight after the people table, because it answers the question that
        // table provokes: these are the ones who talked, so who else is here?
        sections::members(stats),
        sections::conversation(stats, names),
        sections::between(stats),
        sections::said(stats),
        // The churn section's heading is written here rather than inside
        // `churn`, because that function returns nothing at all for an export
        // with no dated months and the `<section>` still has to close.
        section_head("Coming and going", "churn"),
        sections::churn(stats),
        sections::topics(stats),
        sections::records(stats),
        sections::notes(stats, &notes.source, &options.source, &options.stamp),
    );

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

fn section_head(title: &str, anchor: &str) -> String {
    format!(
        "<section id=\"{}\"><div class=\"sechead\"><h2>{}</h2><span class=\"count\"></span></div>",
        esc(anchor),
        esc(title)
    )
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
        assert!(html.contains("<td class=\"n\">177</td>"));
        assert!(html.contains("30 Jun 2025, the last day in this archive"));
        // Escaped like every other name that came out of an export.
        assert!(!html.contains("Bob & Co <the second>"));
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
        assert!(html.contains("class=\"chart axis-strip\""));
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
