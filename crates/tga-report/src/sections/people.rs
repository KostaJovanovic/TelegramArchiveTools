//! One row per person, ranked, each with their presence on the shared axis.

use crate::stats::{densify, every_count, number};
use std::fmt::Write as _;
use tga_stats::Stats;

use super::*;
use crate::charts::{esc, thousands};

pub fn people(stats: &Stats) -> String {
    let folk = &stats.people;
    let act = &stats.activity;
    let rows: Vec<&tga_stats::Person> = folk.rows.iter().filter(|r| r.messages != 0).collect();
    if rows.is_empty() {
        return String::new();
    }

    let series = &act.per_day;
    let per_person = &act.per_day_by_person;
    // One scale for every row, built from every person-day in the archive.
    // Shared, so a shade means the same thing in row 1 and row 30.
    let scale = Quantiles::new(every_count(per_person));

    let mut body = String::new();
    for (index, row) in rows.iter().enumerate() {
        let dense = densify(per_person.get(&row.key), series);
        let mini = presence(&dense, &scale);
        let alias = if row.aliases.is_empty() {
            String::new()
        } else {
            format!(
                " <span class=\"alias\">was {}</span>",
                esc(&row.aliases.join(", "))
            )
        };
        let role = if row.role.is_empty() || row.role == "member" {
            String::new()
        } else {
            format!(" <span class=\"role\">{}</span>", esc(&row.role))
        };
        let cls = if index >= PEOPLE_SHOWN {
            " class=\"overflow\""
        } else {
            ""
        };
        let _ = write!(
            body,
            "<tr{cls}><td class=\"rank\">{}</td>\
             <td class=\"name\">{}{role}{alias}</td>\
             <td class=\"presence\">{mini}</td>\
             <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td>\
             <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
            index + 1,
            esc(&row.name),
            thousands(row.messages),
            pct(row.share),
            number(row.avg_words),
            thousands(row.reactions_received),
            esc(&pretty_date(row.first.as_deref().unwrap_or_default())),
            esc(&pretty_date(row.last.as_deref().unwrap_or_default())),
        );
    }

    let headers = "<tr><th></th><th>Name</th><th>Presence</th>\
                   <th class=\"n\">Messages</th><th class=\"n\">Share</th>\
                   <th class=\"n\">Words/msg</th><th class=\"n\">Reactions</th>\
                   <th class=\"n\">First seen</th><th class=\"n\">Last seen</th></tr>";
    let listing = format!("<table><thead>{headers}</thead><tbody>{body}</tbody></table>");

    let hidden = rows.len().saturating_sub(PEOPLE_SHOWN);
    let more = if hidden == 0 {
        String::new()
    } else {
        format!(
            "<p class=\"note\">{hidden} more {} sent at least one message. \
             Use <span class=\"num\">Everyone</span> above to show them.</p>",
            if hidden == 1 { "person" } else { "people" }
        )
    };

    // Who never posted, and how short the member list is, both belong to the
    // Members section below -- which is where the roster is actually rendered.
    // This used to say "N of the 0 people on the member list never posted" on
    // any export without a `participants.json`, which is both arithmetic
    // nonsense and wrong about where those people came from: with no roster
    // they are reactors and joiners picked out of the history. Both real
    // archives hit that branch.
    let silent = if folk.silent_members == 0 {
        String::new()
    } else {
        format!(
            "<p class=\"note\">{} more {} in this archive without ever posting, \
             so they are not in the table above &#8212; see \
             <a href=\"#members\">Members</a>.</p>",
            folk.silent_members,
            if folk.silent_members == 1 {
                "person appears"
            } else {
                "people appear"
            }
        )
    };

    let awards = if stats.awards.is_empty() {
        String::new()
    } else {
        let cells: String = stats
            .awards
            .iter()
            .map(|a| {
                format!(
                    "<li><b>{}</b><span>{}: {}</span></li>",
                    esc(&a.value),
                    esc(&a.title),
                    esc(&a.name)
                )
            })
            .collect();
        format!(
            "<h3>Who does what</h3><ul class=\"figures\">{cells}</ul>\
             <p class=\"caption\">Restricted to people with 40 messages or more; \
             below that every one of these is won by somebody who posted four \
             times, all of them at 4am.</p>"
        )
    };

    let streak = match &stats.streak {
        None => String::new(),
        Some(run) => format!(
            "<p class=\"note\">Longest unbroken run of days posted on: {}, {} days to {}.</p>",
            esc(&run.name),
            run.days,
            esc(&pretty_date(&run.ended))
        ),
    };

    let votes = format!(
        "<p class=\"note\">Reactions given are a floor, not a total: Telegram \
         names at most three reactors per message and never names an anonymous \
         one. {} of {} reactions in this archive have a name on them.</p>",
        thousands(folk.votes_named),
        thousands(folk.votes_total)
    );

    let known = folk.known_members;
    let count = format!("{} spoke", folk.speakers)
        + &if known != 0 {
            format!(" &#183; {known} on the member list")
        } else {
            String::new()
        };

    format!(
        "{}<p class=\"lede\">The three loudest carry {} of everything said.</p>\
         <p class=\"caption\">Presence runs on the same time axis as the ribbon \
         at the top of the page, and its shading is on one scale shared by \
         every row. {}</p>{listing}{more}{silent}{streak}{awards}{votes}</section>",
        head("People", &count, "people"),
        pct(folk.top3_share),
        esc(&scale.caption("messages in a day"))
    )
}
