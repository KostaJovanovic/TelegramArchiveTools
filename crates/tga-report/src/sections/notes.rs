//! How to read the page: every rule that changes what a number means.

use tga_stats::Stats;

use super::*;
use crate::charts::{esc, thousands};

pub fn notes(stats: &Stats, events_note: &str, source: &str, stamp: &str) -> String {
    let talk = &stats.conversation;
    let folk = &stats.people;
    let mut lines: Vec<(&str, String)> = vec![
        (
            "Time",
            "Every clock face and calendar in this report reads the local \
                  timestamp the export recorded. Durations are measured on epoch \
                  seconds instead, so a gap that crosses a daylight-saving change \
                  is still the length it was."
                .to_string(),
        ),
        (
            "Identity",
            "One person is one Telegram account id, not one display name. \
                      Somebody who renamed themselves is a single row, shown under \
                      the newest name they used; <span class=\"num\">Former \
                      names</span> at the top of the page reveals the rest."
                .to_string(),
        ),
        (
            "Replies",
            format!(
                "A forum topic is itself a thread, so Telegram marks every \
             top-level message in one as a reply to the message that \
             opened the topic. Those are not answers to anybody and are \
             not counted. Replies to your own message are counted as \
             replies ({} of them) but are left out of who-answers-whom.",
                thousands(talk.self_replies)
            ),
        ),
        (
            "Reactions",
            format!(
                "Telegram names at most three reactors per message, and \
             never names anyone who reacted anonymously. Reaction \
             totals are exact; who gave them is a floor ({} of {} are attributed).",
                thousands(folk.votes_named),
                thousands(folk.votes_total)
            ),
        ),
        (
            "Bursts",
            format!(
                "A gap longer than {} minutes ends one and starts the next, \
             counted separately per topic.",
                talk.session_gap / 60
            ),
        ),
        (
            "Scale",
            "Everything with a time axis on this page runs left to right \
                   over the same span, so a mark at the same horizontal \
                   position is the same week wherever you find it. The ribbon \
                   at the top is drawn as height, because the archive is one \
                   series and height reads as quantity. The rows in the people \
                   and topic tables are shaded instead, on one scale shared by \
                   every row: at forty rows, height would draw everyone below \
                   the top few as a scatter of single pixels."
                .to_string(),
        ),
        (
            "Colour",
            "One hue, five steps, generated in OKLab and checked for \
                    even lightness steps and contrast against both backgrounds. \
                    Nothing on this page encodes a category by colour, so none \
                    of it depends on telling two hues apart."
                .to_string(),
        ),
    ];
    if !events_note.is_empty() {
        lines.insert(
            0,
            (
                "Events",
                format!(
                    "The marked events came from {}. They are somebody's \
                     reading of the archive, not a measurement of it, and each \
                     one lists the messages it rests on.",
                    esc(events_note)
                ),
            ),
        );
    }
    let rows: String = lines
        .iter()
        .map(|(title, body)| {
            format!(
                "<li><span class=\"what\">{}</span><span class=\"about\">{body}</span></li>",
                esc(title)
            )
        })
        .collect();
    format!(
        "{}<ul class=\"records\">{rows}</ul>\
         <p class=\"note\">Written by {} on {stamp} from \
         <span class=\"num\">{}</span>. No part of this report contacted Telegram; \
         it is derived entirely from the export on disk.</p></section>",
        head("Notes", "how to read this", "notes"),
        esc(source),
        esc(&stats.export.root)
    )
}
