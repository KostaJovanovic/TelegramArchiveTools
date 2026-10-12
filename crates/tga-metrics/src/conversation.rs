//! The shape of the conversation: who answers whom, and how fast.
//!
//! Three things worth knowing before reading any number out of here.
//!
//! **Latency is measured on epoch seconds**, never on the wall clock. A reply
//! sent twenty minutes after its parent across a DST boundary is twenty
//! minutes, and subtracting the naive local timestamps would call it eighty.
//!
//! **A reply is directed at a person, not at a message.** The edge is
//! *author of the reply -> author of the parent*, and a reply to your own
//! message is dropped from the graph: everyone talks to themselves and
//! counting it makes the loudest person their own closest correspondent.
//!
//! **A session is a burst of talk, not a day.** Consecutive messages within
//! [`SESSION_GAP`] belong to the same session, per topic, because two topics
//! running at once are two conversations and merging them invents replies that
//! never happened.

use std::collections::HashMap;

use tga_read::{Export, Msg};
use tga_stats::{Conversation, Edge, Latency, Session, Starter};

use crate::identity::People;
use crate::util::{round1, stamp_minutes, Counter};

/// A pause longer than this ends the session. Half an hour is the common
/// choice for chat and it is what the report states, so a reader can judge it.
pub const SESSION_GAP: i64 = 30 * 60;

/// A reply this long after its parent is somebody returning to a thread days
/// later, not a response time. It still counts as a reply; it is left out of
/// the latency figures, which the report says.
pub const LATENCY_CAP: i64 = 24 * 60 * 60;

/// The mean of the two middle values on an even count.
///
/// Shared with [`crate::dynamics`] rather than copied there: two definitions of
/// a median is two things to keep agreeing.
pub(crate) fn median(values: &[i64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let n = ordered.len();
    if n % 2 == 1 {
        ordered[n / 2] as f64
    } else {
        (ordered[n / 2 - 1] as f64 + ordered[n / 2] as f64) / 2.0
    }
}

/// Half to even, returning an integer.
///
/// Not `f64::round`, which goes half away from zero. The difference lands in
/// `percentile` below, where the index can fall on exactly .5, and it moves the
/// p90 by one place in the sorted list.
fn round_half_even(x: f64) -> i64 {
    let floor = x.floor();
    if (x - floor - 0.5).abs() < f64::EPSILON {
        let low = floor as i64;
        if low % 2 == 0 {
            low
        } else {
            low + 1
        }
    } else {
        x.round() as i64
    }
}

fn percentile(values: &[i64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let last = ordered.len() - 1;
    let index = (round_half_even(fraction * last as f64).max(0) as usize).min(last);
    ordered[index] as f64
}

fn session(
    stream: &[&Msg],
    topic: usize,
    people: &People,
    starters: &mut Counter<String>,
) -> Session {
    let mut keys: std::collections::HashSet<String> = stream
        .iter()
        .map(|m| people.key_of(m))
        .filter(|k| !k.is_empty())
        .collect();
    keys.remove("");
    let first = people.key_of(stream[0]);
    if !first.is_empty() {
        starters.bump(first.clone());
    }
    Session {
        topic,
        start: stamp_minutes(&stream[0].when),
        messages: stream.len(),
        voices: keys.len(),
        minutes: round1((stream[stream.len() - 1].unix - stream[0].unix) as f64 / 60.0),
        opened_by: if first.is_empty() {
            String::new()
        } else {
            people.get(&first).label().to_string()
        },
    }
}

pub fn compute(export: &Export, people: &People) -> Conversation {
    let msgs: Vec<&Msg> = export.said().collect();
    let index = export.by_id();

    let mut edges: Counter<(String, String)> = Counter::new();
    let mut latencies: Vec<i64> = Vec::new();
    let mut by_person_order: Vec<String> = Vec::new();
    let mut by_person: HashMap<String, Vec<i64>> = HashMap::new();
    let mut replies = 0i64;
    let mut orphans = 0i64;
    let mut self_replies = 0i64;

    for msg in &msgs {
        let Some(reply_to) = msg.reply_to else {
            continue;
        };
        replies += 1;
        let Some(&at) = index.get(&reply_to) else {
            orphans += 1;
            continue;
        };
        let parent = &export.msgs[at];
        let (source, target) = (people.key_of(msg), people.key_of(parent));
        if source.is_empty() || target.is_empty() {
            continue;
        }
        if source == target {
            self_replies += 1;
        } else {
            edges.bump((source.clone(), target.clone()));
        }
        let gap = msg.unix - parent.unix;
        if (0..=LATENCY_CAP).contains(&gap) {
            latencies.push(gap);
            by_person
                .entry(source.clone())
                .or_insert_with(|| {
                    by_person_order.push(source.clone());
                    Vec::new()
                })
                .push(gap);
        }
    }

    // Reactions are the other half of "who pays attention to whom", and the
    // only half that needs no words. Same caveat as everywhere: only the
    // reactors Telegram named are in it.
    let mut reaction_edges: Counter<(String, String)> = Counter::new();
    for msg in &msgs {
        let target = people.key_of(msg);
        if target.is_empty() {
            continue;
        }
        for reaction in &msg.reactions {
            for voter in &reaction.named {
                if !voter.is_empty() && *voter != target {
                    reaction_edges.bump((voter.clone(), target.clone()));
                }
            }
        }
    }

    // Sessions, per topic. Topic order is first-appearance, and it decides
    // `session_longest` on a tie: the first of equal maxima wins.
    let mut topic_order: Vec<usize> = Vec::new();
    let mut per_topic: HashMap<usize, Vec<&Msg>> = HashMap::new();
    for msg in &msgs {
        per_topic
            .entry(msg.topic)
            .or_insert_with(|| {
                topic_order.push(msg.topic);
                Vec::new()
            })
            .push(msg);
    }

    let mut starters: Counter<String> = Counter::new();
    let mut sessions: Vec<Session> = Vec::new();
    for topic in &topic_order {
        let stream = &per_topic[topic];
        let mut current: Vec<&Msg> = Vec::new();
        for msg in stream {
            if !current.is_empty() && msg.unix - current[current.len() - 1].unix > SESSION_GAP {
                sessions.push(session(&current, *topic, people, &mut starters));
                current.clear();
            }
            current.push(msg);
        }
        if !current.is_empty() {
            sessions.push(session(&current, *topic, people, &mut starters));
        }
    }

    let lengths: Vec<i64> = sessions.iter().map(|s| s.messages as i64).collect();
    let voices: Vec<i64> = sessions.iter().map(|s| s.voices as i64).collect();

    let mut slow: Vec<Latency> = by_person_order
        .iter()
        .filter(|key| by_person[*key].len() >= 5)
        .map(|key| {
            let values = &by_person[key];
            Latency {
                key: key.clone(),
                name: people.get(key).label().to_string(),
                replies: values.len(),
                median: median(values).trunc() as i64,
            }
        })
        .collect();
    // Stable, so people with the same median stay in first-reply order.
    slow.sort_by_key(|r| r.median);

    let fastest: Vec<Latency> = slow.iter().take(12).cloned().collect();
    let slowest: Vec<Latency> = slow
        .iter()
        .skip(slow.len().saturating_sub(12))
        .rev()
        .cloned()
        .collect();

    let edge_rows = |counter: &Counter<(String, String)>| -> Vec<Edge> {
        counter
            .most_common(Some(400))
            .into_iter()
            .map(|((a, b), n)| Edge {
                from_name: people.get(&a).label().to_string(),
                to_name: people.get(&b).label().to_string(),
                from: a,
                to: b,
                count: n,
            })
            .collect()
    };

    let longest = sessions
        .iter()
        .enumerate()
        // `max_by` keeps the last of equal maxima and the first is the one to
        // name, so the index breaks the tie the other way.
        .max_by(|a, b| a.1.messages.cmp(&b.1.messages).then_with(|| b.0.cmp(&a.0)))
        .map(|(_, s)| s.clone());

    Conversation {
        replies,
        reply_share: if msgs.is_empty() {
            0.0
        } else {
            replies as f64 / msgs.len() as f64
        },
        orphan_replies: orphans,
        self_replies,
        latency_counted: latencies.len(),
        latency_median: if latencies.is_empty() {
            0
        } else {
            median(&latencies).trunc() as i64
        },
        latency_p90: if latencies.is_empty() {
            0
        } else {
            percentile(&latencies, 0.9).trunc() as i64
        },
        latency_cap: LATENCY_CAP,
        fastest,
        slowest,
        edges: edge_rows(&edges),
        reaction_edges: edge_rows(&reaction_edges),
        sessions: sessions.len(),
        session_gap: SESSION_GAP,
        session_median_messages: if lengths.is_empty() {
            0
        } else {
            median(&lengths).trunc() as i64
        },
        session_median_voices: if voices.is_empty() {
            0
        } else {
            median(&voices).trunc() as i64
        },
        session_longest: longest,
        starters: starters
            .most_common(Some(12))
            .into_iter()
            .map(|(k, n)| Starter {
                name: people.get(&k).label().to_string(),
                key: k,
                count: n,
            })
            .collect(),
    }
}
