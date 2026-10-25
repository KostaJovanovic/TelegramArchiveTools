//! The clock and the calendar: hour of day, day of week, and both at once.

use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

/// The width the heat grid's labels are a reading size at. Narrower than that
/// it pans inside its frame rather than shrinking.
const GRID_MIN: f64 = 900.0;
/// One calendar square, in pixels, at every screen width.
const CAL_CELL: f64 = 11.0;

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

    let messages = |n: i64| format!("{} messages", thousands(n));
    let hour_chart = match duo(stats) {
        Some((a, b)) => {
            duo_key(&a.name, &b.name)
                + &charts::vcols2(&hours, &a.hours, &b.hours, (&a.name, &b.name), 3)
        }
        None => charts::vcols(&charts::Cols {
            labels: &hours,
            values: &per_hour,
            shown: &messages,
            every: 3,
            id: "hours-chart",
        }),
    };
    let day_chart = charts::vcols(&charts::Cols {
        labels: &weekdays,
        values: &per_weekday,
        shown: &messages,
        every: 1,
        id: "",
    });
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
    let (cal, cal_width) = charts::calendar(series, CAL_CELL, Some(&cal_scale));

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
    let quiet_day = {
        let low = per_weekday.iter().min().copied().unwrap_or(0);
        full[per_weekday
            .iter()
            .position(|n| *n == low)
            .unwrap_or(0)
            .min(6)]
    };

    // The share of the day's messages that fall in office hours is the one
    // number that says whether this is a work chat or an evening one.
    let total: i64 = per_hour.iter().sum();
    let office: i64 = per_hour[9..18].iter().sum();
    let hour_take = format!(
        "Busiest at <b>{peak_hour:02}:00</b>; {} of messages arrive between 09:00 and 18:00.",
        share(office, total)
    );
    let day_take = format!("<b>{peak_day}</b> are busiest, {quiet_day} quietest.");

    let (cell_day, cell_hour, cell_n) = act
        .hour_weekday
        .iter()
        .enumerate()
        .flat_map(|(d, row)| row.iter().enumerate().map(move |(h, n)| (d, h, *n)))
        .max_by_key(|(d, h, n)| (*n, std::cmp::Reverse((*d, *h))))
        .unwrap_or((0, 0, 0));
    let grid_take = format!(
        "The single busiest hour of the week is <b>{} {cell_hour:02}:00</b>, {}.",
        WEEKDAYS[cell_day.min(6)],
        messages(cell_n)
    );
    let active = act.active_days;
    let cal_take = format!(
        "Messages on {} of {} days.",
        thousands(active as i64),
        thousands(act.span_days as i64)
    );

    let hour_rows: Vec<Vec<String>> = per_hour
        .iter()
        .enumerate()
        .map(|(h, n)| vec![format!("{h:02}:00"), thousands(*n)])
        .collect();

    let hour_method = if is_duo(stats) {
        "Local time, as the export recorded it. Each person's own messages per \
         hour, side by side on one scale."
    } else {
        "Local time, as the export recorded it. Every message, across the whole archive."
    };

    format!(
        "{}<div class=\"cols2\">{}{}</div>{}{}{}</section>",
        head(
            "Rhythm",
            &format!("busiest at {peak_hour:02}:00, on {peak_day}"),
            "rhythm"
        ),
        panel("Hour of day", &hour_take, hour_method, &hour_chart),
        panel(
            "Day of week",
            &day_take,
            "Every message, across the whole archive, by the weekday it was sent on.",
            &day_chart
        ),
        panel(
            "Hour and weekday",
            &grid_take,
            &format!(
                "Each cell is one hour of one weekday, summed over the whole \
                 archive. An empty cell is an hour in which nothing was ever said. {}",
                esc(&grid_scale.caption("messages"))
            ),
            &(charts::scroll(&heat, GRID_MIN) + &charts::legend(5, "quiet", "busy")),
        ),
        panel(
            "Every day",
            &cal_take,
            &format!(
                "One square per day, a column per week, newest on the right. {}",
                esc(&cal_scale.caption("messages"))
            ),
            &(charts::scroll_fixed(&cal, cal_width) + &charts::legend(5, "silent", "busiest")),
        ),
        data_view(
            "Hour of day, as numbers",
            &table_all(&[("Hour", false), ("Messages", true)], &hour_rows)
        )
    )
}
