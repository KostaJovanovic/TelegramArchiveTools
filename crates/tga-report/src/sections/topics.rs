//! One row per topic, on the same axis and the same shading scale.

use crate::stats::{densify, every_count, number};
use std::fmt::Write as _;
use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

pub fn topics(stats: &Stats) -> String {
    let rows = &stats.topics;
    let act = &stats.activity;
    if rows.is_empty() {
        return String::new();
    }
    let series = &act.per_day;
    let by_topic = &act.per_day_by_topic;
    let scale = Quantiles::new(every_count(by_topic));

    let mut body = String::new();
    for topic in rows {
        let key = topic.index.to_string();
        let mini = if series.is_empty() {
            String::new()
        } else {
            let dense = densify(by_topic.get(&key), series);
            presence(&dense, &scale)
        };
        // A topic's peak day is its own, not the archive's. Blank rather than
        // a zero when nothing was ever posted: there is no busiest day, and
        // "0 on 1970-01-01" is worse than an empty cell.
        let peak = if topic.busiest.messages == 0 {
            String::new()
        } else {
            format!(
                "{} <span class=\"at\">{}</span>",
                thousands(topic.busiest.messages),
                esc(&pretty_date(&topic.busiest.date))
            )
        };
        let _ = write!(
            body,
            "<tr><td class=\"name\">{}</td><td class=\"presence\">{mini}</td>\
             <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td>\
             <td class=\"n\">{peak}</td><td class=\"n\">{}</td>\
             <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td>\
             <td>{}</td></tr>",
            esc(&topic.name),
            thousands(topic.messages as i64),
            thousands(topic.voices as i64),
            thousands(topic.active_days as i64),
            // A topic nobody posted in carries no average at all, and the cell
            // reads `0` rather than being left blank -- the column is numeric
            // and a hole in it looks like a rendering fault.
            topic.avg_words.map_or_else(|| "0".to_string(), number),
            thousands(topic.media as i64),
            thousands(topic.replies as i64),
            thousands(topic.reactions),
            short(&topic.top, 22)
        );
    }
    let headers = "<tr><th>Topic</th><th>Activity</th><th class=\"n\">Messages</th>\
                   <th class=\"n\">Voices</th><th class=\"n\">Active days</th>\
                   <th class=\"n\">Busiest day</th><th class=\"n\">Words/msg</th>\
                   <th class=\"n\">Media</th><th class=\"n\">Replies</th>\
                   <th class=\"n\">Reactions</th><th>Loudest</th></tr>";
    format!(
        "{}<table><thead>{headers}</thead><tbody>{body}</tbody></table>\
         <p class=\"caption\">Same time axis as every other row on this page, \
         and one shading scale across the four. {}</p>{}</section>",
        head("Topics", &rows.len().to_string(), "topics"),
        esc(&scale.caption("messages in a day")),
        clocks(rows)
    )
}

/// Hour of day, one row per topic, each row on its own scale.
///
/// The question this answers is not "which topic is loud" — the table above
/// already answered that four different ways. It is "does this topic keep
/// different hours", and the reference archive says yes: the countdown channel
/// is daytime and the shitpost channel runs past 3am. On a shared scale that is
/// invisible, because one topic carries 139,743 messages and another 488.
fn clocks(rows: &[tga_stats::Topic]) -> String {
    // Every topic empty means every row is track, which is a grid of nothing.
    if rows.iter().all(|t| t.per_hour.iter().all(|n| *n == 0)) {
        return String::new();
    }
    let labels: Vec<String> = rows.iter().map(|t| short(&t.name, 22)).collect();
    let hours: Vec<String> = (0..24).map(|h| format!("{h:02}")).collect();
    let cells: Vec<Vec<i64>> = rows.iter().map(|t| t.per_hour.to_vec()).collect();
    let grid = charts::heatrows(&labels, &hours, &cells, WIDTH, 24.0, 190.0);
    format!(
        "<h3>When each topic is awake</h3><div class=\"split\"><div>{grid}</div>\
         <aside>{}<p class=\"caption\">One row per topic, one cell per hour of \
         the day, summed over the whole archive. <b>Each row is scaled to its \
         own busiest hour</b>, so the rows compare by shape and not by shade: a \
         bright cell in a small topic is not the same number of messages as a \
         bright cell in a large one. Hover any cell for both.</p></aside></div>",
        charts::legend(5, "this row's quietest", "its busiest")
    )
}
