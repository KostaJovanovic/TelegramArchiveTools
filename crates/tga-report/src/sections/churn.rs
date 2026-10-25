//! Arrivals and departures, from the two sources that disagree.
//!
//! The heading is written by [`crate::render`] rather than here, because this
//! returns nothing at all for an export with no dated months and the
//! `<section>` still has to close.

use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

/// The inner markup of the churn section; [`crate::render`] wraps it.
pub fn churn(stats: &Stats) -> String {
    let churn = &stats.churn;
    let months = &churn.months;
    if months.is_empty() {
        return String::new();
    }
    let labels: Vec<String> = months.iter().map(|m| m.chars().skip(2).collect()).collect();
    let every = (months.len() / 14).max(1);

    let active_counts: Vec<i64> = churn.active.iter().map(|n| *n as i64).collect();
    let people = |n: i64| format!("{} people", thousands(n));
    let active = charts::vcols(&charts::Cols {
        labels: &labels,
        values: &active_counts,
        shown: &people,
        every,
        id: "",
    });
    let peak = argmax(&active_counts);
    let active_take = format!(
        "Most voices in <b>{}</b>: {} people posted.",
        esc(&pretty_month(&months[peak])),
        thousands(active_counts[peak])
    );

    let leaves: i64 = churn.announced_leaves.iter().sum();
    // No member list means no dated arrivals — a Telegram Desktop export never
    // has one, and neither does ours if the roster was not fetched. Drawing an
    // empty chart there says "nobody joined", which is a different claim from
    // "this export cannot tell you".
    let roster_joins = &churn.roster_joins;
    let dated: i64 = roster_joins.iter().sum();
    let joined = |n: i64| format!("{} joined", thousands(n));
    let joins = if dated != 0 {
        charts::vcols(&charts::Cols {
            labels: &labels,
            values: roster_joins,
            shown: &joined,
            every,
            id: "",
        })
    } else {
        String::new()
    };

    let note = if dated != 0 {
        format!(
            "Arrivals are dated from the member list, which knows only about \
             people who are still in the group: {} of {} current members carry a \
             join date. Anyone who has since left is missing from it entirely{}",
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
        // A member list can exist and still date nobody, and saying "no member
        // list" then contradicts the People section above it.
        let why = if churn.roster_size != 0 {
            "The member list carries no join dates"
        } else {
            "This export carries no member list"
        };
        format!(
            "{why}, so arrivals cannot be dated{}",
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

    let arrivals = if joins.is_empty() {
        // Nothing to draw, and the reason is still worth one tap.
        format!(
            "<div class=\"caption\">Arrivals cannot be dated. {}</div>",
            how(&note)
        )
    } else {
        let busiest = argmax(roster_joins);
        panel(
            "Arrivals",
            &format!(
                "{} of {} current members carry a join date; most joined in <b>{}</b>.",
                thousands(churn.roster_dated),
                thousands(churn.roster_size as i64),
                esc(&pretty_month(&months[busiest]))
            ),
            &note,
            &joins,
        )
    };

    panel(
        "People talking each month",
        &active_take,
        "Distinct people who posted at least once that month &#8212; not the \
         size of the group.",
        &active,
    ) + &arrivals
}
