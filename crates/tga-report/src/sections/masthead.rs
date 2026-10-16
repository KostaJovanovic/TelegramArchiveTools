//! The title block, the findings, and the bar that stays at the top.

use tga_stats::Stats;

use super::*;
use crate::charts::{esc, thousands};

/// `nav` is `(anchor, title)` for every section that rendered, in page order —
/// built by the caller from what it actually wrote, so a link never scrolls
/// to a section that is not there.
pub fn masthead(stats: &Stats, nav: &[(&str, &str)]) -> String {
    let act = &stats.activity;
    let info = &stats.export;
    let span = if is_empty_activity(act) {
        "no dated messages".to_string()
    } else {
        format!(
            "{} &ndash; {}",
            pretty_date(&act.first),
            pretty_date(&act.last)
        )
    };
    let toc: String = nav
        .iter()
        .map(|(anchor, title)| format!("<a href=\"#{anchor}\">{}</a>", esc(title)))
        .collect();
    let topics = info.topics as i64;
    let what = match duo(stats) {
        Some((a, b)) => format!(
            "A private chat between {} and {}: ",
            esc(&a.name),
            esc(&b.name)
        ),
        None => String::new(),
    };
    // The search box is a progressive enhancement by construction —
    // everything it filters is rendered server-side and visible before it is
    // typed in, so with scripting off the box does nothing and the page is
    // whole. `type="search"` rather than `text`, so the platform gives it a
    // clear control and the Escape key.
    //
    // The focus pill is empty and hidden until somebody picks a person; it
    // names who the page is drawing and is the way back out.
    let bar = format!(
        "<nav class=\"bar\" id=\"bar\" aria-label=\"Sections\"><div class=\"barin\">\
         <div class=\"toc\">{toc}</div>\
         <span class=\"focus\" id=\"focus\" hidden><span class=\"fname\"></span>\
         <button type=\"button\" aria-label=\"Stop focusing\">&#215;</button></span>\
         <input class=\"find\" id=\"find\" type=\"search\" autocomplete=\"off\" \
         placeholder=\"Search\" aria-label=\"Search people, topics, events\">\
         <span class=\"found\" id=\"found\" role=\"status\"></span></div></nav>"
    );
    format!(
        "<header class=\"masthead\">\
         <p class=\"eyebrow\">Telegram archive</p>\
         <h1>{}</h1>\
         <p class=\"lede\">{what}{} messages across {topics} topic{}, {span}.</p>\
         {}</header>{bar}",
        esc(&info.name),
        thousands(info.messages as i64),
        plural(topics, "", "s"),
        glance(stats),
    )
}
