//! Everybody in the chat, in one table: who posted, who is on the member list
//! and never did, and who posted without being on it.
//!
//! **One table, three populations, kept apart by a filter rather than by
//! sections.** They used to be two sections — People for whoever posted,
//! Members for the roster — and the same person appeared in both with
//! different columns. Merged, every row is one account; the chips above it
//! pick a population and the counts on the chips say how big each one is.
//!
//! - **Posted**: the ordinary case, ranked by messages.
//! - **Never posted**: on the member list and silent, or (with no member list)
//!   seen only reacting or joining. The only place in the report they appear,
//!   because every chart counts messages.
//! - **Left or not on the list**: posted, but the member list does not name
//!   them — people who left before the export, or whom a capped list missed.
//!   Their messages are counted everywhere else, which is right; what would be
//!   wrong is implying the list is a complete census when it is not.
//!
//! A private chat gets a side-by-side panel of the two instead: a ranked table
//! of two rows answers nothing.

use crate::stats::{densify, every_count, number};
use std::fmt::Write as _;
use tga_stats::Stats;

use super::*;
use crate::charts::{esc, thousands};

pub fn people(stats: &Stats) -> String {
    let folk = &stats.people;
    if folk.rows.is_empty() {
        return String::new();
    }
    if let Some((a, b)) = duo(stats) {
        return pair(stats, a, b);
    }
    let act = &stats.activity;
    let roster = folk.known_members != 0;

    // Who posted, loudest first; then everyone who never did, by name.
    let mut posted: Vec<&tga_stats::Person> =
        folk.rows.iter().filter(|r| r.messages != 0).collect();
    posted.sort_by_key(|r| std::cmp::Reverse(r.messages));
    let mut quiet: Vec<&tga_stats::Person> = folk.rows.iter().filter(|r| r.messages == 0).collect();
    quiet.sort_by_key(|r| r.name.to_lowercase());
    let unlisted_count = if roster {
        posted.iter().filter(|r| !r.listed).count()
    } else {
        0
    };

    let series = &act.per_day;
    let per_person = &act.per_day_by_person;
    // One scale for every row, built from every person-day in the archive.
    // Shared, so a shade means the same thing in row 1 and row 30.
    let scale = Quantiles::new(every_count(per_person));
    // A member list without join dates gets no Joined column. A column empty
    // in every row reads as a rendering fault, not as "Telegram did not say".
    let any_joined = folk.rows.iter().any(|r| r.joined.is_some());

    let mut body = String::new();
    let everyone = posted.iter().chain(quiet.iter());
    for (index, row) in everyone.enumerate() {
        let spoke = row.messages != 0;
        let mut classes = vec![if spoke { "posted" } else { "silent" }];
        if roster && spoke && !row.listed {
            classes.push("unlisted");
        }
        if index >= PEOPLE_SHOWN {
            classes.push("overflow");
        }
        let mini = if spoke && !series.is_empty() {
            presence(&densify(per_person.get(&row.key), series), &scale)
        } else {
            String::new()
        };
        let alias = if row.aliases.is_empty() {
            String::new()
        } else {
            format!(
                " <span class=\"alias\">was {}</span>",
                esc(&row.aliases.join(", "))
            )
        };
        let mut flags = String::new();
        if row.bot {
            flags.push_str("<span class=\"tagpill\">bot</span>");
        }
        if !row.role.is_empty() && row.role != "member" {
            let _ = write!(flags, " <span class=\"role\">{}</span>", esc(&row.role));
        }
        // The handle under the name, because it is the part of a person that
        // stays put; skipped when the name already *is* the handle, which is
        // what the handles-only setting writes.
        let handle = if row.username.is_empty() || row.name == format!("@{}", row.username) {
            String::new()
        } else {
            format!("<span class=\"handle\">@{}</span>", esc(&row.username))
        };
        let rank = if spoke {
            (index + 1).to_string()
        } else {
            String::new()
        };
        // "Never posted" is the whole point of such a row, so it is a word
        // rather than a zero.
        let said = if spoke {
            thousands(row.messages)
        } else {
            "<span class=\"muted\">never posted</span>".to_string()
        };
        let date_cell = |label: &str, when: Option<&str>| {
            let when = when.unwrap_or_default();
            format!(
                "<td class=\"n date\" data-l=\"{label}\" data-v=\"{}\">{}</td>",
                esc(when),
                esc(&pretty_date(when))
            )
        };
        let joined = if any_joined {
            date_cell("Joined", row.joined.as_deref())
        } else {
            String::new()
        };
        let _ = write!(
            body,
            "<tr class=\"{}\" data-who=\"{}\"><td class=\"rank k\">{rank}</td>\
             <td class=\"name k\"><span class=\"nm\">{}</span>{flags}{alias}{handle}</td>\
             <td class=\"presence\">{mini}</td>\
             <td class=\"n k\" data-l=\"Messages\" data-v=\"{}\">{said}</td>\
             <td class=\"n\" data-l=\"Share\" data-v=\"{}\">{}</td>\
             <td class=\"n\" data-l=\"Words/msg\" data-v=\"{}\">{}</td>\
             <td class=\"n\" data-l=\"Reactions\" data-v=\"{}\">{}</td>\
             {}{}{joined}</tr>",
            classes.join(" "),
            esc(&row.key),
            esc(&row.name),
            row.messages,
            row.share,
            if spoke { pct(row.share) } else { String::new() },
            row.avg_words,
            if spoke {
                number(row.avg_words)
            } else {
                String::new()
            },
            row.reactions_received,
            thousands(row.reactions_received),
            date_cell("First seen", row.first.as_deref()),
            date_cell("Last seen", row.last.as_deref()),
        );
    }

    let headers = format!(
        "<tr><th class=\"rank\"></th><th data-sort=\"t\">Name</th>\
         <th class=\"presence\">Presence</th>\
         <th class=\"n\" data-sort=\"n\">Messages</th><th class=\"n\" data-sort=\"n\">Share</th>\
         <th class=\"n\" data-sort=\"n\">Words/msg</th><th class=\"n\" data-sort=\"n\">Reactions</th>\
         <th class=\"n\" data-sort=\"t\">First seen</th><th class=\"n\" data-sort=\"t\">Last seen</th>{}</tr>",
        if any_joined {
            "<th class=\"n\" data-sort=\"t\">Joined</th>"
        } else {
            ""
        }
    );

    let total = posted.len() + quiet.len();
    let chip = |filter: &str, label: &str, n: usize, on: bool| {
        format!(
            "<button class=\"chip{}\" type=\"button\" data-f=\"{filter}\" aria-pressed=\"{on}\">\
             {label} <span>{}</span></button>",
            if on { " on" } else { "" },
            thousands(n as i64)
        )
    };
    let mut chips = chip("all", "All", total, true);
    chips.push_str(&chip("posted", "Posted", posted.len(), false));
    if !quiet.is_empty() {
        chips.push_str(&chip("silent", "Never posted", quiet.len(), false));
    }
    if unlisted_count != 0 {
        chips.push_str(&chip(
            "unlisted",
            "Left or not on the list",
            unlisted_count,
            false,
        ));
    }
    let renamed = folk.rows.iter().any(|r| !r.aliases.is_empty());
    let aliases = if renamed {
        "<button class=\"chip switch\" id=\"aliases\" type=\"button\" aria-pressed=\"false\">\
         Former names</button>"
    } else {
        ""
    };
    let more = if total > PEOPLE_SHOWN {
        format!(
            "<button class=\"more\" id=\"people-more\" type=\"button\" data-n=\"{total}\">\
             Show all {}</button>",
            thousands(total as i64)
        )
    } else {
        String::new()
    };

    let take = format!(
        "The three loudest carry <b>{}</b> of everything said{}.",
        pct(folk.top3_share),
        if roster && folk.known_members > folk.members_who_spoke {
            format!(
                "; {} of {} members never posted",
                thousands(folk.known_members.saturating_sub(folk.members_who_spoke) as i64),
                thousands(folk.known_members as i64)
            )
        } else {
            String::new()
        }
    );

    let method = format!(
        "Presence runs on the same time axis as the bars at the top of the page, \
         and its shading is on one scale shared by every row. {} \
         Reactions given are a floor, not a total: Telegram names at most three \
         reactors per message and never names an anonymous one. {} of {} \
         reactions in this archive have a name on them.{}{}",
        esc(&scale.caption("messages in a day")),
        thousands(folk.votes_named),
        thousands(folk.votes_total),
        roster_notes(folk, unlisted_count),
        handles_note(&folk.rows),
    );

    let listing = format!(
        "<div class=\"ptools\">{chips}{aliases}</div>\
         <div class=\"tablewrap\"><table class=\"rows people-table\"><thead>{headers}</thead>\
         <tbody>{body}</tbody></table></div>{more}"
    );

    let figures_line = if roster {
        let bots = folk.rows.iter().filter(|r| r.listed && r.bot).count();
        let mut counts = vec![
            (thousands(folk.speakers as i64), "posted"),
            (thousands(folk.known_members as i64), "on the member list"),
            (
                thousands(folk.known_members.saturating_sub(folk.members_who_spoke) as i64),
                "of them never posted",
            ),
        ];
        if unlisted_count != 0 {
            counts.push((
                thousands(unlisted_count as i64),
                "posted but are not listed",
            ));
        }
        if bots != 0 {
            counts.push((thousands(bots as i64), "are bots"));
        }
        figures(&counts)
    } else {
        String::new()
    };

    let streak = match &stats.streak {
        None => String::new(),
        Some(run) => format!(
            "<p class=\"note\">Longest unbroken run of days posted on: <b>{}</b>, {} days to {}.</p>",
            esc(&run.name),
            run.days,
            esc(&pretty_date(&run.ended))
        ),
    };

    let awards = if stats.awards.is_empty() {
        String::new()
    } else {
        let cells: String = stats
            .awards
            .iter()
            .map(|a| {
                format!(
                    "<li data-who=\"{}\"><b>{}</b><span>{}: {}</span></li>",
                    esc(&a.key),
                    esc(&a.value),
                    esc(&a.title),
                    esc(&a.name)
                )
            })
            .collect();
        // The floor the metrics used, not a copy of it. A dump recorded
        // before the floor was carried says so without a number.
        let floor = match stats.award_minimum {
            Some(n) => format!("with {} messages or more", thousands(n)),
            None => "who posted often enough to mean it".to_string(),
        };
        panel(
            "Who does what",
            "",
            &format!(
                "Restricted to people {floor}; below that every one of these is \
                 won by somebody who posted four times, all of them at 4am."
            ),
            &format!("<ul class=\"figures\">{cells}</ul>"),
        )
    };

    let mut count = format!("{} spoke", folk.speakers);
    if roster {
        let _ = write!(count, " &#183; {} on the member list", folk.known_members);
    }

    format!(
        "{}{figures_line}{}{streak}{awards}</section>",
        head("People", &count, "people"),
        panel("Everyone in this chat", &take, &method, &listing),
    )
}

/// What the member list is *not*, when it is not a complete census.
///
/// Two flags rather than one: `complete: false` is the exporter saying it knows
/// it missed some, and `capped: true` is it saying it stopped at a limit. They
/// call for different amounts of doubt and the report can only pass that on if
/// it keeps them apart.
fn roster_notes(folk: &tga_stats::People, unlisted: usize) -> String {
    let mut notes = String::new();
    if folk.known_members == 0 {
        // **This is the branch that used to lie.** With no member list the
        // report once printed "N of the 0 people on the member list never
        // posted"; the quiet rows here are reactors and joiners picked out of
        // the history, not roster entries.
        notes.push_str(
            " This export carries no member list. Everyone under Never posted \
             appears in the history without posting: they reacted to something, \
             or a join or a rename named them. That is a floor and not a count of \
             lurkers &#8212; somebody who read every message and never reacted \
             leaves no trace at all. Drop a <span class=\"num\">participants.json\
             </span> beside the export to turn this into a real member list.",
        );
        return notes;
    }
    if folk.roster_complete == Some(false) {
        notes.push_str(
            " The export says this member list is incomplete, so the count of \
             people who never posted is a floor rather than a total.",
        );
    }
    if folk.roster_capped == Some(true) {
        notes.push_str(
            " The export says it stopped collecting members before it ran out of \
             them, so the list is the first however-many rather than all of them.",
        );
    }
    if unlisted != 0 {
        let _ = write!(
            notes,
            " {} {} in this archive {} on the member list at all &#8212; people \
             who left before the export was taken, or whom a short list missed. \
             Everything they said is still counted everywhere else on this page.",
            thousands(unlisted as i64),
            if unlisted == 1 { "person" } else { "people" },
            if unlisted == 1 { "is not" } else { "are not" }
        );
    }
    notes
}

/// How many of the listed have a handle, which decides whether they can be
/// looked up at all.
fn handles_note(rows: &[tga_stats::Person]) -> String {
    let listed: Vec<&tga_stats::Person> = rows.iter().filter(|r| r.listed).collect();
    let without = listed.iter().filter(|r| r.username.is_empty()).count();
    if without == 0 {
        return String::new();
    }
    format!(
        " {} of the {} on the member list have no <span class=\"num\">@handle</span>. \
         A display name is whatever its owner last set it to; a handle is the only \
         part of a person here that stays put.",
        thousands(without as i64),
        thousands(listed.len() as i64)
    )
}

/// The two people of a private chat, side by side.
fn pair(stats: &Stats, a: &tga_stats::Person, b: &tga_stats::Person) -> String {
    let talk = &stats.conversation;
    let median = |key: &str| {
        talk.fastest
            .iter()
            .chain(talk.slowest.iter())
            .find(|l| l.key == key)
            .map(|l| l.median)
    };
    let started = |key: &str| {
        talk.starters
            .iter()
            .find(|s| s.key == key)
            .map_or(0, |s| s.count)
    };
    let answered = |from: &str, to: &str| {
        talk.edges
            .iter()
            .filter(|e| e.from == from && e.to == to)
            .map(|e| e.count)
            .sum::<i64>()
    };

    // `(label, a, b, shown a, shown b)`; a row whose values are both zero
    // says nothing and is left out.
    let mut rows: Vec<(&str, f64, f64, String, String)> = Vec::new();
    let mut count = |label: &'static str, x: i64, y: i64| {
        if x != 0 || y != 0 {
            rows.push((label, x as f64, y as f64, thousands(x), thousands(y)));
        }
    };
    count("Messages", a.messages, b.messages);
    count("Words", a.words, b.words);
    count("Attachments", a.media, b.media);
    count("Replies", a.replies, b.replies);
    count(
        "Reactions received",
        a.reactions_received,
        b.reactions_received,
    );
    count("Conversations started", started(&a.key), started(&b.key));
    count(
        "Answers to the other",
        answered(&a.key, &b.key),
        answered(&b.key, &a.key),
    );
    if a.avg_words > 0.0 || b.avg_words > 0.0 {
        rows.push((
            "Words per message",
            a.avg_words,
            b.avg_words,
            number(a.avg_words),
            number(b.avg_words),
        ));
    }
    if let (Some(x), Some(y)) = (median(&a.key), median(&b.key)) {
        rows.push((
            "Typical reply time",
            x as f64,
            y as f64,
            duration(x as f64),
            duration(y as f64),
        ));
    }

    let mut body = String::new();
    for (label, x, y, sx, sy) in &rows {
        let whole = x + y;
        let left = if whole > 0.0 { x / whole * 100.0 } else { 50.0 };
        let _ = write!(
            body,
            "<tr><th scope=\"row\">{}</th><td class=\"n one\">{}</td>\
             <td class=\"vsbar\" aria-hidden=\"true\"><i class=\"one\" style=\"width:{}%\"></i>\
             <i class=\"two\" style=\"width:{}%\"></i></td><td class=\"n two\">{}</td></tr>",
            esc(label),
            esc(sx),
            crate::charts::num(left),
            crate::charts::num(100.0 - left),
            esc(sy)
        );
    }
    let emoji = |key: &str| -> String {
        stats
            .content
            .emoji_by_person
            .get(key)
            .map(|list| {
                list.iter()
                    .take(5)
                    .map(|c| esc(&c.label))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    };
    let (ea, eb) = (emoji(&a.key), emoji(&b.key));
    if !ea.is_empty() || !eb.is_empty() {
        let _ = write!(
            body,
            "<tr><th scope=\"row\">Favourite emoji</th><td class=\"one emo\">{ea}</td>\
             <td></td><td class=\"two emo\">{eb}</td></tr>"
        );
    }
    let seen = |p: &tga_stats::Person| esc(&pretty_date(p.first.as_deref().unwrap_or_default()));
    let _ = write!(
        body,
        "<tr><th scope=\"row\">First message</th><td class=\"n one\">{}</td><td></td>\
         <td class=\"n two\">{}</td></tr>",
        seen(a),
        seen(b)
    );

    let handle = |p: &tga_stats::Person| {
        if p.username.is_empty() || p.name == format!("@{}", p.username) {
            String::new()
        } else {
            format!("<span class=\"handle\">@{}</span>", esc(&p.username))
        }
    };
    let table = format!(
        "<div class=\"tablewrap\"><table class=\"vs\"><thead><tr><th></th>\
         <th class=\"n one\" data-who=\"{}\">{}{}</th><th></th>\
         <th class=\"n two\" data-who=\"{}\">{}{}</th></tr></thead>\
         <tbody>{body}</tbody></table></div>",
        esc(&a.key),
        esc(&a.name),
        handle(a),
        esc(&b.key),
        esc(&b.name),
        handle(b)
    );
    let total = a.messages + b.messages;
    let take = format!(
        "<b>{}</b> writes {} of the messages; <b>{}</b> writes {}.",
        esc(&a.name),
        share(a.messages, total),
        esc(&b.name),
        share(b.messages, total)
    );
    format!(
        "{}{}</section>",
        head("The two of you", "side by side", "people"),
        panel(
            "Side by side",
            &take,
            "Every figure is one person's own, from the same counts as the rest \
             of the page. The bar between the two numbers is how they split; a \
             reply time is the median gap before that person answered the other. \
             Answers to the other count replies only, not every message sent \
             after the other's.",
            &table
        )
    )
}
