//! What the messages were made of: length, attachments, emoji, links.

use crate::stats::number;
use std::fmt::Write as _;
use tga_stats::{human_bytes, Stats};

use super::*;
use crate::charts::{self, esc, thousands};

pub fn said(stats: &Stats) -> String {
    let content = &stats.content;
    let kinds = &content.media_kinds;
    let saved = content.media_saved;
    let total = content.media_messages;
    let skipped = total - saved;
    let messages = content.messages as i64;

    let attachments = if kinds.is_empty() {
        String::new()
    } else {
        let rows: Vec<charts::HBar> = kinds
            .iter()
            .map(|k| {
                charts::HBar::new(
                    k.label.clone(),
                    k.sent as f64,
                    format!("{} \u{b7} {}", thousands(k.sent), human_bytes(k.bytes)),
                )
            })
            .collect();
        let top = &kinds[0];
        let skip_note = if skipped > 0 {
            format!(
                " {} of those attachments were over the export&#8217;s size limit, \
                 so they are described in the archive but their bytes are not on \
                 disk. The totals are what was shared, which includes them.",
                thousands(skipped)
            )
        } else {
            String::new()
        };
        let kind_rows: Vec<Vec<String>> = kinds
            .iter()
            .map(|k| {
                vec![
                    esc(&k.label),
                    thousands(k.sent),
                    thousands(k.saved),
                    esc(&human_bytes(k.bytes)),
                ]
            })
            .collect();
        panel(
            "Attachments",
            &format!(
                "Mostly <b>{}</b>: {} of {} attachments.",
                esc(&top.label.to_lowercase()),
                thousands(top.sent),
                thousands(total)
            ),
            &format!(
                "Messages carrying each kind of attachment, and the bytes they \
                 came to.{skip_note}"
            ),
            &(charts::hbars(&rows, Some(0))
                + &data_view(
                    "Attachments, as numbers",
                    &table(
                        &[
                            ("Kind", false),
                            ("Sent", true),
                            ("Saved", true),
                            ("Bytes", true),
                        ],
                        &kind_rows,
                    ),
                )),
        )
    };

    let lengths = &content.lengths;
    let length_labels: Vec<String> = lengths.iter().map(|b| b.label.clone()).collect();
    let length_values: Vec<i64> = lengths.iter().map(|b| b.n).collect();
    let shown = |n: i64| format!("{} messages", thousands(n));
    let length_chart = charts::vcols(&charts::Cols {
        labels: &length_labels,
        values: &length_values,
        shown: &shown,
        every: 1,
        id: "",
    });
    let no_text = lengths
        .iter()
        .find(|bucket| bucket.label == "no text")
        .map(|bucket| bucket.n)
        .unwrap_or(0);
    let length = panel(
        "Message length",
        &format!(
            "A typical message is <b>{} words</b>; {} carry no text at all.",
            number(content.mean_words),
            share(no_text, messages)
        ),
        "In words. The first bucket is messages with no text at all &#8212; a \
         photo, a sticker, a voice note.",
        &length_chart,
    );

    let symbol_rows = |items: &[tga_stats::Count], title: &str| -> String {
        if items.is_empty() {
            return String::new();
        }
        let cells: String = items
            .iter()
            .take(10)
            .map(|item| {
                format!(
                    "<li><b>{}</b><span>{}</span></li>",
                    esc(&item.label),
                    thousands(item.n)
                )
            })
            .collect();
        panel(
            title,
            "",
            "",
            &format!("<ul class=\"figures symbols\">{cells}</ul>"),
        )
    };
    let emoji_rows = symbol_rows(&content.emoji, "Most-used emoji");
    let sticker_rows = symbol_rows(&content.stickers, "Favourite stickers");
    let symbols = if emoji_rows.is_empty() && sticker_rows.is_empty() {
        String::new()
    } else {
        format!("<div class=\"cols2\">{emoji_rows}{sticker_rows}</div>")
    };

    let domains = &content.domains;
    let links = if domains.is_empty() {
        String::new()
    } else {
        let rows: Vec<charts::HBar> = domains
            .iter()
            .take(14)
            .map(|host| charts::HBar::new(host.label.clone(), host.n as f64, thousands(host.n)))
            .collect();
        panel(
            "Where the links went",
            &format!(
                "<b>{}</b> leads, {} of {} links.",
                esc(&domains[0].label),
                thousands(domains[0].n),
                thousands(content.links_total)
            ),
            "Links by the site they point at, with the leading www. removed.",
            &charts::hbars(&rows, Some(0)),
        )
    };

    let mut tag_cols = String::new();
    let hashtags = &content.hashtags;
    if !hashtags.is_empty() {
        let rows: Vec<Vec<String>> = hashtags
            .iter()
            .take(12)
            .map(|tag| vec![format!("#{}", esc(&tag.label)), thousands(tag.n)])
            .collect();
        let _ = write!(
            tag_cols,
            "{}",
            panel(
                "Hashtags",
                "",
                "",
                &table(&[("Tag", false), ("Uses", true)], &rows)
            )
        );
    }
    let mentions = &content.mentions;
    if !mentions.is_empty() {
        let rows: Vec<Vec<String>> = mentions
            .iter()
            .take(12)
            .map(|who| vec![esc(&who.label), thousands(who.n)])
            .collect();
        let _ = write!(
            tag_cols,
            "{}",
            panel(
                "Most mentioned",
                "",
                "",
                &table(&[("Handle", false), ("Mentions", true)], &rows)
            )
        );
    }
    let tags = if tag_cols.is_empty() {
        String::new()
    } else {
        format!("<div class=\"cols2\">{tag_cols}</div>")
    };

    let sources = &content.forward_sources;
    let forwards = if sources.is_empty() {
        String::new()
    } else {
        let rows: Vec<Vec<String>> = sources
            .iter()
            .take(12)
            .map(|src| vec![esc(&src.label), thousands(src.n)])
            .collect();
        panel(
            "Forwarded from",
            &format!(
                "{} messages were forwarded in; <b>{}</b> is the commonest source.",
                thousands(content.forwards),
                esc(&sources[0].label)
            ),
            "",
            &table(&[("Source", false), ("Forwards", true)], &rows),
        )
    };

    let stat_line = figures(&[
        (thousands(content.total_words), "words"),
        (number(content.mean_words), "words in a typical message"),
        (thousands(total), "messages with an attachment"),
        (esc(&human_bytes(content.media_bytes)), "shared"),
        (thousands(content.links_total), "links"),
        (thousands(content.edited), "edited afterwards"),
    ]);

    format!(
        "{}{stat_line}<div class=\"cols2\">{length}{attachments}</div>{}{symbols}\
         {links}{tags}{forwards}</section>",
        head(
            "What was said",
            &format!("{} messages", thousands(messages)),
            "said"
        ),
        super::voice(stats),
    )
}
