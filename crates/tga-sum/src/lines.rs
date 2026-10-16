//! A message as the model reads it: one line.
//!
//! ```text
//! == 2024-12-02 Mon ==
//! #18234 09:14 Ana: …
//! #18240 09:20 Marko (re #18234): … [photo] (4 reactions)
//! #18251 10:02 [Media] Ana: [document "Zapisnik 01.12.docx", dated 2024-12-01 (filename): …]
//! ```
//!
//! The shape ReVision's extraction prompts were tuned on, so Gemma has read a
//! great deal of it already. The topic tag appears only when the lines mix
//! topics; a single topic's name is in the prompt instead.

use chrono::NaiveDate;
use tga_notes::Row;

/// A message's text past this is cut, with an ellipsis: ReVision's
/// `MAX_MSG_CHARS`. Longer than the digest's 400 because a summary is exactly
/// where a long statement or a pasted announcement matters; still short
/// enough that one copy-paste cannot eat a part.
pub const TEXT_CHARS: usize = 1500;

/// The text read off a photo past this is cut: ReVision's `OCR_CAP`.
pub const PHOTO_CHARS: usize = 1500;

/// One message, rendered. `topic` names its topic when lines mix topics.
pub fn line(row: &Row, topic: Option<&str>) -> String {
    let time = row.t.get(11..16).unwrap_or(&row.t);
    let mut s = format!("#{} {time}", row.id);
    if let Some(name) = topic {
        s.push_str(&format!(" [{name}]"));
    }
    let who = if row.who.is_empty() { "?" } else { &row.who };
    s.push(' ');
    s.push_str(who);
    if let Some(action) = &row.service {
        // A join, a pin, a rename: who did it and the action's name is the
        // whole of what it says.
        s.push_str(&format!(" · {}", action.replace('_', " ")));
        let text = flat(&row.text, TEXT_CHARS);
        if !text.is_empty() {
            s.push_str(&format!(": {text}"));
        }
        return s;
    }
    if let Some(re) = row.re {
        s.push_str(&format!(" (re #{re})"));
    }
    s.push(':');
    let text = flat(&row.text, TEXT_CHARS);
    if !text.is_empty() {
        s.push(' ');
        s.push_str(&text);
    }
    if let Some(doc) = &row.doc {
        s.push_str(&format!(" [document \"{}\"", doc.name));
        if !doc.date.is_empty() {
            s.push_str(&format!(", dated {} ({})", doc.date, doc.date_src));
        }
        // **A document is read whole** (2026-10-16, the account holder's ask;
        // it was cut at 1,500 characters). What bounds it is the reader's own
        // cap, `tga_docs::MAX_CHARS`. A message is never split, so a long one
        // is a part of its own — larger than `PART_TOKENS`, still well inside
        // the context.
        let inside = flat(&doc.text, usize::MAX);
        if !inside.is_empty() {
            s.push_str(&format!(": {inside}"));
        }
        s.push(']');
    } else if !row.photo_text.is_empty() {
        s.push_str(&format!(
            " [photo with text: {}]",
            flat(&row.photo_text, PHOTO_CHARS)
        ));
    } else if !row.media.is_empty() {
        let kind = row.media.replace('_', " ");
        // **Every shared file by its name**: a spreadsheet, a deck, an archive
        // or a recording nothing here reads the inside of is still what it is
        // called, which is most of what a summary needs to say it was shared.
        // A sticker's or a GIF's name is the client's, and says nothing.
        if row.file.is_empty() || matches!(row.media.as_str(), "sticker" | "animation") {
            s.push_str(&format!(" [{kind}]"));
        } else {
            s.push_str(&format!(" [{kind} \"{}\"]", flat(&row.file, 200)));
        }
    }
    if let Some(n) = row.reactions.filter(|&n| n > 0) {
        s.push_str(&format!(" ({n} reaction{})", if n == 1 { "" } else { "s" }));
    }
    s
}

/// A message that says nothing a summary could use: a sticker or a GIF with
/// no text, or only "haha", "ok", "da", "ne", "lol", "+1" and the like with
/// nothing attached. ReVision's `normalize.TRIVIAL`, except that any run of
/// "ha" counts ("hahaha" slipped through its pattern).
pub fn trivial(row: &Row) -> bool {
    if row.service.is_some() || row.doc.is_some() || !row.photo_text.is_empty() {
        return false;
    }
    let text = row.text.trim();
    if text.is_empty() {
        return matches!(row.media.as_str(), "sticker" | "animation");
    }
    if !row.media.is_empty() {
        return false;
    }
    let core: String = text
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '+')
        .to_lowercase();
    // A laugh: "ha", "haha", "hahaaa", "xaxa", "хаха".
    let laugh = |h: char, a: char| {
        core.chars().count() >= 2
            && core.starts_with(h)
            && core.chars().nth(1) == Some(a)
            && core.chars().all(|c| c == h || c == a)
    };
    const WORDS: [&str; 13] = [
        "ok", "okej", "oki", "da", "ne", "jes", "aha", "e", "lol", "+1", "ок", "да", "не",
    ];
    laugh('h', 'a') || laugh('x', 'a') || laugh('х', 'а') || WORDS.contains(&core.as_str())
}

/// The line that opens a day.
pub fn day_line(day: NaiveDate) -> String {
    format!("== {} ==", day.format("%Y-%m-%d %a"))
}

/// The day a row is on, from its wall-clock `t`.
pub fn day_of(row: &Row) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(row.t.get(..10)?, "%Y-%m-%d").ok()
}

/// Minutes since midnight, for finding the quiet gaps inside a day.
pub fn minute_of(row: &Row) -> u32 {
    let hm = row.t.get(11..16).unwrap_or("00:00");
    let (h, m) = hm.split_once(':').unwrap_or(("0", "0"));
    h.parse::<u32>().unwrap_or(0) * 60 + m.parse::<u32>().unwrap_or(0)
}

/// Whitespace collapsed to single spaces, cut at `max` characters.
pub fn flat(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let head: String = flat.chars().take(max - 1).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tga_notes::Attachment;

    fn row(id: i64, t: &str, who: &str, text: &str) -> Row {
        Row {
            id,
            t: t.into(),
            who: who.into(),
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_reply_with_media_and_reactions() {
        let mut r = row(7, "2024-12-02 09:20", "Marko", "da,\n  slažem se");
        r.re = Some(3);
        r.media = "photo".into();
        r.reactions = Some(4);
        assert_eq!(
            line(&r, None),
            "#7 09:20 Marko (re #3): da, slažem se [photo] (4 reactions)"
        );
    }

    #[test]
    fn a_topic_is_tagged_only_when_asked() {
        let r = row(1, "2024-12-02 09:14", "Ana", "hi");
        assert_eq!(line(&r, Some("Media")), "#1 09:14 [Media] Ana: hi");
    }

    #[test]
    fn a_document_carries_its_own_date_and_its_text() {
        let mut r = row(9, "2024-12-02 10:02", "Ana", "");
        r.media = "file".into();
        r.doc = Some(Attachment {
            name: "Zapisnik 01.12.docx".into(),
            date: "2024-12-01".into(),
            date_src: "filename".into(),
            text: "Prisutni: …".into(),
            ..Default::default()
        });
        assert_eq!(
            line(&r, None),
            "#9 10:02 Ana: [document \"Zapisnik 01.12.docx\", dated 2024-12-01 (filename): Prisutni: …]"
        );
    }

    #[test]
    fn a_document_is_read_whole() {
        let mut r = row(9, "2024-12-02 10:02", "Ana", "");
        r.doc = Some(Attachment {
            name: "Statut.docx".into(),
            text: "č".repeat(TEXT_CHARS * 4),
            ..Default::default()
        });
        let out = line(&r, None);
        assert!(out.contains(&"č".repeat(TEXT_CHARS * 4)));
        assert!(!out.contains('…'));
    }

    #[test]
    fn a_file_nothing_can_read_is_still_named() {
        let mut r = row(5, "2024-12-02 11:00", "Ana", "evo budžeta");
        r.media = "document".into();
        r.file = "Budžet 2025.xlsx".into();
        assert_eq!(
            line(&r, None),
            "#5 11:00 Ana: evo budžeta [document \"Budžet 2025.xlsx\"]"
        );
        r.media = "video_file".into();
        r.file = "skupština.mp4".into();
        assert_eq!(
            line(&r, None),
            "#5 11:00 Ana: evo budžeta [video file \"skupština.mp4\"]"
        );
        // A sticker's file name is the client's and says nothing.
        r.text.clear();
        r.media = "sticker".into();
        r.file = "sticker.webp".into();
        assert_eq!(line(&r, None), "#5 11:00 Ana: [sticker]");
    }

    #[test]
    fn a_photo_with_text_carries_it_and_one_without_is_just_a_photo() {
        let mut r = row(4, "2024-12-02 10:30", "Ana", "vidi");
        r.media = "photo".into();
        assert_eq!(line(&r, None), "#4 10:30 Ana: vidi [photo]");
        r.photo_text = "Protest u petak\nu podne".into();
        assert_eq!(
            line(&r, None),
            "#4 10:30 Ana: vidi [photo with text: Protest u petak u podne]"
        );
    }

    #[test]
    fn haha_ok_and_a_lone_sticker_are_trivial_and_nothing_else_is() {
        for t in [
            "haha", "Hahaha!!", "ok", "OK.", "da", "ne", "lol", "+1", "хаха", "xaxa", "e",
        ] {
            assert!(trivial(&row(1, "2024-12-02 10:00", "Ana", t)), "{t}");
        }
        for t in ["ok, vidimo se u 5", "ne znam", "hej", "nema", "haos"] {
            assert!(!trivial(&row(1, "2024-12-02 10:00", "Ana", t)), "{t}");
        }
        let mut sticker = row(1, "2024-12-02 10:00", "Ana", "");
        sticker.media = "sticker".into();
        assert!(trivial(&sticker));
        // "ok" under a photo is about the photo.
        let mut photo = row(1, "2024-12-02 10:00", "Ana", "ok");
        photo.media = "photo".into();
        assert!(!trivial(&photo));
    }

    #[test]
    fn a_service_message_names_its_action() {
        let mut r = row(2, "2024-12-02 11:00", "Ana", "");
        r.service = Some("pin_message".into());
        assert_eq!(line(&r, None), "#2 11:00 Ana · pin message");
    }

    #[test]
    fn a_long_text_is_cut_with_an_ellipsis() {
        let long = "a".repeat(TEXT_CHARS + 50);
        let out = flat(&long, TEXT_CHARS);
        assert_eq!(out.chars().count(), TEXT_CHARS);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn a_day_and_a_minute_come_from_the_wall_clock() {
        let r = row(1, "2024-12-02 09:14", "Ana", "");
        assert_eq!(day_of(&r), NaiveDate::from_ymd_opt(2024, 12, 2));
        assert_eq!(minute_of(&r), 9 * 60 + 14);
        assert_eq!(day_line(day_of(&r).unwrap()), "== 2024-12-02 Mon ==");
    }
}
