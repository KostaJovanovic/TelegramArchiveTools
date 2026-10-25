//! What the group did to itself, read off its service messages as a log.
//!
//! Renames, pins, topics opened and renamed, calls, a migration from an older
//! group: each is a dated fact about the group rather than something anybody
//! said, and the rest of the report counts none of them.

use std::collections::HashMap;

use tga_read::{Export, Msg};
use tga_stats::{Count, History, HistoryEvent};

use crate::extras::{snippet, JOIN_ACTIONS, LEAVE_ACTIONS};
use crate::identity::People;
use crate::util::{stamp_minutes, Counter};

/// How many entries the log carries; past this the newest are kept.
const HISTORY_SHOWN: usize = 400;

/// `index` is [`Export::by_id`], built once by the caller.
pub fn compute(export: &Export, people: &People, index: &HashMap<i64, usize>) -> History {
    let service: Vec<&Msg> = export.msgs.iter().filter(|m| m.service).collect();

    let mut kinds: Counter<String> = Counter::new();
    let mut events: Vec<HistoryEvent> = Vec::new();
    for msg in &service {
        // An action Desktop wrote no name for is still an event; it is filed
        // under the name the report gives an unknown one.
        let action = msg.action.clone();
        kinds.bump(action.clone());
        let membership =
            JOIN_ACTIONS.contains(&action.as_str()) || LEAVE_ACTIONS.contains(&action.as_str());
        if membership {
            continue;
        }
        let key = people.key_of(msg);
        let target = msg
            .pinned
            .and_then(|id| index.get(&id))
            .map(|&at| &export.msgs[at]);
        events.push(HistoryEvent {
            id: msg.id,
            topic: msg.topic,
            date: stamp_minutes(&msg.when),
            action,
            name: people.name_of(&key),
            key,
            title: msg.title.clone(),
            pinned: msg.pinned,
            pinned_who: target
                .map(|t| people.name_of(&people.key_of(t)))
                .unwrap_or_default(),
            pinned_text: target.map(snippet).unwrap_or_default(),
        });
    }

    let total = events.len();
    if events.len() > HISTORY_SHOWN {
        events.drain(..events.len() - HISTORY_SHOWN);
    }
    History {
        shown: events.len(),
        events,
        total,
        kinds: kinds
            .most_common(None)
            .into_iter()
            .map(|(label, n)| Count::new(label, n))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, export, msg};

    fn service(id: i64, who: &str, day: u32, action: &str) -> Msg {
        let mut m = msg(id, who, at(day, 12, 0), None);
        m.service = true;
        m.text = String::new();
        m.action = action.into();
        m
    }

    #[test]
    fn the_log_skips_comings_and_goings_but_counts_them() {
        let mut renamed = service(2, "user1", 2, "edit_group_title");
        renamed.title = "New name".into();
        let mut pin = service(4, "user2", 4, "pin_message");
        pin.pinned = Some(1);
        let export = export(vec![
            msg(1, "user1", at(1, 9, 0), None),
            renamed,
            service(3, "user3", 3, "join_group_by_link"),
            pin,
        ]);
        let people = People::new(&export);
        let out = compute(&export, &people, &export.by_id());
        assert_eq!(out.total, 2);
        assert_eq!(out.events[0].title, "New name");
        // The pin names the message it pinned, its author and its text.
        assert_eq!(out.events[1].pinned, Some(1));
        assert_eq!(out.events[1].pinned_who, "user1");
        assert_eq!(out.events[1].pinned_text, "hello");
        assert!(out.kinds.contains(&Count::new("join_group_by_link", 1)));
        assert_eq!(out.kinds.iter().map(|k| k.n).sum::<i64>(), 3);
    }

    #[test]
    fn a_pin_of_a_message_outside_the_export_keeps_only_its_id() {
        let mut pin = service(2, "user1", 2, "pin_message");
        pin.pinned = Some(999);
        let export = export(vec![msg(1, "user1", at(1, 9, 0), None), pin]);
        let people = People::new(&export);
        let out = compute(&export, &people, &export.by_id());
        assert_eq!(out.events[0].pinned, Some(999));
        assert_eq!(out.events[0].pinned_text, "");
    }
}
