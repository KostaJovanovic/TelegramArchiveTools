//! The archive on one axis, with the hand-written notes pinned to it.
//!
//! This is the section every other one is drawn against: the whole span,
//! full width, at one ceiling. Everything below it on the page reuses that
//! axis, so the same horizontal position is the same week wherever it is.

use std::fmt::Write as _;
use tga_notes::{Event, Notes};
use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

pub fn timeline(stats: &Stats, notes: &Notes, names: &Names) -> String {
    let events = &notes.events[..];
    let events_note = notes.source.as_str();
    let act = &stats.activity;
    if is_empty_activity(act) {
        return head("Timeline", "", "whole") + "<p>No dated messages.</p></section>";
    }
    let series = &act.per_day;
    let (buckets, per) = charts::bucket_days(series, WIDTH);
    let top = buckets.iter().map(|(_, _, c)| *c).max().unwrap_or(1);

    let kinds: Vec<String> = notes.kinds();
    let rail = charts::rail(events, series, WIDTH, 34.0, &kinds);
    let ribbon = charts::ribbon(series, WIDTH, 132.0, Some(top), true, "hero");
    let axis = charts::time_axis(series, WIDTH, 16.0);

    let grain = if per == 1 {
        "day".to_string()
    } else {
        format!("{per}-day block")
    };
    let caption = format!(
        "<p class=\"caption\">One bar per {grain}; the tallest is {} messages. \
         Every row further down this page runs left to right over this same span, \
         so the same horizontal position is the same week wherever you find it.</p>",
        thousands(top)
    );

    let counts = &stats.people;
    let talk = &stats.conversation;
    let content = &stats.content;
    let stat_line = figures(&[
        (thousands(act.span_days as i64), "days spanned"),
        (thousands(act.active_days as i64), "days with messages"),
        (thousands(counts.speakers as i64), "people spoke"),
        (thousands(content.media_messages), "attachments"),
        (thousands(counts.votes_total), "reactions"),
        (thousands(talk.replies), "replies"),
    ]);

    let (listing, note) = if events.is_empty() {
        (
            String::new(),
            "<p class=\"note\">No events file yet. The ribbon shows how much was \
             said; it cannot show what any of it meant. Run the analyser with \
             <span class=\"num\">Write the digest</span> switched on, hand the \
             resulting <span class=\"num\">analysis/digest.jsonl</span> and \
             <span class=\"num\">analysis/EVENTS.md</span> to a model or a \
             person, and drop their <span class=\"num\">events.json</span> beside \
             the export. It will be pinned to this axis on the next run.</p>"
                .to_string(),
        )
    } else {
        let mut cards = String::new();
        for event in events {
            let mut when = pretty_date(&event.start.to_string());
            if event.spans() {
                if let Some(end) = event.end {
                    let _ = write!(when, " &ndash; {}", pretty_date(&end.to_string()));
                }
            }
            let cites = if event.messages.is_empty() {
                "<p class=\"cite\">no messages cited</p>".to_string()
            } else {
                let count = event.messages.len();
                let ids: Vec<String> = event
                    .messages
                    .iter()
                    .take(8)
                    .map(|m| m.to_string())
                    .collect();
                format!(
                    "<p class=\"cite\">{count} message{} cited &#183; {}{}</p>",
                    plural(count as i64, "", "s"),
                    esc(&ids.join(", ")),
                    if count > 8 { "&#8230;" } else { "" }
                )
            };
            let kind = if event.kind.is_empty() {
                String::new()
            } else {
                format!("<span class=\"kind\">{}</span> &#183; ", esc(&event.kind))
            };
            let conf = if event.confidence.is_empty() {
                String::new()
            } else {
                format!(" &#183; {} confidence", esc(&event.confidence))
            };
            let summary = {
                let escaped = esc(&event.summary);
                if escaped.is_empty() {
                    "No summary.".to_string()
                } else {
                    escaped
                }
            };
            // Everything below is a field only the `_timeline` notes layout
            // carries, so none of it appears unless the file actually said it.
            let shape = kinds.iter().position(|k| *k == event.kind).unwrap_or(0);
            if !event.time.is_empty() {
                let _ = write!(when, " <span class=\"at\">{}</span>", esc(&event.time));
            }
            let weight = if event.weight.is_empty() {
                String::new()
            } else {
                format!(
                    " <span class=\"w w-{}\">{}</span>",
                    esc(&event.weight),
                    esc(&event.weight)
                )
            };
            let tags = if event.tags.is_empty() {
                String::new()
            } else {
                format!(
                    "<p class=\"tags\">{}</p>",
                    event
                        .tags
                        .iter()
                        .map(|t| format!("<span>{}</span>", esc(t)))
                        .collect::<Vec<_>>()
                        .join("")
                )
            };
            let _ = write!(
                cards,
                "<li class=\"k{shape}\" id=\"event-{}\" data-kind=\"{}\"><time>{when}</time>\
                 <div><h4>{}{weight}</h4><p>{kind}{summary}{conf}</p>{}{tags}{cites}</div></li>",
                esc(&event.id),
                esc(&event.kind),
                esc(&event.title),
                who_line(&event.who, names)
            );
        }
        (
            format!(
                "{}<ul class=\"events\">{cards}</ul>",
                kind_filters(&kinds, events)
            ),
            format!(
                "<p class=\"note\">Events read from {}.</p>",
                esc(events_note)
            ),
        )
    };

    let months: Vec<Vec<String>> = act
        .per_month
        .iter()
        .map(|month| vec![esc(&month.label), thousands(month.n)])
        .collect();

    let coverage = coverage_panel(notes.coverage.as_ref(), series);

    format!(
        "{}{rail}{coverage}{ribbon}{axis}{caption}{stat_line}{note}{listing}{}</section>",
        head(
            "Timeline",
            &format!("{} days", thousands(act.span_days as i64)),
            "whole"
        ),
        data_view(
            "Messages per month",
            &table(&[("Month", false), ("Messages", true)], &months)
        )
    )
}

/// What the notes claim to have read, drawn against the whole archive.
///
/// Renders nothing when the file states no coverage — an absent claim and a
/// claim of nothing are different, and inventing "covers everything" from
/// silence is exactly the reading this block exists to prevent.
fn coverage_panel(cover: Option<&tga_notes::Coverage>, series: &[Day]) -> String {
    let Some(cover) = cover else {
        return String::new();
    };
    let band = charts::coverage_band(cover.from, cover.to, series, WIDTH, 6.0);

    let span = match (cover.from, cover.to) {
        (Some(from), Some(to)) => format!(
            "{} &ndash; {}",
            pretty_date(&from.to_string()),
            pretty_date(&to.to_string())
        ),
        (Some(from), None) => format!("from {}", pretty_date(&from.to_string())),
        (None, Some(to)) => format!("up to {}", pretty_date(&to.to_string())),
        (None, None) => String::new(),
    };

    // The share of the archive's days the claim covers, because "one month of
    // eleven" is the fact and "5 Sep – 5 Oct" only implies it.
    let share = match (cover.from, cover.to, series.first(), series.last()) {
        (Some(from), Some(to), Some(first), Some(last)) => {
            let whole = (iso_day(&last.label) - iso_day(&first.label))
                .num_days()
                .max(1);
            let read = (to - from).num_days().max(0) + 1;
            format!(
                " &#183; {} of {} days",
                thousands(read.min(whole + 1)),
                thousands(whole + 1)
            )
        }
        _ => String::new(),
    };

    let level = if cover.level.is_empty() {
        String::new()
    } else {
        format!(" &#183; {}", esc(&cover.level))
    };
    let note = if cover.note.is_empty() {
        String::new()
    } else {
        format!("<p class=\"caption\">{}</p>", esc(&cover.note))
    };

    format!("{band}<p class=\"coverage\"><b>Read</b> {span}{share}{level}</p>{note}")
}

fn iso_day(text: &str) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap_or_default()
}

/// One toggle per kind actually present, shape-coded to match the rail.
///
/// Built from the kinds the *file* used rather than from a fixed vocabulary, so
/// a file using three of four gets three controls and a file using a word
/// nobody anticipated still gets one. The glyph is the same shape the marker
/// draws, and the word is beside it — nothing rests on telling ● from ■ alone.
fn kind_filters(kinds: &[String], events: &[Event]) -> String {
    if kinds.len() < 2 {
        // One kind is not a choice, and a filter row that can only be all-on or
        // all-off is a control that does nothing useful.
        return String::new();
    }
    const GLYPHS: [&str; 4] = ["\u{25cf}", "\u{25a0}", "\u{25b2}", "\u{25c6}"];
    let cells: String = kinds
        .iter()
        .enumerate()
        .map(|(index, kind)| {
            let count = events.iter().filter(|e| e.kind == *kind).count();
            format!(
                "<button class=\"kind-filter on\" data-k=\"{index}\" aria-pressed=\"true\">\
                 <i class=\"g{}\">{}</i>{} <span>{count}</span></button>",
                index % GLYPHS.len(),
                GLYPHS[index % GLYPHS.len()],
                esc(kind)
            )
        })
        .collect();
    format!("<div class=\"kinds\">{cells}</div>")
}

/// The names an event credits, each checked against the statistics.
///
/// **A `who` that names nobody says so.** PLAN.md asks for this by name, and
/// the reason is practical rather than pedantic: a name in the notes that
/// matches no one in the export almost always means the export spells it
/// differently — somebody renamed themselves, or the writer used the name they
/// know them by. That is a thing the reader can go and fix, and it is invisible
/// unless the report points at it. Reporting it must not break the panel, so an
/// unmatched name is still shown, just marked.
fn who_line(who: &[String], names: &Names) -> String {
    if who.is_empty() {
        return String::new();
    }
    // Case- and space-insensitive, because the notes are typed by hand and the
    // export's spelling is whatever somebody set as their display name.
    let known: std::collections::HashSet<String> = names
        .values()
        .map(|n| {
            n.to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let mut missing = 0usize;
    let cells: String = who
        .iter()
        .map(|name| {
            let key = name
                .to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if known.contains(&key) {
                format!("<span>{}</span>", esc(name))
            } else {
                missing += 1;
                format!("<span class=\"unknown\">{}</span>", esc(name))
            }
        })
        .collect();
    let note = if missing == 0 {
        String::new()
    } else {
        format!(
            " <em>{missing} name{} match{} nobody in this export</em>",
            plural(missing as i64, "", "s"),
            if missing == 1 { "es" } else { "" }
        )
    };
    format!("<p class=\"who\">{cells}{note}</p>")
}
