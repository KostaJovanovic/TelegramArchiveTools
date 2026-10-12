//! The clock and the calendar: hour of day, day of week, and both at once.

use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

pub fn rhythm(stats: &Stats) -> String {
    let act = &stats.activity;
    if is_empty_activity(act) {
        return String::new();
    }
    let hours: Vec<String> = (0..24).map(|h| format!("{h:02}")).collect();
    let weekdays: Vec<String> = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let per_hour = act.per_hour;
    let per_weekday = act.per_weekday;
    let hour_weekday: Vec<Vec<i64>> = act.hour_weekday.iter().map(|row| row.to_vec()).collect();
    let series = &act.per_day;

    let hour_chart = charts::columns(
        &hours,
        &per_hour,
        WIDTH / 2.0 - 28.0,
        190.0,
        3,
        " messages",
        None,
    );
    let day_chart = charts::columns(
        &weekdays,
        &per_weekday,
        WIDTH / 2.0 - 28.0,
        190.0,
        1,
        " messages",
        None,
    );
    let grid_scale = Quantiles::new(hour_weekday.iter().flatten().copied());
    let heat = charts::heatgrid(
        &weekdays,
        &hours,
        &hour_weekday,
        WIDTH,
        28.0,
        46.0,
        Some(&grid_scale),
    );
    let cal_scale = Quantiles::new(series.iter().map(|day| day.n));
    let cal = charts::calendar(series, WIDTH, None, Some(&cal_scale));

    let full = [
        "Mondays",
        "Tuesdays",
        "Wednesdays",
        "Thursdays",
        "Fridays",
        "Saturdays",
        "Sundays",
    ];
    let peak_hour = argmax(&per_hour);
    let peak_day = full[argmax(&per_weekday).min(6)];

    let hour_rows: Vec<Vec<String>> = per_hour
        .iter()
        .enumerate()
        .map(|(h, n)| vec![format!("{h:02}:00"), thousands(*n)])
        .collect();

    format!(
        "{}<div class=\"cols2\"><div><h3>Hour of day</h3>{hour_chart}\
         <p class=\"caption\">Local time, as the export recorded it.</p></div>\
         <div><h3>Day of week</h3>{day_chart}\
         <p class=\"caption\">Every message, across the whole archive.</p></div></div>\
         <h3>Both at once</h3><div class=\"split\"><div>{heat}</div><aside>{}\
         <p class=\"caption\">Each cell is one hour of one weekday, summed over \
         the whole archive. An empty cell is an hour in which nothing was ever \
         said. {}</p></aside></div>\
         <h3>Every day</h3><div class=\"split\"><div>{cal}</div><aside>{}\
         <p class=\"caption\">One square per day. {}</p></aside></div>{}</section>",
        head(
            "Rhythm",
            &format!("busiest at {peak_hour:02}:00, on {peak_day}"),
            "rhythm"
        ),
        charts::legend(5, "quiet", "busy"),
        esc(&grid_scale.caption("messages")),
        charts::legend(5, "silent", "busiest"),
        esc(&cal_scale.caption("messages")),
        data_view(
            "Hour of day, as numbers",
            &table(&[("Hour", false), ("Messages", true)], &hour_rows)
        )
    )
}
