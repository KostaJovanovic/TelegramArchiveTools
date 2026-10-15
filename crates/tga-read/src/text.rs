//! Two string questions the metrics ask constantly, and neither is a crate.
//!
//! Ported from `analyser/read.py`. The ranges and the joiner list are that
//! file's, unchanged — they were chosen there and the reason is worth keeping:
//! the report *counts* emoji, it never has to name or classify one, so the
//! alternative is shipping a Unicode table that goes stale. Where one emoji
//! ends and the next begins is *not* that file's; see [`emoji_runs`].

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

/// The regional indicators, which make a flag two at a time.
const REGIONAL: (u32, u32) = (0x1F1E6, 0x1F1FF);

const ZWJ: char = '\u{200D}';

/// Every emoji sequence in `text`, joined runs kept whole.
///
/// A skin-toned or ZWJ-joined sequence is one emoji to the person who typed
/// it, so it counts once rather than once per code point.
///
/// **Two emoji side by side are two.** The Python appended every pictograph
/// to the run it was in, so `😂😂😂` counted as one emoji whose name was three
/// of them, `👍❤` as a third kind that was neither, and two flags as one. The
/// top list then ranked the repeated string apart from the single one, in
/// exactly the chats that use emoji most. A pictograph now starts a new emoji
/// unless a zero-width joiner asked for it to join the last; regional
/// indicators pair off, two to a flag.
pub fn emoji_runs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut run = String::new();
    // The run ends on a joiner, so the next pictograph belongs to it.
    let mut joined = false;
    // The run is one regional indicator waiting for the other half of a flag.
    let mut half_flag = false;
    for ch in text.chars() {
        let code = ch as u32;
        if (REGIONAL.0..=REGIONAL.1).contains(&code) {
            if !half_flag && !joined && !run.is_empty() {
                out.push(std::mem::take(&mut run));
            }
            run.push(ch);
            half_flag = !half_flag;
            joined = false;
        } else if !run.is_empty() && in_ranges(EMOJI_JOINERS, code) {
            // A skin tone, a variation selector, a keycap or the joiner itself
            // modifies what came before it rather than standing alone.
            run.push(ch);
            joined = ch == ZWJ;
            half_flag = false;
        } else if in_ranges(EMOJI_RANGES, code) {
            if !joined && !run.is_empty() {
                out.push(std::mem::take(&mut run));
            }
            run.push(ch);
            joined = false;
            half_flag = false;
        } else {
            // A run only ever opens on a pictograph or an indicator, so
            // anything in it is an emoji.
            if !run.is_empty() {
                out.push(std::mem::take(&mut run));
            }
            joined = false;
            half_flag = false;
        }
    }
    if !run.is_empty() {
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
    fn two_emoji_side_by_side_are_two() {
        // Three laughs are three of the same emoji, not one emoji called
        // "three laughs".
        assert_eq!(
            emoji_runs("\u{1F602}\u{1F602}\u{1F602}"),
            ["\u{1F602}", "\u{1F602}", "\u{1F602}"]
        );
        // A thumbs-up and a heart are two kinds, not a third.
        assert_eq!(
            emoji_runs("\u{1F44D}\u{2764}\u{FE0F}"),
            ["\u{1F44D}", "\u{2764}\u{FE0F}"]
        );
        // A skin tone still belongs to the hand before it.
        assert_eq!(
            emoji_runs("\u{1F44D}\u{1F3FD}\u{1F44D}"),
            ["\u{1F44D}\u{1F3FD}", "\u{1F44D}"]
        );
    }

    #[test]
    fn flags_pair_off_two_indicators_at_a_time() {
        // Serbia then Ukraine: four indicators, two flags.
        assert_eq!(
            emoji_runs("\u{1F1F7}\u{1F1F8}\u{1F1FA}\u{1F1E6}"),
            ["\u{1F1F7}\u{1F1F8}", "\u{1F1FA}\u{1F1E6}"]
        );
        assert_eq!(
            emoji_runs("\u{1F600}\u{1F1F7}\u{1F1F8}\u{1F600}"),
            ["\u{1F600}", "\u{1F1F7}\u{1F1F8}", "\u{1F600}"]
        );
    }

    #[test]
    fn a_zwj_sequence_built_of_pictographs_is_still_one() {
        // The rainbow flag: white flag, VS16, ZWJ, rainbow.
        assert_eq!(
            emoji_runs("\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}\u{1F308}"),
            ["\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}", "\u{1F308}"]
        );
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
