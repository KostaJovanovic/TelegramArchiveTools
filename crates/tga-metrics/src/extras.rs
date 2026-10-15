//! The bits that do not fit a chart: records, arrivals and departures, topics.
//!
//! A superlative is a claim about one message, so each one carries the id and
//! date that back it. Nothing here reports a record without saying which
//! message it was — an unsourced "busiest day" is a number the reader cannot
//! check.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate, Timelike};
use tga_read::{Export, Msg};
use tga_stats::{
    human_bytes, Activity, Award, Busiest, Churn, ChurnEvent, Figure, Person, Streak, Superlative,
    Topic,
};

use crate::identity::People;
use crate::util::{round1, stamp_minutes, Counter};

/// Actions that add or remove somebody, as Desktop names them.
const JOIN_ACTIONS: &[&str] = &[
    "join_group_by_link",
    "invite_members",
    "invite_to_group_call",
];
const LEAVE_ACTIONS: &[&str] = &["remove_members"];

/// The small hours, for the night-owl figure. Stated in the report so the
/// reader can disagree with the boundary rather than guess at it.
const NIGHT: std::ops::Range<usize> = 0..5;
const MORNING: std::ops::Range<usize> = 5..9;

fn snippet(msg: &Msg) -> String {
    const LIMIT: usize = 140;
    let text = msg.text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return if msg.media.is_empty() {
            "(no text)".to_string()
        } else {
            format!("({})", msg.media.replace('_', " "))
        };
    }
    if text.chars().count() <= LIMIT {
        text
    } else {
        let head: String = text.chars().take(LIMIT - 1).collect();
        format!("{head}…")
    }
}

/// A record about one message, with the message named.
fn record(title: &str, msg: &Msg, people: &People, value: Figure, unit: &str) -> Superlative {
    Superlative {
        title: title.to_string(),
        id: Some(msg.id),
        topic: Some(msg.topic),
        date: stamp_minutes(&msg.when),
        who: people.name_of(&people.key_of(msg)),
        text: snippet(msg),
        value,
        unit: unit.to_string(),
    }
}

/// `index` is [`Export::by_id`], built once by the caller.
pub fn superlatives(
    export: &Export,
    people: &People,
    activity: &Activity,
    index: &HashMap<i64, usize>,
) -> Vec<Superlative> {
    let msgs: Vec<&Msg> = export.said().collect();
    if msgs.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Superlative> = Vec::new();

    // `max` on `(value, -id)`: the biggest, and on a tie the *lower* id.
    let best_by = |score: &dyn Fn(&Msg) -> i64| -> &Msg {
        msgs.iter()
            .copied()
            .max_by(|a, b| score(a).cmp(&score(b)).then_with(|| b.id.cmp(&a.id)))
            .expect("non-empty")
    };

    let reacted = best_by(&|m: &Msg| m.reaction_total());
    if reacted.reaction_total() > 0 {
        out.push(record(
            "Most reacted to",
            reacted,
            people,
            Figure::Count(reacted.reaction_total()),
            "reactions",
        ));
    }

    let longest = best_by(&|m: &Msg| m.words as i64);
    if longest.words > 0 {
        out.push(record(
            "Longest message",
            longest,
            people,
            Figure::Count(longest.words as i64),
            "words",
        ));
    }

    let biggest = best_by(&|m: &Msg| m.file_size);
    if biggest.file_size > 0 {
        out.push(record(
            "Largest file",
            biggest,
            people,
            Figure::Text(human_bytes(biggest.file_size)),
            "",
        ));
    }

    let mut replied: Counter<i64> = Counter::new();
    for msg in &msgs {
        if let Some(parent) = msg.reply_to {
            replied.bump(parent);
        }
    }
    if !replied.is_empty() {
        let (target_id, count) = replied.most_common(Some(1))[0];
        // Spans every message, service entries included — a reply can point at
        // one.
        if let Some(&at) = index.get(&target_id) {
            out.push(record(
                "Most replied to",
                &export.msgs[at],
                people,
                Figure::Count(count),
                "replies",
            ));
        }
    }

    // The two that are about the calendar rather than about one message, and
    // so the only two with no id to cite.
    if !activity.empty {
        out.push(Superlative {
            title: "Busiest day".into(),
            date: activity.busiest_day.date.clone(),
            value: Figure::Count(activity.busiest_day.messages),
            unit: "messages".into(),
            ..Default::default()
        });
        if let Some(quiet) = &activity.quietest {
            out.push(Superlative {
                title: "Longest silence".into(),
                date: format!("{} to {}", quiet.from, quiet.to),
                value: Figure::Count(quiet.days as i64),
                unit: "days".into(),
                ..Default::default()
            });
        }
    }

    out
}

/// Per-person records, restricted to people with enough messages to mean it.
///
/// Without the floor every one of these is won by somebody who sent four
/// messages, all of them at 4am, and the report says something true about
/// nobody.
fn hour_share(row: &Person, hours: std::ops::Range<usize>) -> f64 {
    let sum: i64 = hours.map(|h| row.hours[h]).sum();
    sum as f64 / row.messages as f64
}

pub fn awards(rows: &[Person], minimum: i64) -> Vec<Award> {
    let eligible: Vec<&Person> = rows.iter().filter(|r| r.messages >= minimum).collect();
    if eligible.is_empty() {
        return Vec::new();
    }

    // Plain functions rather than closures: a `Box<dyn Fn>` is `'static` by
    // default, and a closure over two locals cannot be.
    type Score = fn(&Person) -> f64;
    let picks: [(&str, &str, Score); 4] = [
        (
            "Night owl",
            "of their messages between midnight and 5am",
            |r| hour_share(r, NIGHT),
        ),
        ("Early bird", "of their messages between 5am and 9am", |r| {
            hour_share(r, MORNING)
        }),
        ("Most reacted to", "reactions per message they sent", |r| {
            r.reactions_received as f64 / r.messages as f64
        }),
        ("Writes longest", "words per message on average", |r| {
            r.avg_words
        }),
    ];

    let mut out = Vec::new();
    for (title, note, score) in picks {
        let best = eligible
            .iter()
            .copied()
            .max_by(|a, b| {
                score(a)
                    .partial_cmp(&score(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.messages.cmp(&a.messages))
            })
            .expect("non-empty");
        let value = score(best);
        if value <= 0.0 {
            continue;
        }
        out.push(Award {
            title: title.to_string(),
            name: best.name.clone(),
            key: best.key.clone(),
            value: if note.starts_with("of their") {
                format!("{:.0}%", value * 100.0)
            } else {
                format!("{value:.1}")
            },
            note: note.to_string(),
            messages: best.messages,
        });
    }
    out
}

/// The longest run of consecutive days somebody posted on.
pub fn streaks(export: &Export, people: &People) -> Option<Streak> {
    let mut order: Vec<String> = Vec::new();
    let mut per_person: HashMap<String, HashSet<NaiveDate>> = HashMap::new();
    for msg in export.said() {
        let key = people.key_of(msg);
        if key.is_empty() {
            continue;
        }
        per_person
            .entry(key.clone())
            .or_insert_with(|| {
                order.push(key.clone());
                HashSet::new()
            })
            .insert(msg.when.date());
    }

    let mut best_key = String::new();
    let mut best_run = 0usize;
    let mut best_end: Option<NaiveDate> = None;
    for key in &order {
        let mut days: Vec<NaiveDate> = per_person[key].iter().copied().collect();
        days.sort_unstable();
        // Note the shape: `run` is only ever *compared* inside the pairwise
        // walk, so somebody who posted on exactly one day never registers at
        // all. A one-day streak is not a streak.
        let mut run = 1usize;
        for pair in days.windows(2) {
            run = if (pair[1] - pair[0]).num_days() == 1 {
                run + 1
            } else {
                1
            };
            if run > best_run {
                best_key = key.clone();
                best_run = run;
                best_end = Some(pair[1]);
            }
        }
    }

    if best_key.is_empty() {
        return None;
    }
    Some(Streak {
        name: people.name_of(&best_key),
        days: best_run,
        ended: best_end.map(|d| d.to_string()).unwrap_or_default(),
    })
}

/// Arrivals, departures, and how many people were actually talking.
///
/// Arrivals come from two places that disagree, and both are reported.
/// `participants.json` dates every *current* member's join, which is the
/// better source but says nothing about anyone who has since left. The service
/// messages in the history record joins and removals as they happened, but
/// only the ones Telegram announced.
pub fn churn(export: &Export, people: &People, activity: &Activity) -> Churn {
    let month_of = |year: i32, month: u32| format!("{year:04}-{month:02}");

    let mut joined: Counter<String> = Counter::new();
    for member in &export.roster {
        if let Some(when) = member.joined {
            joined.bump(month_of(when.year(), when.month()));
        }
    }

    let mut announced_join: Counter<String> = Counter::new();
    let mut announced_leave: Counter<String> = Counter::new();
    let mut events: Vec<ChurnEvent> = Vec::new();
    for msg in &export.msgs {
        if !msg.service {
            continue;
        }
        let month = month_of(msg.when.year(), msg.when.month());
        let is_join = JOIN_ACTIONS.contains(&msg.action.as_str());
        let is_leave = !is_join && LEAVE_ACTIONS.contains(&msg.action.as_str());
        if !is_join && !is_leave {
            continue;
        }
        let count = if msg.members.is_empty() {
            1
        } else {
            msg.members.len()
        };
        if is_join {
            announced_join.add(month.clone(), count as i64);
        } else {
            announced_leave.add(month.clone(), count as i64);
        }
        events.push(ChurnEvent {
            date: stamp_minutes(&msg.when),
            kind: if is_join { "join" } else { "leave" }.to_string(),
            count,
            who: if msg.name.is_empty() {
                people.name_of(&msg.sender)
            } else {
                msg.name.clone()
            },
            names: msg.members.clone(),
        });
    }

    let mut speaking: HashMap<String, HashSet<String>> = HashMap::new();
    for msg in export.said() {
        let key = people.key_of(msg);
        if key.is_empty() {
            continue;
        }
        speaking
            .entry(month_of(msg.when.year(), msg.when.month()))
            .or_default()
            .insert(key);
    }

    let mut months: Vec<String> = activity
        .per_month
        .iter()
        .map(|row| row.label.clone())
        .collect();
    months.extend(joined.keys().cloned());
    months.extend(announced_join.keys().cloned());
    months.extend(speaking.keys().cloned());
    months.sort();
    months.dedup();

    Churn {
        roster_joins: months.iter().map(|m| joined.get(m)).collect(),
        announced_joins: months.iter().map(|m| announced_join.get(m)).collect(),
        announced_leaves: months.iter().map(|m| announced_leave.get(m)).collect(),
        active: months
            .iter()
            .map(|m| speaking.get(m).map_or(0, HashSet::len))
            .collect(),
        months,
        events,
        roster_dated: joined.total(),
        roster_size: export.roster.len(),
    }
}

/// Every metric that fits in a row, cut per topic.
pub fn topics(export: &Export, people: &People) -> Vec<Topic> {
    // Every topic's messages in one walk, in archive order. Filtering the
    // whole archive once per topic is topics times messages, which for a
    // forum of two hundred topics is ninety million visits to say the same
    // thing.
    let mut by_topic: HashMap<usize, Vec<&Msg>> = HashMap::new();
    for msg in export.said() {
        by_topic.entry(msg.topic).or_default().push(msg);
    }
    let nothing: Vec<&Msg> = Vec::new();

    let mut out = Vec::new();
    for topic in &export.topics {
        let stream = by_topic.get(&topic.index).unwrap_or(&nothing);
        if stream.is_empty() {
            // No `avg_words` at all rather than a zero: there is no average of
            // nothing, and 0.0 would read as "they wrote empty messages".
            out.push(Topic {
                index: topic.index,
                name: topic.name.clone(),
                ..Default::default()
            });
            continue;
        }
        let mut voices: Counter<String> = Counter::new();
        let mut per_hour = [0i64; 24];
        let mut per_day: HashMap<NaiveDate, i64> = HashMap::new();
        for msg in stream {
            let key = people.key_of(msg);
            if !key.is_empty() {
                voices.bump(key);
            }
            per_hour[msg.when.hour() as usize] += 1;
            *per_day.entry(msg.when.date()).or_default() += 1;
        }
        // Ties go to the earlier day. `max_by_key` keeps the *last* maximum,
        // so the sort is explicit rather than left to the iterator: two days
        // with the same count are a coin toss, and a coin toss that changes
        // between runs would fail the baseline for no reason.
        let busiest = {
            let mut days: Vec<(&NaiveDate, &i64)> = per_day.iter().collect();
            days.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            days.first()
                .map_or_else(Busiest::default, |(day, n)| Busiest {
                    date: day.to_string(),
                    messages: **n,
                })
        };
        let words: i64 = stream.iter().map(|m| m.words as i64).sum();
        out.push(Topic {
            active_days: per_day.len(),
            busiest,
            per_hour,
            index: topic.index,
            name: topic.name.clone(),
            messages: stream.len(),
            voices: voices.len(),
            words,
            avg_words: Some(round1(words as f64 / stream.len() as f64)),
            media: stream.iter().filter(|m| !m.media.is_empty()).count(),
            replies: stream.iter().filter(|m| m.reply_to.is_some()).count(),
            reactions: stream.iter().map(|m| m.reaction_total()).sum::<i64>(),
            first: stream[0].when.date().to_string(),
            last: stream[stream.len() - 1].when.date().to_string(),
            top: if voices.is_empty() {
                String::new()
            } else {
                people.name_of(&voices.most_common(Some(1))[0].0)
            },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, export, msg};

    /// A person who sent `messages`, `night` of them between midnight and 5.
    fn person(key: &str, messages: i64, night: i64) -> Person {
        let mut hours = [0i64; 24];
        hours[2] = night;
        hours[12] = messages - night;
        Person {
            key: key.into(),
            name: key.into(),
            messages,
            hours,
            ..Default::default()
        }
    }

    #[test]
    fn an_award_needs_the_floor_and_a_tie_goes_to_fewer_messages() {
        let rows = [
            // All of them at night, and one short of the floor.
            person("user1", 39, 39),
            // Half at night each, on different totals.
            person("user2", 40, 20),
            person("user3", 80, 40),
        ];
        let out = awards(&rows, 40);
        // The other three find nobody above zero, and an award of nothing is
        // not given.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "Night owl");
        assert_eq!(
            out[0].key, "user2",
            "an even score goes to the smaller total"
        );
        assert_eq!(out[0].value, "50%");

        assert!(awards(&rows, 81).is_empty(), "nobody clears the floor");
    }

    #[test]
    fn a_streak_is_consecutive_days_and_one_day_is_not_one() {
        let lone = export(vec![
            msg(1, "user1", at(1, 10, 0), None),
            msg(2, "user2", at(3, 10, 0), None),
        ]);
        assert!(streaks(&lone, &People::new(&lone)).is_none());

        let export = export(vec![
            msg(1, "user1", at(1, 10, 0), None),
            msg(2, "user1", at(2, 10, 0), None),
            msg(3, "user1", at(2, 11, 0), None),
            msg(4, "user1", at(3, 10, 0), None),
            // The same length later: the first to reach it keeps it.
            msg(5, "user2", at(5, 10, 0), None),
            msg(6, "user2", at(6, 10, 0), None),
            msg(7, "user2", at(7, 10, 0), None),
        ]);
        let run = streaks(&export, &People::new(&export)).expect("a streak");
        assert_eq!((run.name.as_str(), run.days), ("user1", 3));
        assert_eq!(run.ended, "2025-01-03");
    }

    #[test]
    fn each_topic_counts_its_own_messages_and_an_empty_one_has_no_average() {
        let mut second = msg(3, "user2", at(1, 11, 0), None);
        second.topic = 1;
        let mut export = export(vec![
            msg(1, "user1", at(1, 10, 0), None),
            msg(2, "user1", at(2, 10, 0), None),
            second,
        ]);
        for index in [1, 2] {
            let mut topic = export.topics[0].clone();
            topic.index = index;
            topic.name = format!("topic {index}");
            export.topics.push(topic);
        }
        let out = topics(&export, &People::new(&export));
        let counts: Vec<usize> = out.iter().map(|t| t.messages).collect();
        assert_eq!(counts, [2, 1, 0]);
        assert_eq!(out[0].active_days, 2);
        assert_eq!(out[0].top, "user1");
        assert_eq!(out[1].first, "2025-01-01");
        assert_eq!(out[2].avg_words, None);
    }

    #[test]
    fn the_most_replied_to_message_is_named_by_its_id() {
        let export = export(vec![
            msg(1, "user1", at(1, 10, 0), None),
            msg(2, "user2", at(1, 10, 1), Some(1)),
            msg(3, "user3", at(1, 10, 2), Some(1)),
            msg(4, "user3", at(1, 10, 3), Some(2)),
        ]);
        let people = People::new(&export);
        let activity = crate::activity::compute(&export, &people);
        let out = superlatives(&export, &people, &activity, &export.by_id());
        let record = out
            .iter()
            .find(|s| s.title == "Most replied to")
            .expect("a record");
        assert_eq!(record.id, Some(1));
        assert_eq!(record.value, Figure::Count(2));
        assert_eq!(record.who, "user1");
    }
}
