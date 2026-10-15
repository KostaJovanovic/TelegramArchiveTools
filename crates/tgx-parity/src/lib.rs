//! The oracle harness, as a library so its legs can also run from a test.
//!
//! The binary is the everyday face of this crate — you point it at
//! `N:\telegram export\…` and it prints a burn-down. But a harness that only
//! exists as a command is a harness that stops running the moment the drive
//! holding the reference goes away, and "the reference is on one machine" is
//! the standing risk in the roadmap. So the legs live here, and
//! `tests/corpus.rs` runs the same code against a small committed corpus.

pub mod archive_leg;
pub mod corpus;
pub mod html_leg;
pub mod json_leg;
pub mod media_leg;
pub mod wire_leg;

use anyhow::Result;
use std::path::{Path, PathBuf};

/// Every folder under `root` holding a `result.json`, sorted.
///
/// Desktop writes `chats/chat_<id>/`; our own exporter writes one folder per
/// topic directly under the root. Both are found without a flag, which is what
/// lets the harness run against either.
pub fn topic_folders(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect(root, &mut out, 0)?;
    out.sort();
    Ok(out)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) -> Result<()> {
    if depth > 3 {
        return Ok(());
    }
    if dir.join("result.json").is_file() {
        out.push(dir.to_path_buf());
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out, depth + 1)?;
        }
    }
    Ok(())
}

/// The variable that lets a failing leg print the chat's own words.
///
/// **Off unless set to `1`.** The reference is other people's conversation,
/// and a differing line of `result.json` or `messages.html` is a message, a
/// name or a file name, whole. A failing corpus test prints its detail into
/// whatever ran it — `save.bat test`, CI's log, an agent's transcript — so by
/// default a leg says *where* the two disagree and what shape the line has,
/// and only the account holder, at their own terminal, asks for the text.
pub const SHOW_TEXT: &str = "TGX_PARITY_SHOW_TEXT";

/// Whether [`SHOW_TEXT`] is `1` for this process.
pub fn show_text() -> bool {
    std::env::var(SHOW_TEXT).is_ok_and(|v| v.trim() == "1")
}

/// Report the first differing line between two texts.
///
/// Deliberately line-based and deliberately brief: a format bug produces
/// thousands of identical diffs and the first one is the whole story. The
/// lines themselves only under [`SHOW_TEXT`]; otherwise the line number, the
/// key or tag it opens with, its length on each side and the column where the
/// two part, which is usually enough to name the bug — an escape, an indent,
/// a missing attribute — without reading the message.
pub fn first_difference(theirs: &str, ours: &str) -> Option<String> {
    first_difference_as(theirs, ours, show_text())
}

fn first_difference_as(theirs: &str, ours: &str, show: bool) -> Option<String> {
    let a: Vec<&str> = theirs.lines().collect();
    let b: Vec<&str> = ours.lines().collect();
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x == y {
            continue;
        }
        if show {
            return Some(format!(
                "line {}:\n      desktop: {:?}\n      ours   : {:?}",
                i + 1,
                x,
                y
            ));
        }
        let col = x.chars().zip(y.chars()).take_while(|(p, q)| p == q).count();
        return Some(format!(
            "line {}, {}, from column {}:\n      desktop: {}\n      ours   : {}\n      \
             ({SHOW_TEXT}=1 prints both lines)",
            i + 1,
            line_lead(x),
            col + 1,
            line_shape(x, col),
            line_shape(y, col)
        ));
    }
    if a.len() != b.len() {
        return Some(format!(
            "line count {} (desktop) vs {} (ours)",
            a.len(),
            b.len()
        ));
    }
    None
}

/// What a line opens with, in the format's vocabulary rather than the chat's.
///
/// A JSON key is Desktop's schema and an HTML tag and `class` are Desktop's
/// markup, so both are safe to print; a JSON string standing alone is a piece
/// of a message's `text` array, and an HTML line that opens with neither is a
/// message's text, so those are named and not shown.
fn line_lead(line: &str) -> String {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix('"') {
        let mut escaped = false;
        let end = rest.char_indices().find(|&(_, c)| {
            let close = c == '"' && !escaped;
            escaped = c == '\\' && !escaped;
            close
        });
        return match end {
            Some((i, _)) if rest[i + 1..].trim_start().starts_with(':') => {
                format!("key {:?}", &rest[..i])
            }
            _ => "a JSON string".into(),
        };
    }
    if let Some(rest) = t.strip_prefix('<') {
        let close = if rest.starts_with('/') { "/" } else { "" };
        let tag: String = rest[close.len()..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '!')
            .collect();
        // Only the tag's own attributes: the text after `>` is the message.
        let head = rest.split('>').next().unwrap_or(rest);
        let class = head
            .split_once("class=\"")
            .and_then(|(_, c)| c.split_once('"'))
            .map(|(c, _)| format!(" class={c:?}"))
            .unwrap_or_default();
        return format!("tag <{close}{tag}{class}>");
    }
    if t.is_empty() {
        return "a blank line".into();
    }
    "a text line".into()
}

/// One side of a differing line, as its length and what sits at `col`.
///
/// A letter or a digit is only named, never shown. Markup, spaces and the
/// other symbols are shown, because those are what a writer gets wrong and
/// none of them spells anything.
fn line_shape(line: &str, col: usize) -> String {
    let len = line.chars().count();
    let indent = line.chars().take_while(|c| *c == ' ').count();
    let at = match line.chars().nth(col) {
        None => "the end of the line".to_string(),
        Some(c) if c.is_alphabetic() => "a letter".into(),
        Some(c) if c.is_numeric() => "a digit".into(),
        Some(c) if c.is_ascii() => format!("{c:?}"),
        Some(c) => format!("U+{:04X}", c as u32),
    };
    format!("{len} chars, indent {indent}, then {at}")
}

/// How many lines differ in total, for the burn-down number.
pub fn differing_lines(theirs: &str, ours: &str) -> usize {
    let a: Vec<&str> = theirs.lines().collect();
    let b: Vec<&str> = ours.lines().collect();
    let common = a.iter().zip(b.iter()).filter(|(x, y)| x != y).count();
    common + a.len().abs_diff(b.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "meet me at the old bridge";

    #[test]
    fn a_differing_json_line_is_described_and_not_printed() {
        let theirs = format!("{{\n   \"text\": \"{SECRET}\",\n}}");
        let ours = format!("{{\n   \"text\": \"{SECRET}!\",\n}}");
        let out = first_difference_as(&theirs, &ours, false).unwrap();
        assert!(!out.contains("bridge"), "{out}");
        assert!(out.contains("line 2"), "{out}");
        assert!(out.contains("key \"text\""), "{out}");
        // The column where they part, and what is there on each side.
        let col = "   \"text\": \"".len() + SECRET.len() + 1;
        assert!(out.contains(&format!("column {col}")), "{out}");
        assert!(out.contains("then '\"'"), "{out}");
        assert!(out.contains("then '!'"), "{out}");
        assert!(out.contains(SHOW_TEXT), "{out}");
    }

    #[test]
    fn a_piece_of_a_text_array_is_not_mistaken_for_a_key() {
        // `"text": [ "plain", {…} ]` puts each plain piece on its own line, and
        // a piece that holds `": ` must still not be printed as a key.
        let theirs = format!("    \"{SECRET} \\\"quoted\\\": x\",");
        let ours = format!("    \"{SECRET}.\",");
        let out = first_difference_as(&theirs, &ours, false).unwrap();
        assert!(!out.contains("bridge"), "{out}");
        assert!(out.contains("a JSON string"), "{out}");
    }

    #[test]
    fn a_differing_html_line_names_its_tag_and_class_only() {
        let theirs = format!("     <div class=\"text\">{SECRET}</div>");
        let ours = format!("     <div class=\"text\">{SECRET} </div>");
        let out = first_difference_as(&theirs, &ours, false).unwrap();
        assert!(!out.contains("bridge"), "{out}");
        assert!(out.contains("tag <div class=\"text\">"), "{out}");
        assert!(out.contains("indent 5"), "{out}");

        let out = first_difference_as(SECRET, "meet me at the new bridge", false).unwrap();
        assert!(!out.contains("bridge"), "{out}");
        assert!(out.contains("a text line"), "{out}");
        assert!(out.contains("then a letter"), "{out}");
    }

    #[test]
    fn the_variable_brings_the_whole_lines_back() {
        let out = first_difference_as(SECRET, "meet me", true).unwrap();
        assert!(out.contains(SECRET), "{out}");
    }

    #[test]
    fn a_line_count_difference_is_still_reported() {
        assert_eq!(
            first_difference_as("a\nb", "a", false).unwrap(),
            "line count 2 (desktop) vs 1 (ours)"
        );
        assert_eq!(first_difference_as("a\nb", "a\nb", false), None);
    }
}
