//! The `dynamics` branch: pairs, answer latency, tenure, retention, depth.
//!
//! Five figures the counting elsewhere leaves unanswerable. The section
//! renders nothing at all when the branch is absent, which is what a dump
//! recorded before it existed looks like.

use std::fmt::Write as _;
use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

/// How many of the people who went quiet the table carries. It shows the
/// first [`charts::CLIP`] of them, like every long list.
const QUIET_ROWS: usize = 30;

/// The `dynamics` branch: pairs, answer latency, tenure, retention and depth.
///
/// It renders nothing at all when the branch is absent, which is the ordinary
/// case for a `--from-stats` dump recorded before the branch existed.
///
/// A private chat keeps only the answer times and the thread depth: a pair
/// ranking of one pair, and "came back" and "gone quiet" of two people, are
/// the group template talking to nobody.
pub fn between(stats: &Stats) -> String {
    let dynamics = match &stats.dynamics {
        Some(branch) if !branch.empty => branch,
        _ => return String::new(),
    };
    let (pairs_b, answer, tenure, retention, depth) = (
        &dynamics.pairs,
        &dynamics.answer,
        &dynamics.tenure,
        &dynamics.retention,
        &dynamics.depth,
    );
    let pair_chat = is_duo(stats);

    // -- who talks with whom ------------------------------------------------
    let rows = &pairs_b.rows;
    let bars: Vec<charts::HBar> = rows
        .iter()
        .map(|row| {
            charts::HBar::new(
                format!("{} & {}", row.a_name, row.b_name),
                row.both as f64,
                format!(
                    "{} \u{2194} {}",
                    thousands(row.a_to_b),
                    thousands(row.b_to_a)
                ),
            )
            .who(format!("{} {}", row.a, row.b))
        })
        .collect();
    let pair_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![
                short(&row.a_name, 26),
                short(&row.b_name, 26),
                thousands(row.a_to_b),
                thousands(row.b_to_a),
                pct(row.balance),
            ]
        })
        .collect();

    // -- when an answer arrives ---------------------------------------------
    let hours: Vec<String> = (0..24).map(|h| format!("{h:02}")).collect();
    let medians = answer.medians;
    let counts = answer.counts;
    let floor = answer.minimum;
    // An hour that has not met the floor is drawn as nothing rather than as a
    // short bar. A median over four replies is not a fast hour, and a bar is
    // read as one.
    let shown: Vec<i64> = medians
        .iter()
        .zip(counts.iter())
        .map(|(m, n)| if *n >= floor { *m } else { 0 })
        .collect();
    let wait = |v: i64| {
        if v > 0 {
            duration(v as f64)
        } else {
            "too few replies".to_string()
        }
    };
    let answer_chart = if shown.iter().all(|v| *v == 0) {
        "<p class=\"caption\">No hour has enough replies to quote a median for.</p>".to_string()
    } else {
        charts::vcols(&charts::Cols {
            labels: &hours,
            values: &shown,
            shown: &wait,
            every: 3,
            id: "",
        })
    };
    let answer_take = shown
        .iter()
        .enumerate()
        .max_by_key(|(hour, v)| (**v, std::cmp::Reverse(*hour)))
        .filter(|(_, v)| **v > 0)
        .map(|(hour, v)| {
            format!(
                "The longest wait is for messages posted at <b>{hour:02}:00</b>: {}.",
                esc(&duration(*v as f64))
            )
        })
        .unwrap_or_default();
    let answer_rows: Vec<Vec<String>> = medians
        .iter()
        .zip(counts.iter())
        .enumerate()
        .map(|(hour, (median, n))| {
            vec![
                format!("{hour:02}:00"),
                thousands(*n),
                if *n >= floor {
                    duration(*median as f64)
                } else {
                    "&#8212;".to_string()
                },
            ]
        })
        .collect();

    // -- how far a thread runs ----------------------------------------------
    let buckets = &depth.buckets;
    let chains = |n: i64| format!("{} chains", thousands(n));
    let depth_labels: Vec<String> = buckets.iter().map(|b| b.label.clone()).collect();
    let depth_values: Vec<i64> = buckets.iter().map(|b| b.n).collect();
    let depth_chart = if buckets.is_empty() {
        "<p class=\"caption\">Nothing in this archive was replied to.</p>".to_string()
    } else {
        charts::vcols(&charts::Cols {
            labels: &depth_labels,
            values: &depth_values,
            shown: &chains,
            every: 1,
            id: "",
        })
    };
    let depth_take = if depth.max > 1 {
        format!(
            "The longest chain of replies runs <b>{}</b> deep.",
            thousands(depth.max)
        )
    } else {
        String::new()
    };

    // -- month over month ---------------------------------------------------
    let months = &retention.months;
    let (active, fresh, stayed, lost) = (
        &retention.active,
        &retention.new,
        // `returning` is the old key for people who spoke two months running;
        // see `tga_stats::Retention`.
        &retention.returning,
        &retention.lost,
    );
    // Who came back after a silence of their own. A dump recorded before the
    // series existed has the three it is the remainder of, so it is read the
    // same from either.
    let back: Vec<i64> = if retention.came_back.len() == months.len() {
        retention.came_back.clone()
    } else {
        (0..months.len())
            .map(|at| {
                let of = |series: &[i64]| series.get(at).copied().unwrap_or(0);
                (of(active) - of(fresh) - of(stayed)).max(0)
            })
            .collect()
    };
    let label_every = (months.len() / 12).max(1);
    let month_labels: Vec<String> = months
        .iter()
        .map(|m| m.get(2..).unwrap_or(m).to_string())
        .collect();
    let people = |n: i64| format!("{} came back", thousands(n));
    let ret_chart = if months.is_empty() {
        String::new()
    } else {
        charts::vcols(&charts::Cols {
            labels: &month_labels,
            values: &back,
            shown: &people,
            every: label_every,
            id: "",
        })
    };
    let ret_take = back
        .iter()
        .enumerate()
        .max_by_key(|(at, n)| (**n, std::cmp::Reverse(*at)))
        .filter(|(_, n)| **n > 0)
        .map(|(at, n)| {
            format!(
                "Most returns in <b>{}</b>: {} people.",
                esc(&pretty_month(&months[at])),
                thousands(*n)
            )
        })
        .unwrap_or_default();
    let ret_rows: Vec<Vec<String>> = months
        .iter()
        .enumerate()
        .map(|(at, month)| {
            let cell = |series: &[i64]| thousands(series.get(at).copied().unwrap_or(0));
            vec![
                esc(month),
                cell(active),
                cell(fresh),
                cell(stayed),
                cell(&back),
                cell(lost),
            ]
        })
        .collect();

    // -- who is still here --------------------------------------------------
    // Ranked by the silence rather than by the message count, because that is
    // the one ordering the People table cannot already give you.
    let mut quiet: Vec<&tga_stats::TenureRow> = tenure
        .rows
        .iter()
        .filter(|row| row.status != "active")
        .collect();
    quiet.sort_by_key(|row| std::cmp::Reverse(row.dormant_days));
    let quiet_rows: Vec<Vec<String>> = quiet
        .iter()
        .take(QUIET_ROWS)
        .map(|row| {
            vec![
                short(&row.name, 30),
                thousands(row.messages),
                esc(&pretty_date(&row.last)),
                thousands(row.dormant_days),
                esc(&row.status),
            ]
        })
        .collect();
    let quiet_table = if quiet_rows.is_empty() {
        "<p class=\"caption\">Everybody with a message in this archive posted \
         one in its last month.</p>"
            .to_string()
    } else {
        table(
            &[
                ("Person", false),
                ("Messages", true),
                ("Last seen", false),
                ("Days since", true),
                ("Status", false),
            ],
            &quiet_rows,
        )
    };

    let counted = answer.counted;
    let stat_line = format!(
        "{} mutual, {} one-way",
        thousands(pairs_b.mutual),
        thousands(pairs_b.one_way)
    );

    let mut out = head("Between people", &stat_line, "between");
    if !pair_chat {
        let _ = write!(
            out,
            "{}",
            figures(&[
                (thousands(pairs_b.mutual), "pairs answering each other"),
                (thousands(tenure.active), "still posting"),
                (thousands(tenure.fading), "gone quiet"),
                (thousands(tenure.gone), "long gone"),
                (pct(retention.kept_mean), "kept month to month"),
                (thousands(depth.max), "deepest chain"),
            ])
        );
    }

    if !pair_chat && !bars.is_empty() {
        let first = &rows[0];
        let _ = write!(
            out,
            "{}",
            panel(
                "Who talks with whom",
                &format!(
                    "<b>{}</b> and <b>{}</b> answer each other most.",
                    esc(&first.a_name),
                    esc(&first.b_name)
                ),
                &format!(
                    "Ranked on the <em>smaller</em> of the two directions, so a \
                     pair is only as strong as the quieter half of it. The two \
                     numbers are replies each way. A pair that only ever runs one \
                     way is somebody being answered, not a correspondence &#8212; \
                     {} of the {} pairs here are that.",
                    thousands(pairs_b.one_way),
                    thousands(pairs_b.mutual + pairs_b.one_way)
                ),
                &(charts::hbars(&bars, Some(0))
                    + &data_view(
                        "Correspondents, as numbers",
                        &table_all(
                            // The arrows as characters: `table` escapes every
                            // header, so an entity here reached the page as the
                            // literal text `&rarr;`.
                            &[
                                ("Person", false),
                                ("Person", false),
                                ("\u{2192}", true),
                                ("\u{2190}", true),
                                ("Balance", true),
                            ],
                            &pair_rows,
                        )
                    )),
            )
        );
    }

    let _ = write!(
        out,
        "<div class=\"cols2\">{}{}</div>",
        panel(
            "How long until an answer",
            &answer_take,
            &format!(
                "The median wait for an answer, by the hour the message being \
                 answered was posted, not the hour the answer arrived. An hour \
                 with fewer than {} replies is left blank rather than drawn from \
                 too little. {} replies counted, none further apart than {}.",
                thousands(floor),
                thousands(counted),
                esc(&duration(answer.cap as f64))
            ),
            &(answer_chart
                + &data_view(
                    "Answer times, as numbers",
                    &table_all(
                        &[("Hour", false), ("Replies", true), ("Median", false)],
                        &answer_rows,
                    )
                )),
        ),
        panel(
            "How far a thread runs",
            &depth_take,
            "Messages by the length of the reply chain they sit at the end of. A \
             chain of two is one answer; most of the archive is a chain of one and \
             is not drawn.",
            &depth_chart,
        )
    );

    if pair_chat {
        out.push_str("</section>");
        return out;
    }

    if !ret_chart.is_empty() {
        let _ = write!(
            out,
            "{}",
            panel(
                "Who came back",
                &ret_take,
                "People who spoke in a month after a silence of their own: they \
                 had spoken before, but not in the month before it. Silent months \
                 stay on the axis, so a gap in the group is a gap here too. \
                 Everybody who spoke in a month is new, stayed from the month \
                 before, or came back. Nobody is announced for going quiet, which \
                 is why this is not in Coming and going.",
                &(ret_chart
                    + &data_view(
                        "Month by month, as numbers",
                        &table_all(
                            &[
                                ("Month", false),
                                ("Spoke", true),
                                ("New", true),
                                ("Stayed", true),
                                ("Came back", true),
                                ("Lost", true),
                            ],
                            &ret_rows,
                        )
                    )),
            )
        );
    }

    let quiet_take = if quiet.is_empty() {
        String::new()
    } else {
        format!(
            "{} people have gone quiet or are gone.",
            thousands(quiet.len() as i64)
        )
    };
    let _ = write!(
        out,
        "{}</section>",
        panel(
            "Gone quiet",
            &quiet_take,
            &format!(
                "Measured against {}, the last day in this archive, not against \
                 today &#8212; the same file read a year from now says the same \
                 thing. Quiet past {} days, gone past {}.",
                esc(&pretty_date(&tenure.as_of)),
                thousands(tenure.active_within),
                thousands(tenure.fading_within),
            ),
            &quiet_table,
        )
    );
    out
}
