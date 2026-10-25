//! A synthetic export for the unit tests: no corpus, and nobody real in it.
//!
//! Shared rather than written per module, because every metric here walks the
//! same `Msg` and a builder copied six times is six places for a new field to
//! be forgotten. The real archives are `save.bat baseline`'s; these are the
//! boundaries it can only notice by accident.

use chrono::{NaiveDate, NaiveDateTime};
use tga_read::{Export, Member, Msg, Reaction, Topic};

/// A moment in January 2025.
pub fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    on(2025, 1, day, hour, minute)
}

pub fn on(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap()
}

/// One message of one word, sent by `who` under the name `who`.
///
/// `unix` is the wall clock read as UTC, so a duration is the difference the
/// test wrote down.
pub fn msg(id: i64, who: &str, when: NaiveDateTime, reply_to: Option<i64>) -> Msg {
    Msg {
        id,
        topic: 0,
        when,
        unix: when.and_utc().timestamp(),
        service: false,
        action: String::new(),
        sender: who.to_string(),
        name: who.to_string(),
        text: "hello".into(),
        chars: 5,
        words: 1,
        reply_to,
        forward_from: String::new(),
        forward_id: String::new(),
        media: String::new(),
        file: String::new(),
        file_name: String::new(),
        file_size: 0,
        media_saved: false,
        sticker_emoji: String::new(),
        emoji: Vec::new(),
        domains: Vec::new(),
        links: Vec::new(),
        mentions: Vec::new(),
        hashtags: Vec::new(),
        reactions: Vec::new(),
        edited: false,
        members: Vec::new(),
        grouped: None,
        poll: None,
        duration: 0,
        title: String::new(),
        pinned: None,
    }
}

/// `count` reactions on a message, the given reactors named.
pub fn reaction(count: i64, named: &[&str]) -> Reaction {
    Reaction {
        emoji: "\u{1F44D}".into(),
        count,
        named: named.iter().map(|k| k.to_string()).collect(),
    }
}

pub fn member(key: &str, name: &str) -> Member {
    Member {
        key: key.into(),
        name: name.into(),
        role: "member".into(),
        joined: None,
        username: String::new(),
        bot: false,
    }
}

/// One General topic holding `msgs`, which are sorted the way the reader
/// sorts them.
pub fn export(mut msgs: Vec<Msg>) -> Export {
    msgs.sort_by(|a, b| a.unix.cmp(&b.unix).then_with(|| a.id.cmp(&b.id)));
    Export {
        name: "test".into(),
        topics: vec![Topic {
            index: 0,
            name: "General".into(),
            folder: Default::default(),
            chat_id: None,
            chat_type: String::new(),
            created: None,
            messages: msgs.len(),
            root: None,
        }],
        msgs,
        ..Default::default()
    }
}
