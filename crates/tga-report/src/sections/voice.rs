//! Voice notes and round videos, inside What was said.

use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

/// The panels, or nothing for a chat nobody recorded anything in.
pub fn voice(stats: &Stats) -> String {
    let Some(v) = &stats.voice else {
        return String::new();
    };
    let notes = v.voice_notes + v.video_notes;
    if notes == 0 {
        return String::new();
    }
    let seconds = v.voice_seconds + v.video_seconds;

    let what = match (v.voice_notes, v.video_notes) {
        (n, 0) => format!("{} voice note{}", thousands(n), plural(n, "", "s")),
        (0, m) => format!("{} round video{}", thousands(m), plural(m, "", "s")),
        (n, m) => format!(
            "{} voice note{} and {} round video{}",
            thousands(n),
            plural(n, "", "s"),
            thousands(m),
            plural(m, "", "s")
        ),
    };
    let lead = v
        .people
        .first()
        .map(|p| format!("; <b>{}</b> recorded the most", esc(&p.name)))
        .unwrap_or_default();
    let take = format!("{what}, {} in all{lead}.", duration(seconds as f64));

    let longest = v.longest.as_ref().map_or(String::new(), |l| {
        format!(
            " The longest ran {}, sent by {} on {}.",
            duration(l.seconds as f64),
            esc(&l.name),
            esc(&pretty_when(&l.date))
        )
    });
    let undated = if v.undated > 0 {
        format!(
            " {} note{} carried no length and add no time.",
            thousands(v.undated),
            plural(v.undated, "", "s")
        )
    } else {
        String::new()
    };
    let rows: Vec<charts::HBar> = v
        .people
        .iter()
        .map(|p| {
            let count = p.voice_notes + p.video_notes;
            charts::HBar::new(
                p.name.clone(),
                p.seconds as f64,
                format!(
                    "{} \u{b7} {} note{}",
                    duration(p.seconds as f64),
                    thousands(count),
                    plural(count, "", "s")
                ),
            )
            .who(p.key.clone())
        })
        .collect();
    let table_rows: Vec<Vec<String>> = v
        .people
        .iter()
        .map(|p| {
            vec![
                short(&p.name, 26),
                thousands(p.voice_notes),
                thousands(p.video_notes),
                esc(&duration(p.seconds as f64)),
                pct(p.share),
            ]
        })
        .collect();
    let people = panel(
        "Voice and video notes",
        &take,
        &format!(
            "Time recorded, from the length Telegram gives each note. A typical \
             note runs {}.{longest}{undated} The share is how much of everything \
             that person sent was a note, which is what a word count misses: their \
             notes are all filed under &#8220;no text&#8221; above.",
            duration(v.median_seconds as f64)
        ),
        &(charts::hbars(&rows, Some(0))
            + &data_view(
                "Notes, as numbers",
                &table_all(
                    &[
                        ("Person", false),
                        ("Voice", true),
                        ("Video", true),
                        ("Time", true),
                        ("Share of theirs", true),
                    ],
                    &table_rows,
                ),
            )),
    );

    let months = if v.per_month.len() >= 2 {
        let labels: Vec<String> = v
            .per_month
            .iter()
            .map(|c| c.label.chars().skip(2).collect())
            .collect();
        let values: Vec<i64> = v.per_month.iter().map(|c| c.n).collect();
        let shown = |n: i64| duration(n as f64);
        let busiest = argmax(&values);
        panel(
            "Recorded each month",
            &format!(
                "Most in <b>{}</b>: {}.",
                esc(&pretty_month(&v.per_month[busiest].label)),
                duration(values[busiest] as f64)
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
    format!("<div class=\"cols2\">{people}{months}</div>")
}
