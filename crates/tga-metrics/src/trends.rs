//! The archive's headline figures, month by month.
//!
//! Every ratio here is one the report already states once for the whole
//! archive — the loudest three's share, the reply share, the median reply,
//! words per message — computed the same way on one month at a time. A group
//! of two years is rarely the same group throughout, and a figure for the
//! whole of it describes a group that never existed.

use std::borrow::Cow;
use std::collections::HashMap;

use tga_read::{Export, Msg};
use tga_stats::{Leader, TrendRow, Trends};

use crate::conversation::{median, LATENCY_CAP};
use crate::identity::People;
use crate::util::{month_axis, Counter, MonthIndex};

/// Timed replies a month needs before its median is quoted. The floor
/// `dynamics::answer` uses for an hour, for the same reason: below it, one
/// slow afternoon is the median.
const LATENCY_MIN: i64 = 20;

/// How many of the loudest people get a row of their own.
const PEOPLE_SHOWN: usize = 12;

#[derive(Default)]
struct Month<'a> {
    messages: i64,
    with_text: i64,
    words: i64,
    media: i64,
    replies: i64,
    reactions: i64,
    latencies: Vec<i64>,
    by_person: Counter<Cow<'a, str>>,
}

/// `index` is [`Export::by_id`], built once by the caller.
pub fn compute(export: &Export, people: &People, index: &HashMap<i64, usize>) -> Trends {
    let said: Vec<&Msg> = export.said().collect();
    let months = month_axis(&said);
    let at = MonthIndex::new(&months);
    let mut cells: Vec<Month> = months.iter().map(|_| Month::default()).collect();
    let mut overall: Counter<Cow<str>> = Counter::new();

    for msg in &said {
        let Some(slot) = at.of(&msg.when) else {
            continue;
        };
        let cell = &mut cells[slot];
        let key = people.key_ref(msg);
        cell.messages += 1;
        if msg.words > 0 {
            cell.with_text += 1;
            cell.words += msg.words as i64;
        }
        if !msg.media.is_empty() {
            cell.media += 1;
        }
        cell.reactions += msg.reaction_total();
        if let Some(parent) = msg.reply_to {
            cell.replies += 1;
            if let Some(&p) = index.get(&parent) {
                let gap = msg.unix - export.msgs[p].unix;
                if (0..=LATENCY_CAP).contains(&gap) {
                    cell.latencies.push(gap);
                }
            }
        }
        if !key.is_empty() {
            cell.by_person.bump(key.clone());
            overall.bump(key);
        }
    }

    let ratio = |part: i64, whole: i64| {
        if whole > 0 {
            part as f64 / whole as f64
        } else {
            0.0
        }
    };
    let mut out = Trends {
        latency_minimum: LATENCY_MIN,
        ..Default::default()
    };
    for cell in &cells {
        let ranked = cell.by_person.most_common(None);
        let top3: i64 = ranked.iter().take(3).map(|(_, n)| n).sum();
        out.messages.push(cell.messages);
        out.voices.push(ranked.len() as i64);
        out.top3_share.push(ratio(top3, cell.messages));
        out.reply_share.push(ratio(cell.replies, cell.messages));
        out.latency_median.push(
            (cell.latencies.len() as i64 >= LATENCY_MIN)
                .then(|| median(&cell.latencies).round() as i64),
        );
        out.mean_words
            .push(crate::util::round1(ratio(cell.words, cell.with_text)));
        out.media_share.push(ratio(cell.media, cell.messages));
        out.reactions_per_message
            .push(ratio(cell.reactions, cell.messages));
        out.leaders.push(ranked.first().map(|(key, n)| Leader {
            name: people.name_of(key),
            key: key.to_string(),
            messages: *n,
            share: ratio(*n, cell.messages),
        }));
    }

    out.people = overall
        .most_common(Some(PEOPLE_SHOWN))
        .into_iter()
        .map(|(key, _)| TrendRow {
            name: people.name_of(&key),
            per_month: cells.iter().map(|c| c.by_person.get(&key)).collect(),
            key: key.into_owned(),
        })
        .collect();
    out.months = months;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, export, msg, on};

    #[test]
    fn each_month_gets_the_figures_the_archive_gets_once() {
        let mut picture = msg(4, "user2", on(2025, 3, 2, 9, 0), None);
        picture.media = "photo".into();
        picture.words = 0;
        let export = export(vec![
            msg(1, "user1", at(1, 9, 0), None),
            msg(2, "user1", at(1, 9, 5), None),
            msg(3, "user2", at(1, 9, 6), Some(1)),
            // February is silent.
            picture,
        ]);
        let people = People::new(&export);
        let out = compute(&export, &people, &export.by_id());
        assert_eq!(out.months, ["2025-01", "2025-02", "2025-03"]);
        assert_eq!(out.messages, [3, 0, 1]);
        assert_eq!(out.voices, [2, 0, 1]);
        assert_eq!(out.reply_share[0], 1.0 / 3.0);
        assert_eq!(out.media_share, [0.0, 0.0, 1.0]);
        // Words per message *with text*, so March's photo is not a zero.
        assert_eq!(out.mean_words, [1.0, 0.0, 0.0]);
        let jan = out.leaders[0].as_ref().expect("January had a leader");
        assert_eq!((jan.key.as_str(), jan.messages), ("user1", 2));
        assert!(out.leaders[1].is_none(), "nobody led a silent month");
        // One timed reply is far under the floor.
        assert_eq!(out.latency_median, [None, None, None]);
        assert_eq!(out.people[0].per_month, [2, 0, 0]);
        assert_eq!(out.people[1].per_month, [1, 0, 1]);
    }

    #[test]
    fn a_month_with_enough_replies_quotes_its_median() {
        let mut msgs = vec![msg(1, "user1", at(1, 9, 0), None)];
        for i in 0..LATENCY_MIN {
            msgs.push(msg(
                i + 2,
                "user2",
                at(1, 9, 0) + chrono::Duration::seconds(60),
                Some(1),
            ));
        }
        let export = export(msgs);
        let people = People::new(&export);
        let out = compute(&export, &people, &export.by_id());
        assert_eq!(out.latency_median, [Some(60)]);
    }
}
