//! When the group talked.
//!
//! Every clock-face and calendar figure here reads `Msg::when`, the export's
//! local wall clock — see the note at the top of `tga-read`. "Who posts at
//! 3am" is a question about the clock in the room, not about UTC.
//!
//! The per-day series is **dense**: every date between the first message and
//! the last is present, silent days included, at zero. A sparse series drawn
//! as a line silently closes a two-month gap into a straight segment and
//! invents activity that never happened.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, Days, NaiveDate, Timelike};
use tga_read::Export;
use tga_stats::{Activity, Busiest, Count, Gap, Sparse};

use crate::identity::People;
use crate::util::round1;

pub fn compute(export: &Export, people: &People) -> Activity {
    let msgs: Vec<_> = export.said().collect();
    if msgs.is_empty() {
        return Activity {
            empty: true,
            ..Default::default()
        };
    }

    let mut per_day: HashMap<NaiveDate, i64> = HashMap::new();
    let mut per_hour = [0i64; 24];
    let mut per_weekday = [0i64; 7];
    let mut hour_weekday = [[0i64; 24]; 7];
    let mut by_topic: HashMap<usize, HashMap<NaiveDate, i64>> = HashMap::new();
    let mut by_person: HashMap<String, HashMap<NaiveDate, i64>> = HashMap::new();

    for msg in &msgs {
        let day = msg.when.date();
        let hour = msg.when.hour() as usize;
        let weekday = msg.when.weekday().num_days_from_monday() as usize;
        *per_day.entry(day).or_default() += 1;
        per_hour[hour] += 1;
        per_weekday[weekday] += 1;
        hour_weekday[weekday][hour] += 1;
        *by_topic
            .entry(msg.topic)
            .or_default()
            .entry(day)
            .or_default() += 1;
        let key = people.key_of(msg);
        if !key.is_empty() {
            *by_person.entry(key).or_default().entry(day).or_default() += 1;
        }
    }

    let first = msgs[0].when.date();
    let last = msgs[msgs.len() - 1].when.date();

    let mut days: Vec<(NaiveDate, i64)> = Vec::new();
    let mut cursor = first;
    while cursor <= last {
        days.push((cursor, per_day.get(&cursor).copied().unwrap_or(0)));
        cursor = cursor
            .checked_add_days(Days::new(1))
            .expect("date overflow");
    }

    let span_days = days.len();
    let active = days.iter().filter(|(_, n)| *n > 0).count();

    // The longest run of silent days, and where it sat. Reported because a
    // month of nothing is as much a fact about a group as its busiest week,
    // and a density ribbon renders it as blank space nobody can measure.
    let mut gap_len = 0usize;
    let mut gap_end: Option<NaiveDate> = None;
    let mut run = 0usize;
    for (day, count) in &days {
        if *count > 0 {
            run = 0;
        } else {
            run += 1;
            if run > gap_len {
                gap_len = run;
                gap_end = Some(*day);
            }
        }
    }
    let quietest = match (gap_len, gap_end) {
        (0, _) | (_, None) => None,
        (len, Some(end)) => {
            let from = end
                .checked_sub_days(Days::new(len as u64 - 1))
                .expect("date underflow");
            Some(Gap {
                days: len,
                from: from.to_string(),
                to: end.to_string(),
            })
        }
    };

    // max by count, ties going to the *later* day.
    let (busiest_day, busiest_count) = days
        .iter()
        .copied()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
        .expect("non-empty");

    let mut per_month: HashMap<String, i64> = HashMap::new();
    for (day, count) in &days {
        *per_month
            .entry(format!("{:04}-{:02}", day.year(), day.month()))
            .or_default() += count;
    }
    let mut per_month: Vec<(String, i64)> = per_month.into_iter().collect();
    per_month.sort();

    let sparse = |counts: &HashMap<NaiveDate, i64>| -> Sparse {
        counts
            .iter()
            .map(|(day, n)| (day.to_string(), *n))
            .collect()
    };

    Activity {
        empty: false,
        first: first.to_string(),
        last: last.to_string(),
        span_days,
        active_days: active,
        mean_per_active_day: if active > 0 {
            round1(msgs.len() as f64 / active as f64)
        } else {
            0.0
        },
        per_day: days
            .iter()
            .map(|(d, n)| Count::new(d.to_string(), *n))
            .collect(),
        per_month: per_month
            .into_iter()
            .map(|(m, n)| Count::new(m, n))
            .collect(),
        per_hour,
        per_weekday,
        hour_weekday,
        busiest_day: Busiest {
            date: busiest_day.to_string(),
            messages: busiest_count,
        },
        quietest,
        // Keyed by topic index and peer key, and **sparse**: the report draws
        // each as a ribbon on the same axis as the whole-archive one, so it
        // densifies against that shared axis at draw time. Storing them dense
        // here costs one entry per person per day of the archive, which for a
        // large group is millions of zeroes nobody reads.
        per_day_by_topic: by_topic
            .iter()
            .map(|(index, counts)| (index.to_string(), sparse(counts)))
            .collect::<BTreeMap<_, _>>(),
        per_day_by_person: by_person
            .iter()
            .map(|(key, counts)| (key.clone(), sparse(counts)))
            .collect::<BTreeMap<_, _>>(),
    }
}
