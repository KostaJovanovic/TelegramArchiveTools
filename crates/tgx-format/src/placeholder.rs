//! What Desktop writes in place of a file it did not save.
//!
//! `file`, `photo` and `thumbnail` normally hold a path relative to the export
//! folder. When Desktop declined to fetch the file they hold one of these
//! sentences instead, and every reader has to tell the two apart: the HTML
//! draws a placeholder row rather than a link, the media leg expects no file
//! on disk, and the database replaces a stored one once a later run fetches
//! the file (`tgx_archive::merge_volatile`).
//!
//! That last one is why this lives here, in the crate everything already
//! depends on. The test was hand-copied at about ten sites, spelled `"(File"`
//! in some and `"(File "` in others, and the archive's copy was tied to the
//! planner's sentences by one test in `tgx-tg`, the only crate allowed to see
//! both.

/// The file was left out by the export's settings.
pub const NOT_INCLUDED: &str = "(File not included. Change data exporting settings to download.)";

/// The file was over the export's size limit.
pub const TOO_LARGE: &str =
    "(File exceeds maximum size. Change data exporting settings to download.)";

/// What every placeholder Desktop writes starts with, including the
/// `"(File unavailable, please try again later)"` it writes for a fetch that
/// failed, which this program never writes itself.
///
/// A path never starts with it: Desktop's paths start with their folder,
/// `photos/`, `files/` and the rest.
pub const PREFIX: &str = "(File ";

/// Whether a media key's value says the file was not saved, rather than
/// where it was.
pub fn is_skipped(value: &str) -> bool {
    value.starts_with(PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sentence_desktop_writes_is_a_placeholder() {
        // Desktop's own wording, as the reference export records it.
        for sentence in [
            NOT_INCLUDED,
            TOO_LARGE,
            "(File unavailable, please try again later)",
        ] {
            assert!(is_skipped(sentence), "{sentence:?}");
        }
        assert_eq!(
            NOT_INCLUDED,
            "(File not included. Change data exporting settings to download.)"
        );
        assert_eq!(
            TOO_LARGE,
            "(File exceeds maximum size. Change data exporting settings to download.)"
        );
    }

    #[test]
    fn a_path_is_not_a_placeholder() {
        for path in [
            "photos/photo_1@01-01-2025.jpg",
            "files/(File notes).pdf",
            "",
            "(Filed).txt",
        ] {
            assert!(!is_skipped(path), "{path:?}");
        }
    }
}
