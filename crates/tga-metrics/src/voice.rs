//! Voice notes and round video notes: who talks instead of typing, and how
//! much of it there is.
//!
//! A word count cannot see any of this. In a group where a few people send
//! two-minute voice notes, the people table ranks them as quiet and the
//! length chart files every one of those notes under "no text".

use std::collections::HashMap;

use tga_read::{Export, Msg};
use tga_stats::{Count, Voice, VoiceLongest, VoiceRow};

use crate::conversation::median;
use crate::identity::People;
use crate::util::{month_axis, month_of, stamp_minutes, Counter};

const VOICE: &str = "voice_message";
const VIDEO: &str = "video_message";

/// How many people get a row.
const PEOPLE_SHOWN: usize = 20;

pub fn compute(export: &Export, people: &People) -> Voice {
    let said: Vec<&Msg> = export.said().collect();

    let mut sent: Counter<String> = Counter::new();
    let mut seconds: Counter<String> = Counter::new();
    let mut voice: HashMap<String, i64> = HashMap::new();
    let mut video: HashMap<String, i64> = HashMap::new();
    let mut per_month: Counter<String> = Counter::new();
    let mut lengths: Vec<i64> = Vec::new();
    let mut out = Voice::default();
    let mut longest: Option<&Msg> = None;

    for msg in &said {
        let key = people.key_of(msg);
        sent.bump(key.clone());
        let kind = msg.media.as_str();
        if kind != VOICE && kind != VIDEO {
            continue;
        }
        let secs = msg.duration.max(0);
        if kind == VOICE {
            out.voice_notes += 1;
            out.voice_seconds += secs;
            *voice.entry(key.clone()).or_default() += 1;
        } else {
            out.video_notes += 1;
            out.video_seconds += secs;
            *video.entry(key.clone()).or_default() += 1;
        }
        if msg.duration <= 0 {
            out.undated += 1;
        } else {
            lengths.push(secs);
        }
        seconds.add(key, secs);
        per_month.add(month_of(&msg.when), secs);
        // The longest, and on a tie the earlier.
        if longest.is_none_or(|best| secs > best.duration.max(0)) {
            longest = Some(msg);
        }
    }

    out.median_seconds = median(&lengths).round() as i64;
    out.per_month = month_axis(&said)
        .into_iter()
        .map(|m| {
            let n = per_month.get(&m);
            Count::new(m, n)
        })
        .collect();
    out.people = seconds
        .most_common(None)
        .into_iter()
        .filter(|(key, _)| !key.is_empty())
        .map(|(key, secs)| {
            let notes = voice.get(&key).copied().unwrap_or(0);
            let videos = video.get(&key).copied().unwrap_or(0);
            let total = sent.get(&key);
            VoiceRow {
                name: people.name_of(&key),
                voice_notes: notes,
                video_notes: videos,
                seconds: secs,
                share: if total > 0 {
                    (notes + videos) as f64 / total as f64
                } else {
                    0.0
                },
                key,
            }
        })
        .take(PEOPLE_SHOWN)
        .collect();
    out.longest = longest.filter(|m| m.duration > 0).map(|m| VoiceLongest {
        id: m.id,
        topic: m.topic,
        date: stamp_minutes(&m.when),
        name: people.name_of(&people.key_of(m)),
        seconds: m.duration,
        kind: m.media.clone(),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, export, msg};

    fn note(id: i64, who: &str, day: u32, kind: &str, secs: i64) -> Msg {
        let mut m = msg(id, who, at(day, 10, 0), None);
        m.text = String::new();
        m.media = kind.into();
        m.duration = secs;
        m
    }

    #[test]
    fn notes_are_counted_and_timed_by_kind_and_by_person() {
        let export = export(vec![
            note(1, "user1", 1, VOICE, 30),
            note(2, "user1", 2, VIDEO, 12),
            note(3, "user2", 3, VOICE, 90),
            msg(4, "user1", at(4, 10, 0), None),
            // A voice note Desktop wrote no duration for.
            note(5, "user2", 5, VOICE, 0),
        ]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert_eq!((out.voice_notes, out.voice_seconds), (3, 120));
        assert_eq!((out.video_notes, out.video_seconds), (1, 12));
        assert_eq!(out.undated, 1);
        assert_eq!(out.median_seconds, 30);
        // user2 recorded longer, so ranks first.
        assert_eq!(out.people[0].key, "user2");
        assert_eq!(out.people[0].seconds, 90);
        assert_eq!(out.people[1].voice_notes, 1);
        assert_eq!(out.people[1].video_notes, 1);
        // Two of user1's three messages were notes.
        assert!((out.people[1].share - 2.0 / 3.0).abs() < 1e-9);
        let longest = out.longest.expect("a longest note");
        assert_eq!((longest.id, longest.seconds), (3, 90));
        assert_eq!(out.per_month, [Count::new("2025-01", 132)]);
    }

    #[test]
    fn a_chat_with_no_notes_has_none() {
        let export = export(vec![msg(1, "user1", at(1, 9, 0), None)]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert_eq!(out.voice_notes + out.video_notes, 0);
        assert!(out.people.is_empty());
        assert!(out.longest.is_none());
    }
}
