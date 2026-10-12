//! Where each person sits between their first message and the last day.

use super::*;

struct Span {
    first: NaiveDate,
    last: NaiveDate,
    days: HashSet<NaiveDate>,
    messages: i64,
    name: String,
}

/// Where each person sits between their first message and the end of the
/// archive.
///
/// `dormant` is measured against the **archive's** last day rather than today,
/// because an export is a fixed document: reading the same file a year later
/// must not silently reclassify everyone in it as gone.
pub(super) fn tenure(msgs: &[&Msg], people: &People) -> Tenure {
    let archive_last = msgs[msgs.len() - 1].when.date();

    let mut order: Vec<String> = Vec::new();
    let mut spans: HashMap<String, Span> = HashMap::new();
    for msg in msgs {
        let key = people.key_of(msg);
        if key.is_empty() {
            continue;
        }
        let day = msg.when.date();
        let span = spans.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Span {
                first: day,
                last: day,
                days: HashSet::new(),
                messages: 0,
                name: people.name_of(&key),
            }
        });
        span.first = span.first.min(day);
        span.last = span.last.max(day);
        span.days.insert(day);
        span.messages += 1;
    }

    let mut rows: Vec<TenureRow> = Vec::new();
    let (mut active, mut fading, mut gone) = (0i64, 0i64, 0i64);
    let mut ranked: Vec<&String> = order.iter().collect();
    // Same order as `people.rows`, so the two tables can be read side by side.
    ranked.sort_by(|a, b| {
        spans[*b].messages.cmp(&spans[*a].messages).then_with(|| {
            spans[*a]
                .name
                .to_lowercase()
                .cmp(&spans[*b].name.to_lowercase())
        })
    });
    for key in ranked {
        let span = &spans[key];
        let dormant = (archive_last - span.last).num_days();
        let status = if dormant <= DORMANT_ACTIVE {
            active += 1;
            "active"
        } else if dormant <= DORMANT_FADING {
            fading += 1;
            "fading"
        } else {
            gone += 1;
            "gone"
        };
        let width = (span.last - span.first).num_days() + 1;
        rows.push(TenureRow {
            key: key.clone(),
            name: span.name.clone(),
            messages: span.messages,
            first: span.first.to_string(),
            last: span.last.to_string(),
            span_days: width,
            active_days: span.days.len(),
            // What share of the days they were around on did they actually
            // speak. A regular with a short tenure scores above somebody who
            // has been here for years and posts twice a season.
            density: if width > 0 {
                span.days.len() as f64 / width as f64
            } else {
                0.0
            },
            dormant_days: dormant,
            status: status.to_string(),
        });
    }

    Tenure {
        rows,
        as_of: archive_last.to_string(),
        active,
        fading,
        gone,
        active_within: DORMANT_ACTIVE,
        fading_within: DORMANT_FADING,
    }
}
