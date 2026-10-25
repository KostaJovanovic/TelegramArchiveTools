//! Every poll the chat ran: what was asked, by whom, and how many answered.
//!
//! A poll is the one place a group's decisions are written down as data
//! rather than as conversation, so it gets a list of its own rather than a
//! count in the media table.

use tga_read::{Export, Msg};
use tga_stats::{Count, PollRow, Polls, Starter};

use crate::conversation::median;
use crate::identity::People;
use crate::util::{month_axis, month_of, stamp_minutes, Counter};

/// How many polls get a row. A group that polls daily would otherwise carry
/// thousands of them into a file meant to stay under a megabyte; past this
/// the newest are kept and the report says how many were left out.
const POLLS_SHOWN: usize = 300;

/// How many people are named as having put polls to the group.
const ASKERS_SHOWN: usize = 10;

pub fn compute(export: &Export, people: &People) -> Polls {
    let said: Vec<&Msg> = export.said().collect();
    let polls: Vec<&Msg> = said.iter().copied().filter(|m| m.poll.is_some()).collect();

    let mut askers: Counter<String> = Counter::new();
    let mut per_month: Counter<String> = Counter::new();
    let mut voters: Vec<i64> = Vec::with_capacity(polls.len());
    let mut rows: Vec<PollRow> = Vec::with_capacity(polls.len());
    for msg in &polls {
        let poll = msg.poll.as_ref().expect("filtered on it");
        let key = people.key_of(msg);
        let forwarded = !msg.forward_from.is_empty() || !msg.forward_id.is_empty();
        // A forwarded poll was put to some other group; the person who
        // forwarded it did not ask it.
        if !forwarded && !key.is_empty() {
            askers.bump(key.clone());
        }
        per_month.bump(month_of(&msg.when));
        voters.push(poll.total_voters);
        rows.push(PollRow {
            id: msg.id,
            topic: msg.topic,
            date: stamp_minutes(&msg.when),
            name: people.name_of(&key),
            key,
            question: poll.question.clone(),
            closed: poll.closed,
            forwarded,
            voters: poll.total_voters,
            answers: poll
                .answers
                .iter()
                .map(|a| Count::new(a.text.clone(), a.voters))
                .collect(),
        });
    }

    let total = rows.len();
    let closed = rows.iter().filter(|r| r.closed).count();
    let forwarded = rows.iter().filter(|r| r.forwarded).count();
    if rows.len() > POLLS_SHOWN {
        rows.drain(..rows.len() - POLLS_SHOWN);
    }

    Polls {
        shown: rows.len(),
        rows,
        total,
        votes: voters.iter().sum(),
        median_voters: median(&voters).round() as i64,
        closed,
        forwarded,
        per_month: month_axis(&said)
            .into_iter()
            .map(|m| {
                let n = per_month.get(&m);
                Count::new(m, n)
            })
            .collect(),
        askers: askers
            .most_common(Some(ASKERS_SHOWN))
            .into_iter()
            .map(|(key, count)| Starter {
                name: people.name_of(&key),
                key,
                count,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, export, msg, on};
    use tga_read::{Poll, PollAnswer};

    fn poll(id: i64, who: &str, when: chrono::NaiveDateTime, voters: &[i64]) -> Msg {
        let mut m = msg(id, who, when, None);
        m.text = String::new();
        m.poll = Some(Poll {
            question: format!("question {id}"),
            closed: false,
            total_voters: voters.iter().sum(),
            answers: voters
                .iter()
                .enumerate()
                .map(|(i, v)| PollAnswer {
                    text: format!("answer {i}"),
                    voters: *v,
                })
                .collect(),
        });
        m
    }

    #[test]
    fn every_poll_is_listed_with_its_tallies_and_its_month() {
        let export = export(vec![
            msg(1, "user1", at(1, 9, 0), None),
            poll(2, "user1", at(2, 9, 0), &[3, 1]),
            poll(3, "user2", on(2025, 3, 2, 9, 0), &[5, 0, 2]),
        ]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert_eq!(out.total, 2);
        assert_eq!(out.votes, 11);
        assert_eq!(out.median_voters, 6);
        assert_eq!(out.rows[1].answers[0], Count::new("answer 0", 5));
        // February had no poll and stays on the axis.
        assert_eq!(
            out.per_month,
            [
                Count::new("2025-01", 1),
                Count::new("2025-02", 0),
                Count::new("2025-03", 1)
            ]
        );
    }

    #[test]
    fn a_forwarded_poll_is_not_the_forwarder_asking() {
        let mut fwd = poll(2, "user2", at(2, 9, 0), &[1]);
        fwd.forward_from = "Another Group".into();
        let export = export(vec![poll(1, "user1", at(1, 9, 0), &[1]), fwd]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert_eq!(out.forwarded, 1);
        assert_eq!(out.askers.len(), 1);
        assert_eq!(out.askers[0].key, "user1");
    }

    #[test]
    fn past_the_cap_the_newest_polls_are_the_ones_kept() {
        let msgs: Vec<Msg> = (0..(POLLS_SHOWN as i64 + 5))
            .map(|i| {
                poll(
                    i + 1,
                    "user1",
                    at(1, 0, 0) + chrono::Duration::minutes(i),
                    &[1],
                )
            })
            .collect();
        let export = export(msgs);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert_eq!(out.total, POLLS_SHOWN + 5);
        assert_eq!(out.shown, POLLS_SHOWN);
        assert_eq!(out.rows[0].id, 6, "the five oldest were left out");
    }

    #[test]
    fn a_chat_with_no_polls_has_an_empty_list() {
        let export = export(vec![msg(1, "user1", at(1, 9, 0), None)]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert_eq!(out.total, 0);
        assert!(out.rows.is_empty());
    }
}
