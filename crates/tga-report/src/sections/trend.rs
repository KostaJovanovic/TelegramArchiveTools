//! The archive's headline figures again, one month at a time.

use tga_stats::{Stats, Trends};

use super::*;
use crate::charts::{self, esc, thousands};

/// A ratio is drawn in tenths of a percent, so the columns keep a decimal's
/// worth of shape on an integer axis.
fn permille(share: f64) -> i64 {
    (share * 1000.0).round() as i64
}

/// One month-by-month panel: columns, and a sentence naming the high and low.
///
/// `values` is one per month; `None` is a month the figure could not be read
/// for, drawn as an empty slot and left out of the high and the low.
fn monthly(
    title: &str,
    method: &str,
    months: &[String],
    values: &[Option<i64>],
    shown: &dyn Fn(i64) -> String,
) -> String {
    let read: Vec<(usize, i64)> = values
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|v| (i, v)))
        .collect();
    if read.len() < 2 {
        return String::new();
    }
    let labels: Vec<String> = months.iter().map(|m| m.chars().skip(2).collect()).collect();
    // An unread month is drawn as -1: `vcols` gives anything at or under zero
    // no height, and the tooltip can then tell it from a month that read 0.
    let drawn: Vec<i64> = values.iter().map(|v| v.unwrap_or(-1)).collect();
    let tip = |n: i64| -> String {
        if n < 0 {
            "no reading".to_string()
        } else {
            shown(n)
        }
    };
    let chart = charts::vcols(&charts::Cols {
        labels: &labels,
        values: &drawn,
        shown: &tip,
        every: (months.len() / 14).max(1),
        id: "",
    });
    // The first maximum and the first minimum.
    let (hi, hi_v) = read
        .iter()
        .copied()
        .fold(read[0], |best, x| if x.1 > best.1 { x } else { best });
    let (lo, lo_v) = read
        .iter()
        .copied()
        .fold(read[0], |best, x| if x.1 < best.1 { x } else { best });
    let take = format!(
        "Highest in <b>{}</b> at {}, lowest in <b>{}</b> at {}.",
        esc(&months[hi]),
        esc(&shown(hi_v)),
        esc(&months[lo]),
        esc(&shown(lo_v))
    );
    panel(title, &take, method, &chart)
}

fn leaders(trends: &Trends) -> String {
    let rows: Vec<Vec<String>> = trends
        .months
        .iter()
        .enumerate()
        .map(|(i, month)| {
            let (who, part) = match trends.leaders.get(i).and_then(Option::as_ref) {
                Some(lead) => (short(&lead.name, 28), pct(lead.share)),
                None => (
                    "<span class=\"muted\">nobody</span>".to_string(),
                    String::new(),
                ),
            };
            vec![
                esc(month),
                thousands(trends.messages.get(i).copied().unwrap_or(0)),
                thousands(trends.voices.get(i).copied().unwrap_or(0)),
                who,
                part,
            ]
        })
        .collect();
    let changes = trends
        .leaders
        .windows(2)
        .filter(|pair| match (&pair[0], &pair[1]) {
            (Some(a), Some(b)) => a.key != b.key,
            _ => false,
        })
        .count();
    let distinct: std::collections::HashSet<&str> = trends
        .leaders
        .iter()
        .flatten()
        .map(|lead| lead.key.as_str())
        .collect();
    panel(
        "Who led each month",
        &format!(
            "{} different people were the loudest in some month; the lead changed hands {} time{}.",
            thousands(distinct.len() as i64),
            thousands(changes as i64),
            plural(changes as i64, "", "s")
        ),
        "The person who sent the most messages that month, and their share of it.",
        &table(
            &[
                ("Month", false),
                ("Messages", true),
                ("Voices", true),
                ("Led by", false),
                ("Their share", true),
            ],
            &rows,
        ),
    )
}

pub fn trend(stats: &Stats) -> String {
    let Some(trends) = &stats.trends else {
        return String::new();
    };
    let months = &trends.months;
    if months.len() < 2 {
        return String::new();
    }
    let duo = is_duo(stats);
    let percent = |n: i64| format!("{:.1}%", n as f64 / 10.0);
    let some = |values: &[f64], scale: &dyn Fn(f64) -> i64| -> Vec<Option<i64>> {
        values
            .iter()
            .zip(&trends.messages)
            .map(|(v, n)| (*n > 0).then(|| scale(*v)))
            .collect()
    };

    let mut panels: Vec<String> = Vec::new();
    if !duo {
        panels.push(monthly(
            "How concentrated the talk was",
            "The loudest three people&#8217;s share of that month&#8217;s messages \
             &#8212; the figure People gives once for the whole archive. High is a \
             few voices carrying the group; low is many people talking.",
            months,
            &some(&trends.top3_share, &permille),
            &percent,
        ));
    }
    panels.push(monthly(
        "How much was an answer",
        "The share of that month&#8217;s messages sent as a reply to another message.",
        months,
        &some(&trends.reply_share, &permille),
        &percent,
    ));
    let minimum = trends.latency_minimum;
    panels.push(monthly(
        "How long an answer took",
        &format!(
            "The median time from a message to a reply to it, for replies within a \
             day. A month with fewer than {} such replies is left empty: there, one \
             slow afternoon would be the median.",
            thousands(minimum)
        ),
        months,
        &trends.latency_median,
        &|n| duration(n as f64),
    ));
    panels.push(monthly(
        "Words per message",
        "The mean, over messages that have any text. A photo with no caption is \
         not a message of zero words.",
        months,
        &some(&trends.mean_words, &|v| (v * 10.0).round() as i64),
        &|n| format!("{:.1} words", n as f64 / 10.0),
    ));
    panels.push(monthly(
        "How much carried an attachment",
        "The share of that month&#8217;s messages with a photo, a file, a voice \
         note, a sticker or any other attachment.",
        months,
        &some(&trends.media_share, &permille),
        &percent,
    ));
    panels.push(monthly(
        "Reactions per message",
        "Every reaction that month, over every message that month.",
        months,
        &some(&trends.reactions_per_message, &|v| {
            (v * 100.0).round() as i64
        }),
        &|n| format!("{:.2}", n as f64 / 100.0),
    ));
    let panels: Vec<String> = panels.into_iter().filter(|p| !p.is_empty()).collect();
    if panels.is_empty() {
        return String::new();
    }
    let grid: String = panels
        .chunks(2)
        .map(|pair| format!("<div class=\"cols2\">{}</div>", pair.concat()))
        .collect();
    let led = if duo { String::new() } else { leaders(trends) };
    format!(
        "{}{grid}{led}</section>",
        head(
            "Over time",
            &format!("{} months", thousands(months.len() as i64)),
            "trend"
        )
    )
}
