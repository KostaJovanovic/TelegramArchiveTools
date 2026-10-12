//! The `dynamics` branch: pairs, answer latency, tenure, retention, depth.
//!
//! Five figures the counting elsewhere leaves unanswerable. The section
//! renders nothing at all when the branch is absent, which is what a dump
//! recorded before it existed looks like.

use std::fmt::Write as _;
use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

/// The `dynamics` branch: pairs, answer latency, tenure, retention and depth.
///
/// It renders nothing at all when the branch is absent, which is the ordinary
/// case for a `--from-stats` dump recorded before the branch existed.
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

    // -- who talks with whom ------------------------------------------------
    let rows = &pairs_b.rows;
    let bars: Vec<charts::BarRow> = rows
        .iter()
        .map(|row| {
            (
                format!("{} & {}", row.a_name, row.b_name),
                row.both as f64,
                format!(
                    "{} \u{2194} {}",
                    thousands(row.a_to_b),
                    thousands(row.b_to_a)
                ),
            )
        })
        .collect();
    let pair_chart = if bars.is_empty() {
        String::new()
    } else {
        charts::bars_h(&bars, WIDTH, 22.0, 260.0, 96.0)
    };
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
    let answer_chart = if shown.iter().all(|v| *v == 0) {
        String::new()
    } else {
        charts::columns(
            &hours,
            &shown,
            WIDTH / 2.0 - 28.0,
            190.0,
            3,
            " median",
            None,
        )
    };
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
    let depth_chart = if buckets.is_empty() {
        String::new()
    } else {
        charts::columns(
            &buckets.iter().map(|b| b.label.clone()).collect::<Vec<_>>(),
            &buckets.iter().map(|b| b.n).collect::<Vec<_>>(),
            WIDTH / 2.0 - 28.0,
            190.0,
            1,
            " chains",
            None,
        )
    };

    // -- month over month ---------------------------------------------------
    let months = &retention.months;
    let (active, fresh, back, lost) = (
        &retention.active,
        &retention.new,
        &retention.returning,
        &retention.lost,
    );
    let label_every = (months.len() / 12).max(1);
    let ret_chart = if months.is_empty() {
        String::new()
    } else {
        charts::columns(
            &months
                .iter()
                .map(|m| m[2..].to_string())
                .collect::<Vec<_>>(),
            back,
            WIDTH,
            170.0,
            label_every,
            " came back",
            None,
        )
    };
    let ret_rows: Vec<Vec<String>> = months
        .iter()
        .enumerate()
        .map(|(at, month)| {
            let cell = |series: &[i64]| thousands(series.get(at).copied().unwrap_or(0));
            vec![
                esc(month),
                cell(active),
                cell(fresh),
                cell(back),
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
        .take(PEOPLE_SHOWN)
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
                ("", false),
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

    if !pair_chart.is_empty() {
        let _ = write!(
            out,
            "<h3>Who talks with whom</h3>{pair_chart}\
             <p class=\"caption\">Ranked on the <em>smaller</em> of the two \
             directions, so a pair is only as strong as the quieter half of it. \
             The two numbers are replies each way. A pair that only ever runs \
             one way is somebody being answered, not a correspondence &#8212; \
             {} of the {} pairs here are that.</p>{}",
            thousands(pairs_b.one_way),
            thousands(pairs_b.mutual + pairs_b.one_way),
            data_view(
                "Correspondents, as numbers",
                &table(
                    &[
                        ("Person", false),
                        ("Person", false),
                        ("&rarr;", true),
                        ("&larr;", true),
                        ("Balance", true),
                    ],
                    &pair_rows,
                )
            )
        );
    }

    let _ = write!(
        out,
        "<div class=\"cols2\"><div><h3>How long until an answer</h3>{}\
         <p class=\"caption\">By the hour the message being answered was posted, \
         not the hour the answer arrived. An hour with fewer than {} replies is \
         left blank rather than drawn from too little. {} replies counted, none \
         further apart than {}.</p></div>\
         <div><h3>How far a thread runs</h3>{}\
         <p class=\"caption\">Messages by the length of the reply chain they sit \
         at the end of. A chain of two is one answer; most of the archive is a \
         chain of one and is not drawn.</p></div></div>{}",
        if answer_chart.is_empty() {
            "<p class=\"caption\">No hour has enough replies to quote a median \
             for.</p>"
                .to_string()
        } else {
            answer_chart
        },
        thousands(floor),
        thousands(counted),
        duration(answer.cap as f64),
        if depth_chart.is_empty() {
            "<p class=\"caption\">Nothing in this archive was replied to.</p>".to_string()
        } else {
            depth_chart
        },
        data_view(
            "Answer times, as numbers",
            &table(
                &[("Hour", false), ("Replies", true), ("Median", false)],
                &answer_rows,
            )
        )
    );

    if !ret_chart.is_empty() {
        let _ = write!(
            out,
            "<h3>Who came back</h3>{ret_chart}\
             <p class=\"caption\">People who spoke in a month <em>and</em> in the \
             month before it. Silent months stay on the axis: closing a gap would \
             report the first month after it as an ordinary return. Nobody is \
             announced for going quiet, which is why this is not in Coming and \
             going.</p>{}",
            data_view(
                "Month by month, as numbers",
                &table(
                    &[
                        ("Month", false),
                        ("Spoke", true),
                        ("New", true),
                        ("Returning", true),
                        ("Lost", true),
                    ],
                    &ret_rows,
                )
            )
        );
    }

    let _ = write!(
        out,
        "<h3>Gone quiet</h3>{quiet_table}\
         <p class=\"caption\">Measured against {}, the last day in this archive, \
         not against today &#8212; the same file read a year from now says the \
         same thing. Quiet past {} days, gone past {}.</p></section>",
        esc(&pretty_date(&tenure.as_of)),
        thousands(tenure.active_within),
        thousands(tenure.fading_within),
    );
    out
}
