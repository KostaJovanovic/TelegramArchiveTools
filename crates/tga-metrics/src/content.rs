//! What the messages were made of.
//!
//! Two figures about media are easy to conflate and are kept apart:
//!
//! * **sent** counts every message that carried an attachment. It is a fact
//!   about the conversation.
//! * **saved** counts the ones whose bytes are on disk. It is a fact about the
//!   *export* — a file over the size limit is recorded, described and skipped.
//!
//! Bytes are the sizes Telegram reported, so the total covers skipped files
//! too and answers "how much was shared", not "how much did this folder cost".

use std::collections::HashMap;

use tga_read::Export;
use tga_stats::{Content, Count, MediaKind};

use crate::identity::People;
use crate::util::{round1, title_case, Counter};

/// Message lengths, in words. Open-ended at the top; the last bucket is
/// everything above the previous edge.
const LENGTH_EDGES: &[usize] = &[1, 3, 6, 11, 21, 51, 101];

/// What to call each `media_type` in prose.
fn media_label(kind: &str) -> String {
    match kind {
        "photo" => "Photos",
        "video_file" => "Videos",
        "animation" => "GIFs",
        "sticker" => "Stickers",
        "voice_message" => "Voice notes",
        "video_message" => "Video notes",
        "audio_file" => "Audio",
        "document" => "Files",
        other => return title_case(&other.replace('_', " ")),
    }
    .to_string()
}

fn bucket_names() -> Vec<String> {
    let mut names = vec!["no text".to_string()];
    for pair in LENGTH_EDGES.windows(2) {
        names.push(format!("{}-{}", pair[0], pair[1] - 1));
    }
    names.push(format!("{}+", LENGTH_EDGES[LENGTH_EDGES.len() - 1]));
    names
}

fn bucket(words: usize) -> String {
    if words == 0 {
        return "no text".to_string();
    }
    for pair in LENGTH_EDGES.windows(2) {
        if pair[0] <= words && words < pair[1] {
            return format!("{}-{}", pair[0], pair[1] - 1);
        }
    }
    format!("{}+", LENGTH_EDGES[LENGTH_EDGES.len() - 1])
}

/// A peer key the report can turn into a name.
fn is_peer_key(key: &str) -> bool {
    key.starts_with("user") || key.starts_with("chat") || key.starts_with("channel")
}

pub fn compute(export: &Export, people: &People) -> Content {
    let msgs: Vec<_> = export.said().collect();

    let mut media: Counter<String> = Counter::new();
    let mut media_saved: Counter<String> = Counter::new();
    let mut media_bytes: Counter<String> = Counter::new();
    let mut lengths: Counter<String> = Counter::new();
    let mut emoji: Counter<String> = Counter::new();
    let mut emoji_by_person: Vec<String> = Vec::new();
    let mut emoji_person_counts: HashMap<String, Counter<String>> = HashMap::new();
    let mut stickers: Counter<String> = Counter::new();
    let mut domains: Counter<String> = Counter::new();
    let mut hashtags: Counter<String> = Counter::new();
    let mut mentions: Counter<String> = Counter::new();
    let mut forward_sources: Counter<String> = Counter::new();
    let mut with_text = 0i64;
    let mut edited = 0i64;

    for msg in &msgs {
        lengths.bump(bucket(msg.words));
        if msg.words > 0 {
            with_text += 1;
        }
        if msg.edited {
            edited += 1;
        }
        if !msg.media.is_empty() {
            media.bump(msg.media.clone());
            media_bytes.add(msg.media.clone(), msg.file_size);
            if msg.media_saved {
                media_saved.bump(msg.media.clone());
            }
        }
        if !msg.sticker_emoji.is_empty() {
            stickers.bump(msg.sticker_emoji.clone());
        }
        if !msg.emoji.is_empty() {
            // Once per message, not once per emoji in it: it is a fresh string
            // each time.
            let key = people.key_of(msg);
            for run in &msg.emoji {
                emoji.bump(run.clone());
                if !key.is_empty() {
                    emoji_person_counts
                        .entry(key.clone())
                        .or_insert_with(|| {
                            emoji_by_person.push(key.clone());
                            Counter::new()
                        })
                        .bump(run.clone());
                }
            }
        }
        for host in &msg.domains {
            if !host.is_empty() {
                domains.bump(host.clone());
            }
        }
        for tag in &msg.hashtags {
            hashtags.bump(tag.clone());
        }
        for who in &msg.mentions {
            mentions.bump(who.clone());
        }
        let source = if msg.forward_from.is_empty() {
            &msg.forward_id
        } else {
            &msg.forward_from
        };
        if !source.is_empty() {
            forward_sources.bump(source.clone());
        }
    }

    let kinds: Vec<MediaKind> = media
        .most_common(None)
        .into_iter()
        .map(|(kind, count)| MediaKind {
            label: media_label(&kind),
            sent: count,
            saved: media_saved.get(&kind),
            bytes: media_bytes.get(&kind),
            kind,
        })
        .collect();

    let ordered_lengths: Vec<Count> = bucket_names()
        .into_iter()
        .map(|name| {
            let n = lengths.get(&name);
            Count::new(name, n)
        })
        .collect();

    let total_words: i64 = msgs.iter().map(|m| m.words as i64).sum();
    let total_chars: i64 = msgs.iter().map(|m| m.chars as i64).sum();

    let emoji_person: std::collections::BTreeMap<String, Vec<Count>> = emoji_by_person
        .iter()
        .map(|key| {
            let top = emoji_person_counts[key]
                .most_common(Some(3))
                .into_iter()
                .map(|(e, n)| Count::new(e, n))
                .collect();
            (key.clone(), top)
        })
        .collect();

    // A @mention of somebody with no public username arrives as a peer id,
    // which is unreadable. Resolve it to the name if we know them.
    let mention_rows: Vec<Count> = mentions
        .most_common(Some(20))
        .into_iter()
        .map(|(who, n)| {
            let person = people.get(&who);
            let shown = if is_peer_key(&who) && person.name != who {
                person.label().to_string()
            } else {
                format!("@{who}")
            };
            Count::new(shown, n)
        })
        .collect();

    let forward_rows: Vec<Count> = forward_sources
        .most_common(Some(15))
        .into_iter()
        .map(|(src, n)| {
            let shown = if is_peer_key(&src) {
                people.get(&src).label().to_string()
            } else {
                src.clone()
            };
            Count::new(shown, n)
        })
        .collect();

    let pairs = |items: Vec<(String, i64)>| -> Vec<Count> {
        items.into_iter().map(|(k, n)| Count::new(k, n)).collect()
    };

    Content {
        messages: msgs.len(),
        with_text,
        edited,
        total_words,
        total_chars,
        mean_words: if with_text > 0 {
            round1(total_words as f64 / with_text as f64)
        } else {
            0.0
        },
        lengths: ordered_lengths,
        media_kinds: kinds,
        media_messages: media.total(),
        media_saved: media_saved.total(),
        media_bytes: media_bytes.total(),
        emoji: pairs(emoji.most_common(Some(40))),
        emoji_total: emoji.total(),
        emoji_by_person: emoji_person,
        stickers: pairs(stickers.most_common(Some(20))),
        domains: pairs(domains.most_common(Some(25))),
        links_total: domains.total(),
        hashtags: pairs(hashtags.most_common(Some(20))),
        mentions: mention_rows,
        forwards: forward_sources.total(),
        forward_sources: forward_rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, export, msg};

    #[test]
    fn every_emoji_in_a_message_is_counted_and_credited_to_its_sender() {
        let laugh = "\u{1F602}".to_string();
        let thumb = "\u{1F44D}".to_string();
        let mut one = msg(1, "user1", at(1, 10, 0), None);
        one.emoji = vec![laugh.clone(), laugh.clone(), thumb.clone()];
        let mut two = msg(2, "user2", at(1, 10, 1), None);
        two.emoji = vec![laugh.clone()];
        let export = export(vec![one, two, msg(3, "user1", at(1, 10, 2), None)]);

        let out = compute(&export, &People::new(&export));
        assert_eq!(out.emoji_total, 4);
        assert_eq!(
            out.emoji,
            [Count::new(laugh.clone(), 3), Count::new(thumb.clone(), 1)]
        );
        assert_eq!(
            out.emoji_by_person["user1"],
            [Count::new(laugh.clone(), 2), Count::new(thumb, 1)]
        );
        assert_eq!(out.emoji_by_person["user2"], [Count::new(laugh, 1)]);
    }

    #[test]
    fn an_attachment_is_sent_whether_or_not_its_bytes_were_saved() {
        let mut saved = msg(1, "user1", at(1, 10, 0), None);
        saved.media = "photo".into();
        saved.media_saved = true;
        saved.file_size = 100;
        let mut skipped = msg(2, "user1", at(1, 10, 1), None);
        skipped.media = "photo".into();
        skipped.file_size = 50;
        let export = export(vec![saved, skipped]);

        let out = compute(&export, &People::new(&export));
        assert_eq!(
            (out.media_messages, out.media_saved, out.media_bytes),
            (2, 1, 150)
        );
        let photos = &out.media_kinds[0];
        assert_eq!(
            (photos.label.as_str(), photos.sent, photos.saved),
            ("Photos", 2, 1)
        );
    }
}
