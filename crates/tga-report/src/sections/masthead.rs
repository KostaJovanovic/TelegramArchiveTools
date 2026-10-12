//! The title block: what this archive is, and the way in.

use tga_stats::Stats;

use super::*;
use crate::charts::{esc, thousands};

pub fn masthead(stats: &Stats) -> String {
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
    let mut links = vec![
        ("whole", "Timeline"),
        ("rhythm", "Rhythm"),
        ("people", "People"),
        ("talk", "Conversation"),
        ("said", "What was said"),
        ("topics", "Topics"),
        ("records", "Records"),
        ("notes", "Notes"),
    ];
    // Inserted rather than appended, so the nav reads in page order. Gated on
    // the branch for the same reason the section is: a `--from-stats` dump
    // recorded before `dynamics` existed would otherwise get a nav entry that
    // scrolls nowhere.
    if stats.dynamics.is_some() {
        links.insert(4, ("between", "Between people"));
    }
    // Same gate as the section itself: with no member list *and* nobody silent,
    // `members` renders nothing, and a nav entry that scrolls nowhere is worse
    // than a missing one.
    let people = &stats.people;
    if people.known_members != 0 || people.silent_members != 0 {
        links.insert(3, ("members", "Members"));
    }
    let toc: String = links
        .iter()
        .map(|(anchor, title)| format!("<a href=\"#{anchor}\">{}</a>", esc(title)))
        .collect();
    let topics = info.topics as i64;
    // The search box is the one piece of `_timeline`'s data surface this report
    // did not already have. It is a progressive enhancement by construction —
    // everything it filters is rendered server-side and visible before it is
    // typed in, so with scripting off the box does nothing and the page is
    // whole. `type="search"` rather than `text`, so the platform gives it a
    // clear control and the Escape key.
    let find = "<input class=\"find\" id=\"find\" type=\"search\" autocomplete=\"off\" \
                placeholder=\"Search people, topics, events\" \
                aria-label=\"Search people, topics, events\">\
                <span class=\"found\" id=\"found\" role=\"status\"></span>";
    format!(
        "<header class=\"masthead\">\
         <p class=\"eyebrow\">Telegram archive</p>\
         <h1>{}</h1>\
         <p class=\"lede\">{} messages across {topics} topic{}, {span}.</p>\
         <nav class=\"toc\">{toc}</nav>\
         <div class=\"switches\">\
         <button class=\"switch\" id=\"aliases\" aria-pressed=\"false\">Former names</button>\
         <button class=\"switch\" id=\"everyone\" aria-pressed=\"false\">Everyone</button>\
         {find}</div></header>",
        esc(&info.name),
        thousands(info.messages as i64),
        plural(topics, "", "s"),
    )
}
