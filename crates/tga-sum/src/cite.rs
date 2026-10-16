//! Citations: `[#18234]` or `[#18234, #18301]`, checked against the input.
//!
//! A model will cite an id it never read — a neighbour's, a guess, one carried
//! over from a note it merged. A citation the reader can click and land on the
//! wrong message is worse than none, so every id not in the input is dropped,
//! and a bracket left empty goes with the space before it.
//!
//! A model will also cite everything it can: `[#1][#2][#3]…` ten deep after
//! one claim, which reads as a wall and points at nothing in particular.
//! Brackets with nothing but spaces between them are one citation, written
//! as one bracket, and it keeps the first [`MOST`] ids.
//!
//! A model will also drop the `#` the prompt asks for and write
//! `[304615, 304694]`. Unrecognised, those passed through unchecked and were
//! drawn as plain text, not links. A bracket of bare numbers is a citation
//! too, but **only when every number in it is an id the model was given**: a
//! year or a footnote in brackets is not one and stays as it was. Either way
//! it comes out written as `[#…]`.
//!
//! [`Filter`] does this on a stream: it holds back only what may still change
//! — an unfinished bracket, a citation the next one may join, and the spaces
//! before them — so the text still appears as it is written.

use std::collections::HashSet;

/// The most ids one citation keeps, as every prompt asks.
pub const MOST: usize = 3;

/// An unclosed bracket longer than this is not held back any more: a model
/// looping on ids would otherwise hold the whole answer.
const LONGEST: usize = 400;

/// `text` with every citation checked against `known`, joined and capped.
pub fn clean(text: &str, known: &HashSet<i64>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let (before, from) = rest.split_at(open);
        let Some((ids, len)) = group(from, known) else {
            out.push_str(before);
            out.push('[');
            rest = &from[1..];
            continue;
        };
        let mut kept: Vec<i64> = Vec::new();
        for id in ids {
            if known.contains(&id) && !kept.contains(&id) {
                kept.push(id);
            }
        }
        kept.truncate(MOST);
        if kept.is_empty() {
            out.push_str(before.trim_end_matches(' '));
        } else {
            let list: Vec<String> = kept.iter().map(|id| format!("#{id}")).collect();
            out.push_str(before);
            out.push('[');
            out.push_str(&list.join(", "));
            out.push(']');
        }
        rest = &from[len..];
    }
    out.push_str(rest);
    out
}

/// The citation at the start of `from`, which begins with `[`, and every one
/// that follows it after nothing but spaces: their ids in order, and how many
/// bytes of `from` they take. `None` when the first bracket is no citation.
fn group(from: &str, known: &HashSet<i64>) -> Option<(Vec<i64>, usize)> {
    let mut ids = Vec::new();
    let mut end = 0;
    loop {
        let at = &from[end..];
        let next = at.trim_start_matches(' ');
        let lead = at.len() - next.len();
        let Some(close) = next.strip_prefix('[').and_then(|_| next.find(']')) else {
            break;
        };
        let Some(got) = ids_in(&next[1..close], known) else {
            break;
        };
        ids.extend(got);
        end += lead + close + 1;
    }
    (!ids.is_empty()).then_some((ids, end))
}

/// The ids a bracket's inside cites, or `None` when it is not a citation:
/// `#1, #2` always, `1, 2` only when every one of them is `known`.
fn ids_in(inside: &str, known: &HashSet<i64>) -> Option<Vec<i64>> {
    let (ids, marked) = parse(inside)?;
    (marked || ids.iter().all(|id| known.contains(id))).then_some(ids)
}

/// The numbers in a bracket's inside, each with or without its `#`, and
/// whether every one had it. `None` when anything else is in there.
fn parse(inside: &str) -> Option<(Vec<i64>, bool)> {
    let mut marked = true;
    let ids: Option<Vec<i64>> = inside
        .split(',')
        .map(|part| {
            let part = part.trim();
            let digits = match part.strip_prefix('#') {
                Some(d) => d.trim(),
                None => {
                    marked = false;
                    part
                }
            };
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            digits.parse().ok()
        })
        .collect();
    ids.filter(|v| !v.is_empty()).map(|v| (v, marked))
}

/// Whether a bracket's inside so far could still turn out to be a citation.
fn may_cite(inside: &str) -> bool {
    inside
        .chars()
        .all(|c| c == '#' || c == ',' || c == ' ' || c.is_ascii_digit())
}

/// Every id cited in `text`, in order, repeats kept. Read from text that has
/// been through [`clean`], so a citation is the `[#…]` it wrote, and a bare
/// bracket left over is not one.
pub fn cited(text: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let from = &rest[open..];
        if let Some((ids, true)) = from.find(']').and_then(|close| parse(&from[1..close])) {
            out.extend(ids);
        }
        rest = &from[1..];
    }
    out
}

/// [`clean`], a piece at a time.
pub struct Filter<'a> {
    known: &'a HashSet<i64>,
    held: String,
}

impl<'a> Filter<'a> {
    pub fn new(known: &'a HashSet<i64>) -> Self {
        Filter {
            known,
            held: String::new(),
        }
    }

    /// Take a piece of the stream; return what can be shown now.
    pub fn push(&mut self, piece: &str) -> String {
        self.held.push_str(piece);
        let keep_from = self.unfinished();
        let ready: String = self.held.drain(..keep_from).collect();
        clean(&ready, self.known)
    }

    /// The stream has ended: everything left.
    pub fn finish(&mut self) -> String {
        let rest = std::mem::take(&mut self.held);
        clean(&rest, self.known)
    }

    /// Where the tail that may still change starts: an unclosed `[` that may
    /// yet be a citation, the citations before it — or at the very end — that
    /// a next one may join, and the spaces before all of them. Trailing spaces
    /// are held too, since a citation that loses every id takes them with it.
    fn unfinished(&self) -> usize {
        let h = &self.held;
        let mut at = h.trim_end_matches(' ').len();
        if let Some(open) = h[..at].rfind('[') {
            let inside = &h[open + 1..at];
            if !inside.contains(']') && inside.len() <= LONGEST && may_cite(inside) {
                at = open;
            }
        }
        loop {
            let before = h[..at].trim_end_matches(' ');
            let Some(body) = before.strip_suffix(']') else {
                break;
            };
            let Some(open) = body.rfind('[') else {
                break;
            };
            if ids_in(&body[open + 1..], self.known).is_none() {
                break;
            }
            at = open;
        }
        h[..at].trim_end_matches(' ').len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> HashSet<i64> {
        [10, 11, 12, 13, 14].into_iter().collect()
    }

    fn streamed(text: &str, size: usize, k: &HashSet<i64>) -> String {
        let mut f = Filter::new(k);
        let mut out = String::new();
        let chars: Vec<char> = text.chars().collect();
        for piece in chars.chunks(size) {
            out.push_str(&f.push(&piece.iter().collect::<String>()));
        }
        out.push_str(&f.finish());
        out
    }

    #[test]
    fn an_unknown_id_is_dropped_and_an_empty_bracket_with_its_space() {
        let k = known();
        assert_eq!(clean("A [#10, #99]. B [#98].", &k), "A [#10]. B.");
        assert_eq!(clean("ok [#11,#12]", &k), "ok [#11, #12]");
    }

    #[test]
    fn a_bracket_that_is_not_a_citation_is_left_alone() {
        let k = known();
        assert_eq!(
            clean("[photo] and [a, b] and [", &k),
            "[photo] and [a, b] and ["
        );
    }

    #[test]
    fn brackets_side_by_side_are_one_citation_of_at_most_three() {
        let k = known();
        assert_eq!(
            clean("He was for it [#10][#11] [#12][#13] [#14].", &k),
            "He was for it [#10, #11, #12]."
        );
        assert_eq!(
            clean("A [#10, #11, #12, #13, #14] B", &k),
            "A [#10, #11, #12] B"
        );
        // Unknown ids go before the cap is counted, and a repeat is one id.
        assert_eq!(clean("A [#99][#10] [#10, #98][#11]", &k), "A [#10, #11]");
        // Not across words, a line break, or a bracket that is no citation.
        assert_eq!(
            clean("A [#10] and [#11]\n[#12] [photo] [#13]", &k),
            "A [#10] and [#11]\n[#12] [photo] [#13]"
        );
    }

    #[test]
    fn a_stream_cut_anywhere_gives_the_same_text() {
        let k = known();
        let text = "One [#10]. Two [#99]. Three [#11, #77] and [x] end [#12] \
                    then [#10][#11] [#12][#13] [#99] here [#99] [#98] gone, \
                    [#1x] [ #13 ] [photo] [#14]] and [#10, #11, #12, #13] \
                    [see [#12]] last [#13]   [#14]";
        let whole = clean(text, &k);
        for size in 1..12 {
            assert_eq!(streamed(text, size, &k), whole, "pieces of {size}");
        }
    }

    #[test]
    fn a_stream_holds_only_what_may_still_change() {
        let k = known();
        let mut f = Filter::new(&k);
        assert_eq!(f.push("Done [#10]"), "Done");
        // The next citation may still join it.
        assert_eq!(f.push(" "), "");
        assert_eq!(f.push("[#11"), "");
        assert_eq!(f.push("] and"), " [#10, #11] and");
        // A bracket that cannot be a citation is not held.
        assert_eq!(f.push(" [photo"), " [photo");
        assert_eq!(f.finish(), "");
    }

    #[test]
    fn cited_ids_are_found_in_order() {
        assert_eq!(cited("a [#3] b [#1, #2] [photo] [2022]"), vec![3, 1, 2]);
    }

    /// **The model drops the `#`.** `[10, 11]` was passed through unchecked
    /// and drawn as plain text; it is a citation when every number is an id
    /// the model was given, and a year or a footnote is not.
    #[test]
    fn a_citation_without_its_hash_is_checked_and_written_with_it() {
        let k = known();
        assert_eq!(
            clean("Said [10, 11]. Then [12] [#13].", &k),
            "Said [#10, #11]. Then [#12, #13]."
        );
        // One number not given: not a citation at all, so nothing is dropped.
        assert_eq!(
            clean("In [2022], see [1] and [10, 99].", &k),
            "In [2022], see [1] and [10, 99]."
        );
        assert_eq!(clean("[-10] [+10] [ 10 ]", &k), "[-10] [+10] [#10]");
        let text = "A [10, 11] b [2022] c [12][13] d [10, 99] e [14]";
        let whole = clean(text, &k);
        for size in 1..12 {
            assert_eq!(streamed(text, size, &k), whole, "pieces of {size}");
        }
    }
}
