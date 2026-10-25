//! Every poll the chat ran, with its tallies.

use tga_stats::{PollRow, Stats};

use super::*;
use crate::charts::{self, esc, thousands};

/// The answers as one cell, the winner in bold: `petak 6 · subota 3`.
fn answers(row: &PollRow) -> String {
    let top = row.answers.iter().map(|a| a.n).max().unwrap_or(0);
    row.answers
        .iter()
        .map(|a| {
            let text = esc(&a.label);
            let text = if top > 0 && a.n == top {
                format!("<b>{text}</b>")
            } else {
                text
            };
            format!("{text}<span class=\"num muted\"> {}</span>", thousands(a.n))
        })
        .collect::<Vec<_>>()
        .join(" &#183; ")
}

pub fn polls(stats: &Stats) -> String {
    let Some(p) = &stats.polls else {
        return String::new();
    };
    if p.total == 0 {
        return String::new();
    }

    let line = figures(&[
        (thousands(p.total as i64), "polls"),
        (thousands(p.votes), "votes cast"),
        (thousands(p.median_voters), "voters in a typical poll"),
        (thousands(p.closed as i64), "closed"),
    ]);

    let months = if p.per_month.len() >= 2 {
        let labels: Vec<String> = p
            .per_month
            .iter()
            .map(|c| c.label.chars().skip(2).collect())
            .collect();
        let values: Vec<i64> = p.per_month.iter().map(|c| c.n).collect();
        let shown = |n: i64| format!("{} poll{}", thousands(n), plural(n, "", "s"));
        let busiest = argmax(&values);
        panel(
            "Polls each month",
            &format!(
                "Most in <b>{}</b>: {}.",
                esc(&p.per_month[busiest].label),
                shown(values[busiest])
            ),
            "",
            &charts::vcols(&charts::Cols {
                labels: &labels,
                values: &values,
                shown: &shown,
                every: (labels.len() / 14).max(1),
                id: "",
            }),
        )
    } else {
        String::new()
    };
    let askers = if p.askers.len() >= 2 {
        let rows: Vec<charts::HBar> = p
            .askers
            .iter()
            .map(|a| {
                charts::HBar::new(a.name.clone(), a.count as f64, thousands(a.count))
                    .who(a.key.clone())
            })
            .collect();
        panel(
            "Who asks",
            "",
            "Polls each person put to the group. A poll forwarded in from somewhere \
             else was not asked by whoever forwarded it, and is left out.",
            &charts::hbars(&rows, Some(0)),
        )
    } else {
        String::new()
    };

    let rows: Vec<Vec<String>> = p
        .rows
        .iter()
        .rev()
        .map(|row| {
            let mut question = esc(&row.question);
            if row.forwarded {
                question.push_str(" <span class=\"muted\">(forwarded)</span>");
            }
            if row.closed {
                question.push_str(" <span class=\"muted\">(closed)</span>");
            }
            vec![
                esc(&pretty_when(&row.date)),
                short(&row.name, 24),
                question,
                answers(row),
                thousands(row.voters),
            ]
        })
        .collect();
    let cut = if p.shown < p.total {
        format!(
            " The newest {} of {} are listed.",
            thousands(p.shown as i64),
            thousands(p.total as i64)
        )
    } else {
        String::new()
    };
    let list = panel(
        "Every poll",
        "",
        &format!(
            "Newest first. Each answer with the number who chose it, the most \
             chosen in bold. The export records how many chose each answer and \
             nothing about who they were, so nothing here can say who voted for \
             what.{cut}"
        ),
        &table(
            &[
                ("When", false),
                ("Asked by", false),
                ("Question", false),
                ("Answers", false),
                ("Voters", true),
            ],
            &rows,
        ),
    );

    format!(
        "{}{line}<div class=\"cols2\">{months}{askers}</div>{list}</section>",
        head("Polls", &thousands(p.total as i64), "polls")
    )
}
