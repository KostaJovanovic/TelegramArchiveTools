//! `.pdf`: the text layer, when there is one.
//!
//! Two things about PDFs that the other two formats do not share.
//!
//! **Many of them hold no text at all.** A photographed page is a JPEG in a
//! PDF wrapper, and the corpus is full of them -- somebody photographs a
//! printed dopis and shares it. There is no OCR here, so those come back empty
//! and [`crate::read`] drops the row's text rather than claiming the document
//! was blank.
//!
//! **The parser panics.** `pdf-extract` unwinds on malformed input rather than
//! returning an error, and an export folder is exactly where malformed input
//! lives: files truncated by a failed download, and files written by every
//! phone PDF app there is. One bad attachment must not take the run down, so
//! the call is wrapped.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::OnceLock;

/// Stop caught PDF panics from printing.
///
/// `catch_unwind` stops the panic; it does not stop the *hook*, which runs
/// first and prints. `pdf-extract` panics with the whole offending font
/// dictionary as its message, so one run over the KRGM export put four
/// thousand-character `/CharProcs` dumps on stderr and buried the two lines
/// saying where the digest had been written.
///
/// The hook is filtered by the panic's own location rather than swapped in and
/// out around each call: taking and restoring it would be a race against the
/// window's other thread, and would silence a real panic that happened to land
/// in the gap. Filtering means everything that is not the PDF parser still
/// prints exactly as before.
///
/// Installed once, and never removed -- there is nothing to restore it to that
/// is more correct than this.
fn quiet_parser_panics() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let from_parser = info.location().is_some_and(|at| {
                let file = at.file();
                file.contains("pdf-extract") || file.contains("lopdf")
            });
            if !from_parser {
                previous(info);
            }
        }));
    });
}

pub fn extract(path: &Path) -> Option<String> {
    quiet_parser_panics();
    let bytes = std::fs::read(path).ok()?;
    // `AssertUnwindSafe` because `bytes` is a plain `Vec<u8>` that is moved in
    // and never observed again on the unwind path -- there is no half-mutated
    // state for a caught panic to leave behind.
    let caught = catch_unwind(AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(&bytes)
    }));
    match caught {
        Ok(Ok(text)) => Some(text),
        // Both arms are the same answer: no text. They are kept apart because
        // the difference matters when reading a backtrace -- an `Err` here is
        // a PDF the parser rejected, the outer one is a PDF that killed it.
        Ok(Err(_)) => None,
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rubbish_is_none_rather_than_a_panic() {
        let dir = std::env::temp_dir().join("tga-docs-pdf-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("truncated.pdf");
        // A header and nothing else, which is what a download cut short by a
        // dropped connection leaves on disk.
        std::fs::write(&path, b"%PDF-1.4\n1 0 obj\n<< /Type /Cata").expect("write");
        assert_eq!(extract(&path), None);
    }

    #[test]
    fn a_missing_file_is_none() {
        assert_eq!(extract(Path::new("no-such-file-anywhere.pdf")), None);
    }
}
