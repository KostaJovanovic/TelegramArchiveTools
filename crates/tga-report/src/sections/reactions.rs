//! Which reactions the group uses, how that changed, and each person's own.

use tga_stats::{Count, Stats};

use super::*;
use crate::charts::{self, esc, thousands};

/// How many reactions the symbol list shows.
const KINDS_SHOWN: usize = 16;

/// `👍 12 · 😂 4`, the way a person's favourites are written in a cell.
fn favourites(items: &[Count]) -> String {
    if items.is_empty() {
        return "<span class=\"muted\">&#8212;</span>".to_string();
    }
    items
        .iter()
        .map(|c| {
            format!(
                "{}<span class=\"num muted\"> {}</span>",
                esc(&c.label),
                thousands(c.n)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn reactions(stats: &Stats) -> String {
    let Some(r) = &stats.reactions else {
        return String::new();
    };
    if r.total == 0 {
        return String::new();
    }

    let line = figures(&[
        (thousands(r.total), "reactions"),
        (share(r.reacted, r.messages), "of messages drew one"),
        (
            format!("{:.2}", r.total as f64 / r.messages.max(1) as f64),
            "per message",
        ),
        (thousands(r.kinds.len() as i64), "different reactions"),
    ]);

    let cells: String = r
        .kinds
        .iter()
        .take(KINDS_SHOWN)
        .map(|k| {
            format!(
                "<li><b>{}</b><span>{} &#183; {}</span></li>",
                esc(&k.label),
                thousands(k.n),
                share(k.n, r.total)
            )
        })
        .collect();
    let custom = if r.custom > 0 {
        format!(
            " Custom emoji are pooled as one entry: {} of them, whose pictures the \
             export names only by a number.",
            thousands(r.custom)
        )
    } else {
        String::new()
    };
    let kinds = panel(
        "Which reactions",
        &format!(
            "<b>{}</b> is {} of every reaction.",
            esc(&r.kinds[0].label),
            share(r.kinds[0].n, r.total)
        ),
        &format!("Every reaction Telegram counted, by kind.{custom}"),
        &format!("<ul class=\"figures symbols\">{cells}</ul>"),
    );

    let over_time = if r.months.len() >= 2 {
        let labels: Vec<String> = r
            .months
            .iter()
            .map(|m| m.chars().skip(2).collect())
            .collect();
        let values: Vec<i64> = r
            .reacted_per_month
            .iter()
            .zip(&r.messages_per_month)
            .map(|(reacted, all)| {
                if *all > 0 {
                    (*reacted as f64 / *all as f64 * 1000.0).round() as i64
                } else {
                    -1
                }
            })
            .collect();
        let shown = |n: i64| {
            if n < 0 {
                "no messages".to_string()
            } else {
                format!("{:.1}% reacted to", n as f64 / 10.0)
            }
        };
        panel(
            "Reacting, month by month",
            "",
            "The share of each month&#8217;s messages that drew at least one reaction.",
            &charts::vcols(&charts::Cols {
                labels: &labels,
                values: &values,
                shown: &shown,
                every: (r.months.len() / 14).max(1),
                id: "",
            }),
        )
    } else {
        String::new()
    };

    let rows: Vec<Vec<String>> = r
        .people
        .iter()
        .map(|p| {
            vec![
                short(&p.name, 26),
                thousands(p.messages),
                thousands(p.received),
                if p.messages > 0 {
                    format!("{:.2}", p.received as f64 / p.messages as f64)
                } else {
                    "&#8212;".to_string()
                },
                favourites(&p.received_top),
                thousands(p.given),
                favourites(&p.given_top),
            ]
        })
        .collect();
    let given_note = if r.named < r.total {
        format!(
            " What a person <i>gave</i> can only be counted where Telegram named the \
             reactor: {} of the {} reactions here. It names a few per message unless \
             the exporter asked for the full list, and never names an anonymous \
             reaction, so these are floors.",
            thousands(r.named),
            thousands(r.total)
        )
    } else {
        " Telegram named the reactor on every reaction here.".to_string()
    };
    let people = panel(
        "Each person\u{2019}s reactions",
        "",
        &format!(
            "Received is every reaction on the messages they sent, and the three \
             they drew most.{given_note}"
        ),
        &table(
            &[
                ("Person", false),
                ("Messages", true),
                ("Received", true),
                ("Per message", true),
                ("Draws most", false),
                ("Gave", true),
                ("Gives most", false),
            ],
            &rows,
        ),
    );

    format!(
        "{}{line}<div class=\"cols2\">{kinds}{over_time}</div>{people}</section>",
        head(
            "Reactions",
            &format!("{} reactions", thousands(r.total)),
            "reactions"
        )
    )
}
