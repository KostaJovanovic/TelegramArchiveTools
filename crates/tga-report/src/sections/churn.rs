//! Arrivals and departures, from the two sources that disagree.
//!
//! The heading is written by [`crate::render`] rather than here, because this
//! returns nothing at all for an export with no dated months and the
//! `<section>` still has to close.

use tga_stats::Stats;

use super::*;
use crate::charts::{self, thousands};

/// The inner markup of the churn section; [`crate::render`] wraps it.
pub fn churn(stats: &Stats) -> String {
    let churn = &stats.churn;
    let months = &churn.months;
    if months.is_empty() {
        return String::new();
    }
    let labels: Vec<String> = months.iter().map(|m| m.chars().skip(2).collect()).collect();
    let every = (months.len() / 14).max(1);

    let active = charts::columns(
        &labels,
        &churn.active.iter().map(|n| *n as i64).collect::<Vec<_>>(),
        WIDTH,
        200.0,
        every,
        " people",
        None,
    );
    let leaves: i64 = churn.announced_leaves.iter().sum();
    // No member list means no dated arrivals — a Telegram Desktop export never
    // has one, and neither does ours if the roster was not fetched. Drawing an
    // empty chart there says "nobody joined", which is a different claim from
    // "this export cannot tell you".
    let roster_joins = &churn.roster_joins;
    let dated: i64 = roster_joins.iter().sum();
    let joins = if dated != 0 {
        charts::columns(&labels, roster_joins, WIDTH, 170.0, every, " joined", None)
    } else {
        String::new()
    };

    let note = if dated != 0 {
        format!(
            "<p class=\"caption\">Arrivals are dated from the member list, which \
             knows only about people who are still in the group: {} of {} current \
             members carry a join date. Anyone who has since left is missing from \
             it entirely{}</p>",
            thousands(churn.roster_dated),
            thousands(churn.roster_size as i64),
            if leaves != 0 {
                format!(
                    ", though the history announces {} removals.",
                    thousands(leaves)
                )
            } else {
                ".".to_string()
            }
        )
    } else {
        let announced: i64 = churn.announced_joins.iter().sum();
        format!(
            "<p class=\"caption\">This export carries no member list, so arrivals \
             cannot be dated{}</p>",
            if leaves != 0 || announced != 0 {
                format!(
                    ". The history does announce {} joins and {} removals, which is \
                     only the ones Telegram posted about.",
                    thousands(announced),
                    thousands(leaves)
                )
            } else {
                ", and the history announces none.".to_string()
            }
        )
    };

    format!(
        "<h3>People talking each month</h3>{active}\
         <p class=\"caption\">Distinct people who posted at least once that month \
         &#8212; not the size of the group.</p>{}{joins}{note}",
        if joins.is_empty() {
            ""
        } else {
            "<h3>Arrivals</h3>"
        }
    )
}
