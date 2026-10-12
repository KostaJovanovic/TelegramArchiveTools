//! Of the people who spoke last month, how many spoke again.

use super::*;

/// Of the people who spoke last month, how many spoke again.
///
/// `churn` reports the arrivals and departures Telegram *announced*, which is
/// the only kind it can see. Nobody is announced for going quiet, and going
/// quiet is how a group actually ends — so this counts the four states that
/// membership list cannot: still here, back after a gap, new, and lost.
pub(super) fn retention(msgs: &[&Msg], people: &People) -> Retention {
    let month_of = |day: NaiveDate| format!("{:04}-{:02}", day.year(), day.month());

    let mut spoke: HashMap<String, HashSet<String>> = HashMap::new();
    for msg in msgs {
        let key = people.key_of(msg);
        if key.is_empty() {
            continue;
        }
        spoke
            .entry(month_of(msg.when.date()))
            .or_default()
            .insert(key);
    }

    // The axis is every month between the first and the last, silent ones
    // included. Taking only the months that appear closes a six-month gap into
    // a single step and reports the return after it as ordinary retention.
    let (first, last) = (msgs[0].when.date(), msgs[msgs.len() - 1].when.date());
    let mut months: Vec<String> = Vec::new();
    let (mut year, mut month) = (first.year(), first.month());
    while (year, month) <= (last.year(), last.month()) {
        months.push(format!("{year:04}-{month:02}"));
        if month == 12 {
            year += 1;
            month = 1;
        } else {
            month += 1;
        }
    }

    let empty: HashSet<String> = HashSet::new();
    let mut before: HashSet<String> = HashSet::new();
    let mut previous: &HashSet<String> = &empty;
    let (mut act, mut fresh, mut back, mut lost) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut kept: Vec<f64> = Vec::new();

    for name in &months {
        let now = spoke.get(name).unwrap_or(&empty);
        act.push(now.len() as i64);
        fresh.push(now.iter().filter(|k| !before.contains(*k)).count() as i64);
        back.push(now.intersection(previous).count() as i64);
        lost.push(previous.difference(now).count() as i64);
        if !previous.is_empty() {
            kept.push(now.intersection(previous).count() as f64 / previous.len() as f64);
        }
        before.extend(now.iter().cloned());
        previous = now;
    }

    Retention {
        months,
        active: act,
        new: fresh,
        returning: back,
        lost,
        people: before.len(),
        // The mean of the monthly rates, not the ratio of the totals: every
        // month gets one vote, so a single enormous month cannot speak for the
        // years around it. Emitted unrounded, like `share` and `reply_share` —
        // rounding to a percent here and dividing back by 100 puts float noise
        // in the dump and still hands the report the same figure.
        kept_mean: if kept.is_empty() {
            0.0
        } else {
            kept.iter().sum::<f64>() / kept.len() as f64
        },
        months_counted: kept.len(),
    }
}
