//! What a text attachment says, and when it is really from.
//!
//! An export folder is mostly pictures, but the documents in it are where the
//! decisions are written down. In the KRGM corpus 695 messages carry a
//! document, 610 of them with an empty `text` field -- the message *is* the
//! attachment -- and the digest used to render every one of those as
//! `{"media": "document"}` and nothing else. A model reading that has been told
//! a file existed, which is not the same as being told what it said.
//!
//! Two jobs, and the second is the harder one:
//!
//! 1. [`extract`] turns a `.txt`, `.docx` or `.pdf` into plain text.
//! 2. [`dates`] works out when the document is actually *from*, which is
//!    usually not when it was posted and never what the filesystem says.
//!
//! **Why the date needs working out at all.** Every file in an export folder
//! carries the mtime of the moment the exporter wrote it -- all 44,000 files in
//! the KRGM corpus say `2026-08-27`. The message date is better but still
//! wrong whenever somebody posts an archive: five KRGM minutes from December
//! 2024 through February 2025 were all dumped into the chat on 2025-09-24,
//! between 232 and 268 days after the meetings they record. Pinned to their
//! post date they form a spike on one afternoon in September and say nothing
//! about the winter they came from.

use std::path::Path;

pub mod dates;
mod docx;
mod pdf;
mod plain;

pub use dates::{Dates, Source};

/// The most text taken from one document.
///
/// A KRGM zapisnik runs three to eight thousand characters, so in practice
/// nothing is cut and the cap only catches the outliers -- `sns276.docx` is
/// 1.6 MB and would otherwise outweigh a month of conversation on its own.
pub const MAX_CHARS: usize = 20_000;

/// The most bytes read off disk before giving up on a file.
///
/// Separate from [`MAX_CHARS`] and much larger, because the cost of a `.docx`
/// is in inflating it, not in the text that comes out: a 1.6 MB archive can
/// hold a few pages of prose and a great many embedded images.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// A document, read.
#[derive(Debug, Clone, Default)]
pub struct Doc {
    pub text: String,
    /// Whether [`MAX_CHARS`] cut it.
    pub cut: bool,
    pub dates: Dates,
}

/// Whether this crate has anything to say about a file, by extension.
///
/// Extension rather than the export's `mime_type`, which is what Telegram was
/// told by the sending client and is `application/octet-stream` often enough to
/// matter.
pub fn is_document(name: &str) -> bool {
    matches!(
        extension(name).as_str(),
        "txt" | "md" | "log" | "csv" | "docx" | "pdf"
    )
}

fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Read one document and work out its date.
///
/// `name` is the name the sender gave the file, which is not the path on disk:
/// the exporter deduplicates, so `Zapisnik-KRGM-16-07-2026 (2).docx` on disk
/// was `Zapisnik-KRGM-16-07-2026.docx` in the chat. The dates come out of
/// `name` for that reason, and `posted` supplies the year when the name gives
/// only a day and a month.
///
/// Returns `None` when the file is not a document, is missing, or yielded no
/// text at all -- a scanned PDF with no text layer is the common case, and an
/// empty `text` in the digest is worse than no key, because it reads as "the
/// document was blank".
pub fn read(path: &Path, name: &str, posted: chrono::NaiveDate) -> Option<Doc> {
    if !is_document(name) {
        return None;
    }
    let text = extract(path, name)?;
    let text = squeeze(&text);
    if text.trim().is_empty() {
        return None;
    }
    let (text, cut) = if text.chars().count() > MAX_CHARS {
        (text.chars().take(MAX_CHARS).collect(), true)
    } else {
        (text, false)
    };
    let dates = dates::infer(name, &text, posted);
    Some(Doc { text, cut, dates })
}

/// A document to plain text, or `None` if it could not be read.
///
/// Every failure is a `None` rather than an error. A digest is written
/// alongside a report that has already succeeded, and one unreadable
/// attachment out of seven hundred is not worth failing the run over; the row
/// simply keeps its filename and loses its text.
pub fn extract(path: &Path, name: &str) -> Option<String> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() <= MAX_BYTES => {}
        _ => return None,
    }
    match extension(name).as_str() {
        "txt" | "md" | "log" | "csv" => plain::extract(path),
        "docx" => docx::extract(path),
        "pdf" => pdf::extract(path),
        _ => None,
    }
}

/// Collapse runs of blank lines and trailing spaces, keep single newlines.
///
/// Unlike the message digest's `squeeze`, this does **not** flatten to one
/// line: a zapisnik is a list of points and running them together loses the
/// only structure it has. JSONL survives it because the newlines are escaped
/// as `\n` inside the JSON string, not written raw.
fn squeeze(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blanks = 0;
    for line in text.lines() {
        // Both ends. The digest has no use for indentation, and `.docx`
        // extraction leaves a lot of it -- Word writes a paragraph's leading
        // spaces as text, not as a margin.
        let line = line.trim();
        if line.trim().is_empty() {
            blanks += 1;
            // One blank line is a paragraph break and worth keeping; the
            // fourteen that `.docx` extraction leaves between two sentences
            // are not.
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_matched_regardless_of_case() {
        assert!(is_document("Zapisnik.DOCX"));
        assert!(is_document("notes.Txt"));
        assert!(!is_document("photo_2649@07-01-2026_20-19-54.jpg"));
        assert!(!is_document("no-extension"));
    }

    #[test]
    fn a_doubled_extension_reads_as_the_last_one() {
        // `krgm_12_3_2025.docx.pdf` is in the corpus: exported to PDF and the
        // old extension left in the name. It is a PDF.
        assert!(is_document("krgm_12_3_2025.docx.pdf"));
        assert_eq!(extension("krgm_12_3_2025.docx.pdf"), "pdf");
    }

    #[test]
    fn blank_runs_collapse_but_line_structure_survives() {
        assert_eq!(squeeze("a\n\n\n\n b \n\nc\n"), "a\n\nb\n\nc");
    }
}
