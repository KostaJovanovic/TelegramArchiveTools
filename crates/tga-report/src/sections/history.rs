//! What the group did to itself, as a dated log.

use tga_stats::{HistoryEvent, Stats};

use super::*;
use crate::charts::{esc, thousands};

/// What an action did, in words, for the log.
///
/// Every name here is Desktop's. An action outside the list is shown by its
/// own name with the underscores taken out, which is ugly and true — the same
/// trade `label_for` makes for a peer key.
fn what(event: &HistoryEvent) -> String {
    let title = || format!("<b>{}</b>", esc(&event.title));
    match event.action.as_str() {
        "edit_group_title" => format!("renamed the group to {}", title()),
        "migrate_from_group" if !event.title.is_empty() => {
            format!("moved the group here from {}", title())
        }
        "migrate_from_group" => "moved the group here from an older one".to_string(),
        "migrate_to_supergroup" => "moved the group to a supergroup".to_string(),
        "topic_created" => format!("opened the topic {}", title()),
        "topic_edit" if !event.title.is_empty() => format!("renamed a topic to {}", title()),
        "topic_edit" => "changed a topic&#8217;s icon or state".to_string(),
        "pin_message" => pinned(event),
        "edit_group_photo" => "changed the group photo".to_string(),
        "delete_group_photo" => "removed the group photo".to_string(),
        "create_group" => "created the group".to_string(),
        "create_channel" => "created the channel".to_string(),
        "poll_append_answer" => "added an answer to a poll".to_string(),
        "poll_delete_answer" => "removed an answer from a poll".to_string(),
        "group_call" => "started a group call".to_string(),
        "group_call_scheduled" => "scheduled a group call".to_string(),
        "phone_call" => "made a call".to_string(),
        "clear_history" => "cleared the history".to_string(),
        "set_messages_ttl" => "changed when messages delete themselves".to_string(),
        "edit_chat_theme" => "changed the chat theme".to_string(),
        "set_chat_wallpaper" => "changed the wallpaper".to_string(),
        "boost_apply" => "boosted the group".to_string(),
        "" => "did something the export does not name".to_string(),
        other => esc(&other.replace('_', " ")),
    }
}

fn pinned(event: &HistoryEvent) -> String {
    match event.pinned {
        Some(_) if !event.pinned_text.is_empty() => format!(
            "pinned a message by {}: <span class=\"muted\">{}</span>",
            esc(&event.pinned_who),
            esc(&event.pinned_text)
        ),
        Some(id) => format!(
            "pinned message {} <span class=\"muted\">(not in this export)</span>",
            thousands(id)
        ),
        None => "pinned a message".to_string(),
    }
}

/// The figures line: the commonest kinds of change, comings and goings left
/// to the section that charts them.
fn tally(stats: &Stats) -> String {
    let Some(h) = &stats.history else {
        return String::new();
    };
    let count = |names: &[&str]| -> i64 {
        h.kinds
            .iter()
            .filter(|k| names.contains(&k.label.as_str()))
            .map(|k| k.n)
            .sum()
    };
    let items: Vec<(i64, &str)> = vec![
        (count(&["pin_message"]), "pins"),
        (count(&["edit_group_title"]), "renames"),
        (count(&["topic_created"]), "topics opened"),
        (count(&["topic_edit"]), "topic edits"),
        (
            count(&["edit_group_photo", "delete_group_photo"]),
            "photo changes",
        ),
        (
            count(&["group_call", "phone_call", "group_call_scheduled"]),
            "calls",
        ),
    ];
    let shown: Vec<(String, &str)> = items
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| (thousands(n), label))
        .collect();
    if shown.is_empty() {
        String::new()
    } else {
        figures(&shown)
    }
}

pub fn history(stats: &Stats) -> String {
    let Some(h) = &stats.history else {
        return String::new();
    };
    if h.events.is_empty() {
        return String::new();
    }
    let rows: Vec<Vec<String>> = h
        .events
        .iter()
        .map(|e| vec![esc(&pretty_when(&e.date)), short(&e.name, 24), what(e)])
        .collect();
    let cut = if h.shown < h.total {
        format!(
            " The newest {} of {} are listed.",
            thousands(h.shown as i64),
            thousands(h.total as i64)
        )
    } else {
        String::new()
    };
    let log = panel(
        "Every change",
        "",
        &format!(
            "Oldest first: the service messages Telegram posts when somebody renames \
             the group, pins a message, opens a topic or starts a call. Joins and \
             removals are left to Coming and going.{cut}"
        ),
        &table(&[("When", false), ("Who", false), ("What", false)], &rows),
    );
    format!(
        "{}{}{log}</section>",
        head("Group history", &thousands(h.total as i64), "history"),
        tally(stats)
    )
}
