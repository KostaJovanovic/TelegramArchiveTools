//! The eleven sections of the report, and the small helpers they share.
//!
//! **The structure is one shared time axis.** The archive's whole life is drawn
//! once, full width, at the top; every topic and every person below is drawn as
//! a smaller ribbon on *that same axis with that same ceiling*. Nothing is
//! rescaled to fill its own row, because the moment a row is rescaled you can
//! no longer read two rows against each other — and reading them against each
//! other is the entire question. It costs the quiet rows their detail. That is
//! the trade, and it is stated in the notes rather than hidden.
//!
//! Every `&str` that came from the export goes through [`esc`] on the way in.
//! A report opens as a local file, so anything that survives as markup runs
//! with that origin, and every name, filename, emoji and link in here came from
//! strangers on the internet. The same applies to the events file, which is
//! written outside the program entirely.

use std::collections::HashMap;

use chrono::NaiveDate;
use tga_stats::Activity;

use crate::charts::{self, esc, Day, Quantiles};

/// The viewBox width every chart on the page is drawn in.
pub const WIDTH: f64 = 1120.0;
/// How many people get a row before the rest go behind the switch.
const PEOPLE_SHOWN: usize = 30;
/// The matrix is unreadable past this, and the network diagram says the rest.
const MATRIX_PEOPLE: usize = 16;

/// Peer key -> display name, for the two charts that label by key.
pub type Names = HashMap<String, String>;

mod between;
mod churn;
mod conversation;
mod masthead;
mod members;
mod notes;
mod people;
mod records;
mod rhythm;
mod said;
mod timeline;
mod topics;

pub use between::between;
pub use churn::churn;
pub use conversation::conversation;
pub use masthead::masthead;
pub use members::members;
pub use notes::notes;
pub use people::people;
pub use records::records;
pub use rhythm::rhythm;
pub use said::said;
pub use timeline::timeline;
pub use topics::topics;

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

/// `2025-09-05` or `2025-09-05T14:30:00` -> `5 Sep 2025`.
///
/// Anything it cannot parse comes back unchanged. The alternative — raising —
/// would cost the whole report over one malformed timestamp in one row.
pub fn pretty_date(iso: &str) -> String {
    let head: String = iso.chars().take(10).collect();
    match NaiveDate::parse_from_str(&head, "%Y-%m-%d") {
        Ok(day) => {
            let text = day.format("%d %b %Y").to_string();
            text.trim_start_matches('0').to_string()
        }
        Err(_) => iso.to_string(),
    }
}

/// Seconds, in the largest unit that keeps the number readable.
pub fn duration(seconds: f64) -> String {
    let seconds = seconds.trunc() as i64;
    if seconds < 90 {
        return format!("{seconds} s");
    }
    if seconds < 90 * 60 {
        return format!("{:.0} min", seconds as f64 / 60.0);
    }
    if seconds < 36 * 3600 {
        return format!("{:.1} h", seconds as f64 / 3600.0);
    }
    format!("{:.1} days", seconds as f64 / 86400.0)
}

/// Trim a name for a narrow table cell, full string on the title.
///
/// A 36-character display name in a 90px column wraps to five lines and triples
/// the height of the row it is in.
fn short(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return esc(text);
    }
    let head: String = text.chars().take(limit - 1).collect();
    format!("<span title=\"{}\">{}&#8230;</span>", esc(text), esc(&head))
}

fn pct(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

/// A section rule: micro-heading left, a count right.
///
/// `count` is markup, not text — it carries entities like `&#183;` — so every
/// caller escapes its own interpolations. `title` is escaped here.
fn head(title: &str, count: &str, anchor: &str) -> String {
    let tag = if anchor.is_empty() {
        String::new()
    } else {
        format!(" id=\"{}\"", esc(anchor))
    };
    format!(
        "<section{tag}><div class=\"sechead\"><h2>{}</h2><span class=\"count\">{count}</span></div>",
        esc(title)
    )
}

/// `(value markup, label)` — the value is markup so a caller can put an `<em>`
/// or an entity in it; the label is escaped here.
fn figures(items: &[(String, &str)]) -> String {
    let cells: String = items
        .iter()
        .map(|(value, label)| format!("<li><b>{value}</b><span>{}</span></li>", esc(label)))
        .collect();
    format!("<ul class=\"figures\">{cells}</ul>")
}

/// `(name, numeric)` per column; cells are markup and escape their own input.
fn table(headers: &[(&str, bool)], rows: &[Vec<String>]) -> String {
    let head: String = headers
        .iter()
        .map(|(name, numeric)| {
            if *numeric {
                format!("<th class=\"n\">{}</th>", esc(name))
            } else {
                format!("<th>{}</th>", esc(name))
            }
        })
        .collect();
    let body: String = rows
        .iter()
        .map(|row| {
            let cells: String = row
                .iter()
                .zip(headers.iter())
                .map(|(cell, (_, numeric))| {
                    if *numeric {
                        format!("<td class=\"n\">{cell}</td>")
                    } else {
                        format!("<td>{cell}</td>")
                    }
                })
                .collect();
            format!("<tr>{cells}</tr>")
        })
        .collect();
    format!("<table class=\"\"><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table>")
}

/// Every chart's honest twin: the numbers, as text, one click away.
fn data_view(label: &str, table: &str) -> String {
    format!(
        "<details class=\"data\"><summary>{}</summary>{table}</details>",
        esc(label)
    )
}

fn plural(count: i64, singular: &str, plural: &str) -> &'static str {
    // Returns one of two `'static` words rather than a formatted string,
    // because every call site is inside a `format!` already.
    let _ = (singular, plural);
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// An archive with no dated messages in it.
///
/// Checked rather than inferred from an empty series, because the two are
/// different facts: a dump recorded from an export that had nothing in it says
/// so, and the sections that cannot draw anything say so back.
fn is_empty_activity(activity: &Activity) -> bool {
    activity.empty
}

// ---------------------------------------------------------------------------
// shared marks
// ---------------------------------------------------------------------------

/// The index of the first maximum.
fn argmax(values: &[i64]) -> usize {
    let top = values.iter().max().copied().unwrap_or(0);
    values.iter().position(|v| *v == top).unwrap_or(0)
}

/// The display name for a peer key.
///
/// Falls back to the key itself, which is what `People::get` hands back for a
/// key it does not know. A key on screen is ugly and it is *true*; a blank cell
/// silently drops a person out of the chart.
fn label_for(names: &Names, key: &str) -> String {
    names.get(key).cloned().unwrap_or_else(|| {
        if key.is_empty() {
            "unknown".to_string()
        } else {
            key.to_string()
        }
    })
}

/// One presence row: a person's or a topic's activity on the shared axis.
///
/// See [`charts::strip_compact`] for what the compact form gives up and why it
/// is the whole size budget — these rows are 64% of a large report drawn the
/// obvious way.
pub const PRESENCE_WIDTH: f64 = WIDTH * 0.34;
const PRESENCE_HEIGHT: f64 = 15.0;

fn presence(dense: &[Day], scale: &Quantiles) -> String {
    charts::strip_compact(dense, PRESENCE_WIDTH, PRESENCE_HEIGHT, scale, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_loses_its_leading_zero_and_keeps_its_month_name() {
        assert_eq!(pretty_date("2025-09-05"), "5 Sep 2025");
        assert_eq!(pretty_date("2025-12-14"), "14 Dec 2025");
        // The people table stores `first`/`last` as full timestamps.
        assert_eq!(pretty_date("2025-09-05T14:30:00"), "5 Sep 2025");
    }

    #[test]
    fn an_unparseable_date_comes_back_unchanged_rather_than_failing() {
        // A malformed timestamp in one row must not cost the whole report.
        assert_eq!(pretty_date(""), "");
        assert_eq!(pretty_date("soon"), "soon");
    }

    #[test]
    fn a_duration_climbs_units_at_the_documented_boundaries() {
        assert_eq!(duration(0.0), "0 s");
        assert_eq!(duration(89.0), "89 s");
        assert_eq!(duration(90.0), "2 min");
        assert_eq!(duration(5399.0), "90 min");
        assert_eq!(duration(5400.0), "1.5 h");
        assert_eq!(duration(129_600.0), "1.5 days");
    }

    #[test]
    fn a_short_name_is_left_whole_and_a_long_one_keeps_its_title() {
        assert_eq!(short("Ana", 22), "Ana");
        let long = short("a name far longer than the column", 22);
        assert!(long.starts_with("<span title="));
        assert!(long.ends_with("&#8230;</span>"));
    }

    #[test]
    fn an_unknown_key_labels_as_itself_rather_than_as_a_blank_cell() {
        let names = Names::new();
        assert_eq!(label_for(&names, "user123"), "user123");
        assert_eq!(label_for(&names, ""), "unknown");
    }

    #[test]
    fn argmax_takes_the_first_maximum_rather_than_the_last() {
        assert_eq!(argmax(&[1, 9, 3, 9]), 1);
        assert_eq!(argmax(&[]), 0);
    }
}
