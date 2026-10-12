//! Two string questions the metrics ask constantly, and neither is a crate.
//!
//! Ported from `analyser/read.py`. The ranges and the joiner list are that
//! file's, unchanged — they were chosen there and the reason is worth keeping:
//! the report *counts* emoji, it never has to name or classify one, so the
//! alternative is shipping a Unicode table that goes stale.

/// Code-point ranges that carry pictographic emoji.
///
/// Written as ranges rather than a regex so each block can be named and the
/// source stays plain ASCII.
const EMOJI_RANGES: &[(u32, u32)] = &[
    (0x00A9, 0x00A9),   // copyright
    (0x00AE, 0x00AE),   // registered
    (0x203C, 0x2049),   // double exclamation, interrobang
    (0x2122, 0x2122),   // trade mark
    (0x2194, 0x21AA),   // arrows
    (0x231A, 0x231B),   // watch, hourglass
    (0x23E9, 0x23FA),   // media controls
    (0x24C2, 0x24C2),   // circled M
    (0x25AA, 0x25FE),   // geometric shapes
    (0x2600, 0x27BF),   // misc symbols and dingbats
    (0x2934, 0x2935),   // curved arrows
    (0x2B00, 0x2BFF),   // misc symbols and arrows
    (0x3030, 0x3030),   // wavy dash
    (0x303D, 0x303D),   // part alternation mark
    (0x3297, 0x3299),   // circled ideographs
    (0x1F000, 0x1FAFF), // the pictographic planes
];

/// Attach to a preceding emoji rather than standing alone.
const EMOJI_JOINERS: &[(u32, u32)] = &[
    (0x1F3FB, 0x1F3FF), // skin tone modifiers
    (0xFE0E, 0xFE0F),   // variation selectors
    (0x200D, 0x200D),   // zero-width joiner
    (0x20E3, 0x20E3),   // combining enclosing keycap
    (0x1F1E6, 0x1F1FF), // regional indicators (flags)
];

fn in_ranges(ranges: &[(u32, u32)], code: u32) -> bool {
    ranges
        .iter()
        .any(|&(low, high)| low <= code && code <= high)
}

/// Every emoji sequence in `text`, joined runs kept whole.
///
/// A skin-toned or ZWJ-joined sequence is one emoji to the person who typed
/// it, so it counts once rather than once per code point.
pub fn emoji_runs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut run = String::new();
    let mut has_base = false;
    for ch in text.chars() {
        let code = ch as u32;
        if in_ranges(EMOJI_RANGES, code) {
            run.push(ch);
            has_base = true;
        } else if !run.is_empty() && in_ranges(EMOJI_JOINERS, code) {
            run.push(ch);
        } else {
            if !run.is_empty() && has_base {
                out.push(std::mem::take(&mut run));
            }
            run.clear();
            has_base = false;
        }
    }
    if !run.is_empty() && has_base {
        out.push(run);
    }
    out
}

/// The host of `url`, lowercased, `www.` dropped.
///
/// Deliberately not a public-suffix lookup: that needs a list that expires,
/// and the report ranks hosts, it does not bill anyone for them.
pub fn domain_of(url: &str) -> String {
    let host = url.trim();
    // Python: split("://", 1)[-1] — everything after the first separator, or
    // the whole string when there is none.
    let host = host.split_once("://").map_or(host, |(_, rest)| rest);
    let host = host.split('/').next().unwrap_or("");
    let host = host.split('?').next().unwrap_or("");
    let host = host.split('#').next().unwrap_or("");
    // `[-1]` on an unbounded split: everything after the *last* '@'.
    let host = host.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    let host = host.trim().to_lowercase();
    match host.strip_prefix("www.") {
        Some(rest) => rest.to_string(),
        None => host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_joined_sequence_counts_once() {
        // Skin tone and ZWJ both attach to the emoji before them.
        assert_eq!(emoji_runs("\u{1F44D}\u{1F3FD}").len(), 1);
        assert_eq!(emoji_runs("\u{1F468}\u{200D}\u{1F4BB}").len(), 1);
        // Two separated by a space are two.
        assert_eq!(emoji_runs("\u{1F600} \u{1F600}").len(), 2);
    }

    #[test]
    fn a_joiner_with_no_emoji_before_it_is_not_an_emoji() {
        // A bare variation selector or ZWJ in ordinary text must not open a
        // run — `has_base` is what stops it.
        assert!(emoji_runs("a\u{200D}b").is_empty());
        assert!(emoji_runs("plain text").is_empty());
    }

    #[test]
    fn a_domain_loses_its_scheme_port_path_and_www() {
        assert_eq!(domain_of("https://www.Example.com/a/b?c#d"), "example.com");
        assert_eq!(domain_of("example.com:8443"), "example.com");
        assert_eq!(domain_of("http://user@host.tld/x"), "host.tld");
        // No scheme at all still yields a host.
        assert_eq!(domain_of("t.me/somechannel"), "t.me");
    }
}
