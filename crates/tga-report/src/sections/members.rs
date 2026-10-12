//! The member list, rendered — not just counted.
//!
//! `participants.json` is the one file beside an export that says who is *in*
//! the group as opposed to who happened to talk in it. Until this section
//! existed the report read it, took two numbers off it, and threw the rest
//! away: the handles, the bot flags, and the names of everybody who never
//! posted. A reader who wanted to know which of the 43 members had never said
//! a word had to go and open the JSON.
//!
//! **The three populations are different and the section keeps them apart.**
//!
//! - On the member list *and* in the history — the ordinary case.
//! - On the member list and never seen — lurkers, and the only place in the
//!   report they appear at all, because every other table filters on
//!   `messages != 0`.
//! - In the history and *not* on the member list — people who left before the
//!   export, or whom a capped roster missed. Their messages are still counted
//!   everywhere else, which is right; what is wrong is implying the list is a
//!   complete census when it demonstrably is not.
//!
//! The last of the three is why this cannot be a footnote on the people table.
//! A roster of 43 against 45 speakers looks like a rounding difference until
//! you notice only 41 names appear in both.

use std::fmt::Write as _;
use tga_stats::Stats;

use super::*;
use crate::charts::{esc, thousands};

/// Rows before the rest go behind the switch.
///
/// Higher than the people table's 30 on purpose: these rows carry no presence
/// strip, so they are cheap, and a member list truncated at 30 fails at the one
/// job this section has. A roster past this is long enough that the switch is
/// the kinder default.
const SHOWN: usize = 120;

pub fn members(stats: &Stats) -> String {
    let folk = &stats.people;
    // Nothing to say without a member list. The report already reports the
    // people who spoke; a section that only repeated them would be noise.
    if folk.known_members == 0 {
        return absent(stats);
    }

    // `listed`, not "has a handle" and not `silent`: a member may have set no
    // handle, and `silent` is only true for a member who never appears in the
    // history at all. Either proxy drops real members from the one table whose
    // job is to be complete.
    let mut listed: Vec<&tga_stats::Person> =
        stats.people.rows.iter().filter(|r| r.listed).collect();
    // Loudest first, then by name, so the ordering matches every other table
    // on the page rather than the order the JSON happened to be written in.
    listed.sort_by(|a, b| {
        b.messages
            .cmp(&a.messages)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let mut body = String::new();
    for (index, row) in listed.iter().enumerate() {
        let handle = if row.username.is_empty() {
            // Stated rather than left blank: an account with no handle cannot
            // be searched for or linked to, and that is a fact about them
            // rather than a gap in this table.
            "<span class=\"muted\">no handle</span>".to_string()
        } else {
            format!("<span class=\"handle\">@{}</span>", esc(&row.username))
        };
        let flags = {
            let mut marks = String::new();
            if row.bot {
                marks.push_str("<span class=\"tagpill\">bot</span>");
            }
            if row.role != "member" && !row.role.is_empty() {
                let _ = write!(marks, "<span class=\"role\">{}</span>", esc(&row.role));
            }
            marks
        };
        // "Never posted" is the whole point of the row, so it is a word rather
        // than a zero -- a `0` in a column of counts reads as a measurement
        // that came out low, not as an absence.
        let said = if row.messages == 0 {
            "<span class=\"muted\">never posted</span>".to_string()
        } else {
            thousands(row.messages)
        };
        let seen = match (&row.first, &row.last) {
            (Some(first), Some(last)) => format!(
                "{} &ndash; {}",
                esc(&pretty_date(first)),
                esc(&pretty_date(last))
            ),
            _ => String::new(),
        };
        let cls = if index >= SHOWN {
            " class=\"overflow\""
        } else {
            ""
        };
        let _ = write!(
            body,
            "<tr{cls}><td class=\"name\">{}{flags}</td><td>{handle}</td>\
             <td class=\"n\">{said}</td><td class=\"n\">{seen}</td>\
             <td class=\"n\">{}</td></tr>",
            esc(&row.name),
            match &row.joined {
                Some(when) => esc(&pretty_date(when)),
                None => String::new(),
            }
        );
    }

    let headers = "<tr><th>Name</th><th>Handle</th><th class=\"n\">Messages</th>\
                   <th class=\"n\">Seen</th><th class=\"n\">Joined</th></tr>";

    let bots = listed.iter().filter(|r| r.bot).count();
    let quiet = folk.known_members.saturating_sub(folk.members_who_spoke);
    let unlisted = folk.speakers.saturating_sub(folk.members_who_spoke);

    let mut counts = vec![
        (thousands(folk.known_members as i64), "on the member list"),
        (thousands(folk.members_who_spoke as i64), "of them posted"),
        (thousands(quiet as i64), "never posted"),
    ];
    if unlisted != 0 {
        counts.push((thousands(unlisted as i64), "posted but are not listed"));
    }
    if bots != 0 {
        counts.push((thousands(bots as i64), "are bots"));
    }

    let more = if listed.len() > SHOWN {
        format!(
            "<p class=\"note\">{} more {} on the list. Use \
             <span class=\"num\">Everyone</span> above to show them.</p>",
            listed.len() - SHOWN,
            if listed.len() - SHOWN == 1 {
                "person is"
            } else {
                "people are"
            }
        )
    } else {
        String::new()
    };

    format!(
        "{}<p class=\"lede\">{}</p>{}{}<table><thead>{headers}</thead>\
         <tbody>{body}</tbody></table>{more}{}</section>",
        head(
            "Members",
            &format!("{} listed", thousands(folk.known_members as i64)),
            "members"
        ),
        lede(folk),
        figures(&counts),
        caveats(folk, unlisted),
        handles_note(&listed)
    )
}

/// The sentence at the top, which has to be true of all four shapes this takes.
fn lede(folk: &tga_stats::People) -> String {
    let quiet = folk.known_members.saturating_sub(folk.members_who_spoke);
    if quiet == 0 {
        return "Everybody on the member list posted at least once.".to_string();
    }
    format!(
        "{} of the {} people on the member list never posted, so this is the \
         only table on the page they appear in.",
        thousands(quiet as i64),
        thousands(folk.known_members as i64)
    )
}

/// What the member list is *not*, when it is not a complete census.
///
/// Two flags rather than one: `complete: false` is the exporter saying it knows
/// it missed some, and `capped: true` is it saying it stopped at a limit. They
/// call for different amounts of doubt and the report can only pass that on if
/// it keeps them apart.
fn caveats(folk: &tga_stats::People, unlisted: usize) -> String {
    let mut notes = String::new();
    if folk.roster_complete == Some(false) {
        notes.push_str(
            "<p class=\"note\">The export says this member list is incomplete, \
             so the count of people who never posted is a floor rather than a \
             total.</p>",
        );
    }
    if folk.roster_capped == Some(true) {
        notes.push_str(
            "<p class=\"note\">The export says it stopped collecting members \
             before it ran out of them, so the list is the first however-many \
             rather than all of them.</p>",
        );
    }
    if unlisted != 0 {
        let _ = write!(
            notes,
            "<p class=\"note\">{} {} in this archive {} on the member list at \
             all &#8212; people who left before the export was taken, or whom a \
             short list missed. Everything they said is still counted \
             everywhere else on this page.</p>",
            thousands(unlisted as i64),
            if unlisted == 1 { "person" } else { "people" },
            if unlisted == 1 { "is not" } else { "are not" }
        );
    }
    notes
}

/// How many of the listed have a handle, which decides whether they can be
/// looked up at all.
fn handles_note(listed: &[&tga_stats::Person]) -> String {
    let without = listed.iter().filter(|r| r.username.is_empty()).count();
    if without == 0 {
        return String::new();
    }
    format!(
        "<p class=\"caption\">{} of the {} have no <span class=\"num\">@handle</span>. \
         A display name is whatever its owner last set it to; a handle is the only \
         part of a person here that stays put, so a row without one can be \
         recognised today and not next month.</p>",
        thousands(without as i64),
        thousands(listed.len() as i64)
    )
}

/// What to say when the export carried no member list at all.
///
/// **This is the branch that used to lie.** With no `participants.json` the
/// people section still printed "N of the 0 people on the member list never
/// posted", which is both arithmetically absurd and wrong about where those
/// people came from: they are reactors and joiners picked out of the history,
/// not roster entries. Both real archives hit this branch.
fn absent(stats: &Stats) -> String {
    let folk = &stats.people;
    // Everybody the history mentions who never posted a word. Not `silent`,
    // which is only ever set on a roster row -- with no roster these people are
    // reactors and joiners, and they have names.
    let quiet: Vec<&tga_stats::Person> = folk.rows.iter().filter(|r| r.messages == 0).collect();
    if quiet.is_empty() {
        return String::new();
    }
    // Named rather than counted. The people table filters them out, so without
    // this the report knows 25 names it never says -- and "25 people" is a fact
    // you cannot act on, where a list is one you can go and check.
    let names: String = quiet
        .iter()
        .map(|r| {
            format!(
                "<li>{}</li>",
                if r.name.is_empty() {
                    "<span class=\"muted\">unnamed account</span>".to_string()
                } else {
                    esc(&r.name)
                }
            )
        })
        .collect();
    let n = quiet.len();
    format!(
        "{}<p class=\"lede\">This export carries no member list, so there is no \
         roll to compare the conversation against.</p>\
         <p class=\"note\">{} {} below {} in this archive without ever posting: \
         they reacted to something, or a join or a rename named them. That is a \
         floor and not a count of lurkers &#8212; somebody who read every \
         message and never touched a reaction leaves no trace at all, and \
         Telegram names at most three reactors per message.</p>\
         <ul class=\"namelist\">{names}</ul>\
         <p class=\"caption\">Drop a <span class=\"num\">participants.json</span> \
         beside the export to turn this into a real member list, with handles, \
         roles and everybody who never spoke &#8212; including the ones who \
         never even reacted, who leave no trace here at all.</p></section>",
        head(
            "Members",
            &format!("{} never posted", thousands(n as i64)),
            "members"
        ),
        thousands(n as i64),
        if n == 1 { "person" } else { "people" },
        if n == 1 { "appears" } else { "appear" }
    )
}
