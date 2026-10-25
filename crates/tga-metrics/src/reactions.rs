//! Reactions: which ones, how often, and each person's own.
//!
//! The people table already counts reactions received and given. What it
//! cannot say is *which*: a person whose messages draw 👍 and one whose draw
//! 😂 are doing different things in a group, and so are the months before and
//! after everybody stopped reacting at all.
//!
//! Given is only ever what Telegram named. See `tga_stats::Reactions`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use tga_read::{Export, Msg};
use tga_stats::{Count, ReactionRow, Reactions};

use crate::identity::People;
use crate::util::{month_axis, Counter, MonthIndex};

/// The label every custom emoji is pooled under.
pub const CUSTOM: &str = "custom emoji";

/// How many people get a row, from each end: the most reacted to, and the
/// readiest to react.
const PEOPLE_SHOWN: usize = 25;

/// How many of a person's favourites are named.
const TOP: usize = 3;

/// A custom emoji is written as its document id, and a reaction the reader
/// could not identify as `?`. Neither is anything a reader can look at.
fn is_custom(emoji: &str) -> bool {
    emoji == "?" || (!emoji.is_empty() && emoji.bytes().all(|b| b.is_ascii_digit()))
}

pub fn compute(export: &Export, people: &People) -> Reactions {
    let said: Vec<&Msg> = export.said().collect();
    let months = month_axis(&said);
    let at = MonthIndex::new(&months);

    let mut out = Reactions {
        messages: said.len() as i64,
        per_month: vec![0; months.len()],
        reacted_per_month: vec![0; months.len()],
        messages_per_month: vec![0; months.len()],
        ..Default::default()
    };
    // Borrowed from the messages throughout: every message is counted once
    // in `sent`, and an owned key per message was most of what this cost.
    type Key<'a> = Cow<'a, str>;
    let mut kinds: Counter<Key> = Counter::new();
    let mut sent: Counter<Key> = Counter::new();
    let mut received: Counter<Key> = Counter::new();
    let mut given: Counter<Key> = Counter::new();
    let mut received_kinds: HashMap<Key, Counter<Key>> = HashMap::new();
    let mut given_kinds: HashMap<Key, Counter<Key>> = HashMap::new();

    for msg in &said {
        let author = people.key_ref(msg);
        sent.bump(author.clone());
        let slot = at.of(&msg.when);
        if let Some(slot) = slot {
            out.messages_per_month[slot] += 1;
        }
        if msg.reactions.is_empty() {
            continue;
        }
        let mut any = false;
        for reaction in &msg.reactions {
            if reaction.count <= 0 {
                continue;
            }
            any = true;
            let label: Key = if is_custom(&reaction.emoji) {
                out.custom += reaction.count;
                Cow::Borrowed(CUSTOM)
            } else {
                Cow::Borrowed(reaction.emoji.as_str())
            };
            out.total += reaction.count;
            kinds.add(label.clone(), reaction.count);
            if let Some(slot) = slot {
                out.per_month[slot] += reaction.count;
            }
            if !author.is_empty() {
                received.add(author.clone(), reaction.count);
                received_kinds
                    .entry(author.clone())
                    .or_default()
                    .add(label.clone(), reaction.count);
            }
            for voter in &reaction.named {
                out.named += 1;
                given.bump(Cow::Borrowed(voter.as_str()));
                given_kinds
                    .entry(Cow::Borrowed(voter.as_str()))
                    .or_default()
                    .bump(label.clone());
            }
        }
        if any {
            out.reacted += 1;
            if let Some(slot) = slot {
                out.reacted_per_month[slot] += 1;
            }
        }
    }

    let top = |counter: Option<&Counter<Key>>| -> Vec<Count> {
        counter
            .map(|c| {
                c.most_common(Some(TOP))
                    .into_iter()
                    .map(|(label, n)| Count::new(label, n))
                    .collect()
            })
            .unwrap_or_default()
    };

    // The most reacted to, then anyone among the readiest reactors who is not
    // already there, so somebody who reacts to everything and posts nothing
    // still has a row.
    let mut keys: Vec<Key> = Vec::new();
    let mut seen: HashSet<Key> = HashSet::new();
    for (key, _) in received
        .most_common(Some(PEOPLE_SHOWN))
        .into_iter()
        .chain(given.most_common(Some(PEOPLE_SHOWN)))
    {
        if !key.is_empty() && seen.insert(key.clone()) {
            keys.push(key);
        }
    }
    let mut rows: Vec<ReactionRow> = keys
        .into_iter()
        .map(|key| ReactionRow {
            name: people.name_of(&key),
            messages: sent.get(&key),
            received: received.get(&key),
            received_top: top(received_kinds.get(&key)),
            given: given.get(&key),
            given_top: top(given_kinds.get(&key)),
            key: key.into_owned(),
        })
        .collect();
    // Stable, so a tie keeps the order the two rankings produced.
    rows.sort_by_key(|r| std::cmp::Reverse(r.received));

    out.people = rows;
    out.kinds = kinds
        .most_common(None)
        .into_iter()
        .map(|(label, n)| Count::new(label, n))
        .collect();
    out.months = months;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, export, msg, on};
    use tga_read::Reaction;

    fn reacted(
        id: i64,
        who: &str,
        when: chrono::NaiveDateTime,
        emoji: &str,
        count: i64,
        named: &[&str],
    ) -> Msg {
        let mut m = msg(id, who, when, None);
        m.reactions.push(Reaction {
            emoji: emoji.into(),
            count,
            named: named.iter().map(|k| k.to_string()).collect(),
        });
        m
    }

    #[test]
    fn each_reaction_is_counted_by_kind_by_month_and_by_person() {
        let mut both = reacted(2, "user1", at(2, 9, 0), "\u{1F602}", 2, &["user2"]);
        both.reactions.push(Reaction {
            emoji: "5368324170671202286".into(),
            count: 1,
            named: vec![],
        });
        let export = export(vec![
            reacted(1, "user1", at(1, 9, 0), "\u{1F44D}", 3, &["user2", "user3"]),
            both,
            msg(3, "user2", on(2025, 2, 1, 9, 0), None),
        ]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert_eq!(out.total, 6);
        assert_eq!(out.named, 3, "given is only what Telegram named");
        assert_eq!(out.custom, 1);
        assert_eq!((out.reacted, out.messages), (2, 3));
        assert_eq!(out.kinds[0], Count::new("\u{1F44D}", 3));
        assert!(out.kinds.contains(&Count::new(CUSTOM, 1)));
        assert_eq!(out.per_month, [6, 0]);
        assert_eq!(out.reacted_per_month, [2, 0]);
        assert_eq!(out.messages_per_month, [2, 1]);

        let ana = &out.people[0];
        assert_eq!((ana.key.as_str(), ana.received, ana.given), ("user1", 6, 0));
        assert_eq!(ana.received_top[0], Count::new("\u{1F44D}", 3));
        // user2 posted once and drew nothing, and still has a row as a giver.
        let bob = out.people.iter().find(|r| r.key == "user2").expect("a row");
        assert_eq!((bob.received, bob.given), (0, 2));
        assert_eq!(bob.messages, 1);
    }

    #[test]
    fn a_custom_emoji_is_pooled_and_a_real_one_is_not() {
        assert!(is_custom("5368324170671202286"));
        assert!(is_custom("?"));
        assert!(!is_custom("\u{2764}"));
        assert!(!is_custom(""));
    }
}
