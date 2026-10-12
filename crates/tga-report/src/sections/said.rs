//! What the messages were made of: length, attachments, emoji, links.

use crate::stats::{human_bytes, number};
use std::fmt::Write as _;
use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

pub fn said(stats: &Stats) -> String {
    let content = &stats.content;
    let kinds = &content.media_kinds;
    let media_chart = if kinds.is_empty() {
        String::new()
    } else {
        let rows: Vec<charts::BarRow> = kinds
            .iter()
            .map(|k| {
                (
                    k.label.clone(),
                    k.sent as f64,
                    format!("{}  {}", thousands(k.sent), human_bytes(k.bytes)),
                )
            })
            .collect();
        charts::bars_h(&rows, WIDTH, 22.0, 140.0, 64.0)
    };

    let lengths = &content.lengths;
    // Narrower than the measure on purpose. Nine buckets across the full page
    // gives 124px bands, and a column capped at 24px inside one of those reads
    // as nine unrelated marks rather than as a distribution.
    let length_chart = charts::columns(
        &lengths.iter().map(|b| b.label.clone()).collect::<Vec<_>>(),
        &lengths.iter().map(|b| b.n).collect::<Vec<_>>(),
        WIDTH * 0.56,
        210.0,
        1,
        " messages",
        None,
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
        format!("<h3>{title}</h3><ul class=\"figures\">{cells}</ul>")
    };
    let emoji_rows = symbol_rows(&content.emoji, "Most-used emoji");
    let sticker_rows = symbol_rows(&content.stickers, "Favourite stickers");

    let domains = &content.domains;
    let links = if domains.is_empty() {
        String::new()
    } else {
        let rows: Vec<charts::BarRow> = domains
            .iter()
            .take(14)
            .map(|host| (host.label.clone(), host.n as f64, thousands(host.n)))
            .collect();
        format!(
            "<h3>Where the links went</h3>{}",
            charts::bars_h(&rows, WIDTH, 22.0, 210.0, 64.0)
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
            "<div><h3>Hashtags</h3>{}</div>",
            table(&[("Tag", false), ("Uses", true)], &rows)
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
            "<div><h3>Most mentioned</h3>{}</div>",
            table(&[("Handle", false), ("Mentions", true)], &rows)
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
        format!(
            "<h3>Forwarded from</h3>{}",
            table(&[("Source", false), ("Forwards", true)], &rows)
        )
    };

    let saved = content.media_saved;
    let total = content.media_messages;
    let skipped = total - saved;
    let messages = content.messages as i64;
    let stat_line = figures(&[
        (thousands(content.total_words), "words"),
        (number(content.mean_words), "words in a typical message"),
        (thousands(total), "messages with an attachment"),
        (esc(&human_bytes(content.media_bytes)), "shared"),
        (thousands(content.links_total), "links"),
        (thousands(content.edited), "edited afterwards"),
    ]);

    let skip_note = if skipped > 0 {
        format!(
            "<p class=\"note\">{} of those attachments were over the \
             export&#8217;s size limit, so they are described in the archive but their \
             bytes are not on disk. The totals above are what was shared, which \
             includes them.</p>",
            thousands(skipped)
        )
    } else {
        String::new()
    };

    let no_text = lengths
        .iter()
        .find(|bucket| bucket.label == "no text")
        .map(|bucket| bucket.n)
        .unwrap_or(0);

    let attachments = if media_chart.is_empty() {
        String::new()
    } else {
        format!("<h3>Attachments</h3>{media_chart}{skip_note}")
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

    format!(
        "{}{stat_line}<h3>Message length</h3><div class=\"split\"><div>{length_chart}</div>\
         <aside><p class=\"caption\">In words. The first bucket is messages with no \
         text at all &#8212; a photo, a sticker, a voice note, which is {} of \
         everything sent.</p></aside></div>{attachments}{emoji_rows}{sticker_rows}\
         {links}{tags}{forwards}{}</section>",
        head(
            "What was said",
            &format!("{} messages", thousands(messages)),
            "said"
        ),
        pct(no_text as f64 / messages.max(1) as f64),
        data_view(
            "Attachments, as numbers",
            &table(
                &[
                    ("Kind", false),
                    ("Sent", true),
                    ("Saved", true),
                    ("Bytes", true)
                ],
                &kind_rows
            )
        )
    )
}
