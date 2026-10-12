//! The extremes, each one citing the message it rests on.

use std::fmt::Write as _;
use tga_stats::Figure;
use tga_stats::Stats;

use super::*;
use crate::charts::{esc, thousands};

pub fn records(stats: &Stats) -> String {
    let items = &stats.superlatives;
    if items.is_empty() {
        return String::new();
    }
    let mut body = String::new();
    for item in items {
        // A superlative's value is a count for most records and a formatted
        // string for one — the largest file, which is already `1.8 MB`. The
        // count gets its group separators; the text is escaped as it stands.
        let value = match &item.value {
            Figure::Count(n) => thousands(*n),
            Figure::Text(text) => esc(text),
        };
        let unit = if item.unit.is_empty() {
            String::new()
        } else {
            format!(" <span class=\"num\">{}</span>", esc(&item.unit))
        };
        let mut about = esc(&item.date);
        if !item.who.is_empty() {
            let _ = write!(about, " &#183; {}", esc(&item.who));
        }
        let detail = if item.text.is_empty() {
            String::new()
        } else {
            format!("<br><span>{}</span>", esc(&item.text))
        };
        let _ = write!(
            body,
            "<li><span class=\"what\">{}</span><span class=\"big\">{value}</span>\
             <span class=\"about\">{about}{unit}{detail}</span></li>",
            esc(&item.title)
        );
    }
    format!(
        "{}<ul class=\"records\">{body}</ul></section>",
        head("Records", &items.len().to_string(), "records")
    )
}
