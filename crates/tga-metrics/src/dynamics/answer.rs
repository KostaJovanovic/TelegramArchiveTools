//! Reply latency, cut by the hour the parent was posted.

use super::*;

/// Reply latency, cut by the hour the message being answered was posted.
///
/// Keyed on the **parent's** hour, not the reply's. "How long until somebody
/// answers me" is a question about when you posted; keying it on when the
/// answer arrived measures the answerer's habits instead, which is a different
/// question and the one `conversation::fastest` already covers.
pub(super) fn answer(msgs: &[&Msg], export: &Export, index: &HashMap<i64, usize>) -> Answer {
    let mut buckets: Vec<Vec<i64>> = vec![Vec::new(); 24];
    for msg in msgs {
        let Some(parent) = parent_of(msg, export, index) else {
            continue;
        };
        let gap = msg.unix - parent.unix;
        if (0..=LATENCY_CAP).contains(&gap) {
            buckets[parent.when.hour() as usize].push(gap);
        }
    }

    let mut counts = [0i64; 24];
    let mut medians = [0i64; 24];
    for (hour, values) in buckets.iter().enumerate() {
        counts[hour] = values.len() as i64;
        medians[hour] = if values.is_empty() {
            0
        } else {
            median(values).trunc() as i64
        };
    }

    // Only hours with enough replies to mean something get to be the best or
    // the worst. Without the floor both are won by 04:00, where three replies
    // landed and one of them was instant.
    let solid: Vec<usize> = (0..24).filter(|h| counts[*h] >= ANSWER_MIN).collect();
    let pick = |wanted: &dyn Fn(i64, i64) -> bool| -> Option<HourPick> {
        let mut best: Option<usize> = None;
        for hour in &solid {
            match best {
                Some(current) if !wanted(medians[*hour], medians[current]) => {}
                _ => best = Some(*hour),
            }
        }
        best.map(|hour| HourPick {
            hour,
            median: medians[hour],
            replies: counts[hour],
        })
    };

    Answer {
        counted: counts.iter().sum::<i64>(),
        counts,
        medians,
        minimum: ANSWER_MIN,
        cap: LATENCY_CAP,
        fastest_hour: pick(&|candidate, current| candidate < current),
        slowest_hour: pick(&|candidate, current| candidate > current),
    }
}
