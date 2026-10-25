//! The sections of the report, and the small helpers they share.
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
/// How many people get a row before the rest go behind the switch: the same
/// number as every other long list ([`charts::CLIP`]). `report.js` carries it
/// too, as `SHOWN`, because the people table re-cuts itself when filtered.
const PEOPLE_SHOWN: usize = charts::CLIP;
/// The matrix is unreadable past this, and the network diagram says the rest.
const MATRIX_PEOPLE: usize = 16;

/// Peer key -> display name, for the two charts that label by key.
pub type Names = HashMap<String, String>;

mod between;
mod churn;
mod conversation;
mod glance;
mod history;
mod masthead;
mod notes;
mod people;
mod polls;
mod reactions;
mod records;
mod rhythm;
mod said;
mod timeline;
mod topics;
mod trend;
mod voice;
mod words;

pub use between::between;
pub use churn::churn;
pub use conversation::conversation;
pub use glance::glance;
pub use history::history;
pub use masthead::masthead;
pub use notes::notes;
pub use people::people;
pub use polls::polls;
pub use reactions::reactions;
pub use records::records;
pub use rhythm::rhythm;
pub use said::said;
pub use timeline::timeline;
pub use topics::topics;
pub use trend::trend;
pub use voice::voice;
pub use words::words;

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

/// `2025-09` -> `Sep 2025`, for a sentence. An axis keeps the short form.
///
/// In a take the bare `2025-09` broke at its hyphen on a phone, leaving the
/// month alone on the next line. Anything else comes back unchanged.
pub fn pretty_month(month: &str) -> String {
    match NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d") {
        Ok(day) => day.format("%b %Y").to_string(),
        Err(_) => month.to_string(),
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

/// A section rule: heading left, a count right, and the button that folds it.
///
/// `count` is markup, not text — it carries entities like `&#183;` — so every
/// caller escapes its own interpolations. `title` is escaped here.
///
/// The fold button does nothing without the script, and the section is open
/// without it: folding is a way to shorten a long page, never a way to hide
/// something from a reader whose browser runs no scripts.
pub(crate) fn head(title: &str, count: &str, anchor: &str) -> String {
    let tag = if anchor.is_empty() {
        String::new()
    } else {
        format!(" id=\"{}\"", esc(anchor))
    };
    format!(
        "<section{tag}><div class=\"sechead\"><h2>{}</h2><span class=\"count\">{count}</span>\
         <button class=\"fold\" type=\"button\" aria-expanded=\"true\" \
         aria-label=\"Fold this section\"></button></div>",
        esc(title)
    )
}

/// How something was measured, behind a (?) button.
///
/// **The method is one tap away, never in the way.** Every chart used to carry
/// its method as a paragraph under it, and the paragraphs competed with the
/// data for the reader's eye. A `<details>`, so it opens with no script; empty
/// text renders nothing, so a chart with nothing to explain gets no button.
pub(crate) fn how(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!(
        "<details class=\"how\"><summary title=\"How this is measured\" \
         aria-label=\"How this is measured\">?</summary><div class=\"howtext\">{text}</div></details>"
    )
}

/// One chart in its panel: a title, one plain sentence saying what it shows,
/// the method behind (?), and the chart.
///
/// `take` and `method` are markup; every caller escapes what it puts in them.
pub(crate) fn panel(title: &str, take: &str, method: &str, body: &str) -> String {
    let take = if take.is_empty() {
        String::new()
    } else {
        format!("<p class=\"take\">{take}</p>")
    };
    format!(
        "<div class=\"panel\"><h3>{}</h3>{take}{}{body}</div>",
        esc(title),
        how(method)
    )
}

/// A private chat: exactly two people said anything.
///
/// Such a chat gets its own layout — the two side by side — because the group
/// template's answers are meaningless for it: "the three loudest carry 100%",
/// a two-by-two matrix, a network of two dots.
pub fn is_duo(stats: &tga_stats::Stats) -> bool {
    stats.people.rows.iter().filter(|r| r.messages > 0).count() == 2
}

/// The two people of a private chat, louder first.
pub(crate) fn duo(stats: &tga_stats::Stats) -> Option<(&tga_stats::Person, &tga_stats::Person)> {
    let mut two: Vec<&tga_stats::Person> = stats
        .people
        .rows
        .iter()
        .filter(|r| r.messages > 0)
        .collect();
    if two.len() != 2 {
        return None;
    }
    two.sort_by_key(|p| std::cmp::Reverse(p.messages));
    Some((two[0], two[1]))
}

/// The key to a private chat's two colours, names escaped here.
pub(crate) fn duo_key(a: &str, b: &str) -> String {
    format!(
        "<p class=\"duokey\"><span class=\"one\">{}</span><span class=\"two\">{}</span></p>",
        esc(a),
        esc(b)
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
///
/// Every column sorts on a tap of its header (`data-sort`), by the number in
/// it when it is numeric. Past [`charts::CLIP`] rows the rest go behind Show
/// more; see [`table_all`] for the one kind of table that keeps every row.
fn table(headers: &[(&str, bool)], rows: &[Vec<String>]) -> String {
    let cut = rows.len().saturating_sub(charts::CLIP);
    table_rows(headers, rows, charts::CLIP) + &charts::more_button(cut)
}

/// A table that shows every row: the numbers behind a chart, in a
/// [`data_view`]. That is already folded away behind its own disclosure, and
/// its whole job is to be the complete figures.
fn table_all(headers: &[(&str, bool)], rows: &[Vec<String>]) -> String {
    table_rows(headers, rows, usize::MAX)
}

/// Whether a table becomes a stack of cards on a phone: four columns or
/// more, or any text column after the first. Two short columns — a tag and
/// its count — read better left as a table.
fn stacks(headers: &[(&str, bool)]) -> bool {
    headers.len() >= 4 || headers.iter().skip(1).any(|(_, numeric)| !numeric)
}

fn table_rows(headers: &[(&str, bool)], rows: &[Vec<String>], keep: usize) -> String {
    // A stacking table labels every cell with its column, which is what the
    // phone layout prints beside the value once the header row is a line of
    // sort buttons. The other tables keep their header and need no labels.
    let stack = stacks(headers);
    let label = |name: &str| {
        if stack {
            format!(" data-l=\"{}\"", esc(name))
        } else {
            String::new()
        }
    };
    let head: String = headers
        .iter()
        .map(|(name, numeric)| {
            if *numeric {
                format!("<th class=\"n\" data-sort=\"n\">{}</th>", esc(name))
            } else {
                format!("<th data-sort=\"t\">{}</th>", esc(name))
            }
        })
        .collect();
    let body: String = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let cells: String = row
                .iter()
                .zip(headers.iter())
                .map(|(cell, (name, numeric))| {
                    let l = label(name);
                    if *numeric {
                        format!("<td class=\"n\"{l}>{cell}</td>")
                    } else {
                        format!("<td{l}>{cell}</td>")
                    }
                })
                .collect();
            if index >= keep {
                format!("<tr class=\"cut\">{cells}</tr>")
            } else {
                format!("<tr>{cells}</tr>")
            }
        })
        .collect();
    let class = if stack { " class=\"stack\"" } else { "" };
    format!(
        "<div class=\"tablewrap\"><table{class}><thead><tr>{head}</tr></thead>\
         <tbody>{body}</tbody></table></div>"
    )
}

/// `2025-03-18 20:15`, or a bare date, the way every other date on the page
/// is written: `18 Mar 2025, 20:15`.
pub fn pretty_when(text: &str) -> String {
    let day = pretty_date(text);
    let time: String = text.chars().skip(11).take(5).collect();
    if day != text && time.len() == 5 && time.as_bytes()[2] == b':' {
        format!("{day}, {time}")
    } else {
        day
    }
}

/// The share of `part` in `whole`, as a percentage with no decimal past 10%.
fn share(part: i64, whole: i64) -> String {
    if whole <= 0 {
        return "0%".to_string();
    }
    let p = part as f64 / whole as f64 * 100.0;
    if p >= 10.0 {
        format!("{p:.0}%")
    } else {
        format!("{p:.1}%")
    }
}

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

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

/// How many people Focus can draw over the charts.
const FOCUS_PEOPLE: usize = 30;

/// What Focus draws: each of the loudest people's own bars on the hero's
/// grid, and their own hours.
///
/// **Data, not markup, and only for the few people it is useful for.** A
/// second ribbon per person drawn into the page would double the hero for
/// every row of the people table; as JSON it is a few hundred bytes a person,
/// and the script draws one at a time. Bucketed by [`charts::bucket_days`] on
/// the hero's own width, so bar `i` here is bar `i` there.
///
/// Every `<` is written as `<`, so a name cannot close the script tag it
/// sits in.
pub fn focus_data(stats: &tga_stats::Stats) -> String {
    let act = &stats.activity;
    if is_empty_activity(act) || is_duo(stats) {
        return String::new();
    }
    let mut speakers: Vec<&tga_stats::Person> = stats
        .people
        .rows
        .iter()
        .filter(|r| r.messages > 0)
        .collect();
    speakers.sort_by_key(|r| std::cmp::Reverse(r.messages));
    let mut people = serde_json::Map::new();
    for person in speakers.into_iter().take(FOCUS_PEOPLE) {
        let dense = crate::stats::densify(act.per_day_by_person.get(&person.key), &act.per_day);
        let (buckets, _) = charts::bucket_days(&dense, WIDTH);
        let bars: Vec<String> = buckets
            .iter()
            .enumerate()
            .filter(|(_, (_, _, n))| *n > 0)
            .map(|(i, (_, _, n))| format!("{i}:{n}"))
            .collect();
        people.insert(
            person.key.clone(),
            serde_json::json!({
                "name": person.name,
                "bars": bars.join(","),
                "hours": person.hours.to_vec(),
            }),
        );
    }
    let text = serde_json::Value::Object(people)
        .to_string()
        .replace('<', "\\u003c");
    format!("<script type=\"application/json\" id=\"focus-data\">{text}</script>")
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
    fn a_long_table_shows_seven_and_a_data_view_shows_everything() {
        let rows: Vec<Vec<String>> = (0..9).map(|i| vec![i.to_string()]).collect();
        let cut = table(&[("N", true)], &rows);
        assert_eq!(cut.matches("<tr class=\"cut\">").count(), 2);
        // The button follows the wrap, which is what the script opens.
        assert!(cut.ends_with("</table></div><button class=\"more\" type=\"button\" data-cut=\"7\">Show 2 more</button>"));
        let whole = table_all(&[("N", true)], &rows);
        assert!(!whole.contains("cut") && !whole.contains("<button"));
    }

    #[test]
    fn argmax_takes_the_first_maximum_rather_than_the_last() {
        assert_eq!(argmax(&[1, 9, 3, 9]), 1);
        assert_eq!(argmax(&[]), 0);
    }
}
