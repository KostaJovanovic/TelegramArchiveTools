//! Extraction and date inference against real documents.
//!
//! The unit tests prove the patterns against filenames typed into the source.
//! They cannot prove that a `.docx` Word actually wrote comes apart, or that
//! `pdf-extract` survives 238 PDFs off people's phones. Only the corpus does
//! that.
//!
//! **The skip is deliberate and it is loud**, for the reason in
//! `tga-read/tests/corpus.rs`: libtest discards stdout for a passing test, so a
//! corpus test that quietly does nothing is one nobody knows stopped running.
//! `TGA_REQUIRE_CORPUS=1` turns a missing export into a failure.

use std::path::{Path, PathBuf};

/// The export with the documents in it. `TGA_DOCS_EXPORT` overrides.
///
/// Matched by prefix rather than named in full: the folder's real name ends
/// `3.0 ©®™️`, with a variation selector and a trailing space, and a literal
/// in the source is one careless edit away from silently never matching.
const PARENT: &str = r"N:\telegram_export";
const PREFIX: &str = "KROVNA RADNA GRUPA ZA MEDIJE";

fn corpus() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("TGA_DOCS_EXPORT") {
        let path = PathBuf::from(path);
        if path.is_dir() {
            return Some(path);
        }
    } else if let Ok(entries) = std::fs::read_dir(PARENT) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(PREFIX) {
                return Some(entry.path());
            }
        }
    }
    let required = std::env::var("TGA_REQUIRE_CORPUS").as_deref() == Ok("1");
    assert!(
        !required,
        "TGA_REQUIRE_CORPUS=1 but no export starting {PREFIX:?} under {PARENT}"
    );
    eprintln!("SKIP: no document corpus — set TGA_DOCS_EXPORT to point at an export");
    None
}

/// Every `<topic>/files/*` in the export. Attachments live there and nowhere
/// else; a `.txt` at a topic's root is `missing_media.txt`, the exporter's own
/// log of what it failed to download, and is not a document anybody sent.
fn attachments(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(topics) = std::fs::read_dir(root) else {
        return out;
    };
    for topic in topics.flatten() {
        let Ok(files) = std::fs::read_dir(topic.path().join("files")) else {
            continue;
        };
        out.extend(files.flatten().map(|f| f.path()).filter(|p| p.is_file()));
    }
    out
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn every_docx_on_disk_comes_apart() {
    let Some(root) = corpus() else { return };
    let docx: Vec<PathBuf> = attachments(&root)
        .into_iter()
        .filter(|p| name_of(p).to_ascii_lowercase().ends_with(".docx"))
        .collect();
    assert!(docx.len() > 100, "only {} .docx found", docx.len());

    let empty: Vec<String> = docx
        .iter()
        .filter(|p| tga_docs::extract(p, &name_of(p)).is_none_or(|t| t.trim().is_empty()))
        .map(|p| name_of(p))
        .collect();
    eprintln!(
        "docx: {} read, {} empty",
        docx.len() - empty.len(),
        empty.len()
    );
    // A `.docx` is a zip of XML and always has a text layer, so unlike a PDF
    // there is no honest reason for one to come back empty.
    assert!(empty.is_empty(), "these .docx yielded nothing: {empty:?}");
}

/// One pass over every PDF, checking both things worth checking.
///
/// Deliberately one test and not two: parsing 238 PDFs is a minute of the
/// suite's wall clock, and a second test that walked them again to assert one
/// more thing would double that for nothing.
#[test]
fn pdfs_have_a_text_layer_and_none_of_them_bring_the_run_down() {
    let Some(root) = corpus() else { return };
    let floor = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).expect("a real date");
    let posted = chrono::NaiveDate::from_ymd_opt(2026, 8, 27).expect("a real date");

    let pdfs: Vec<PathBuf> = attachments(&root)
        .into_iter()
        .filter(|p| name_of(p).to_ascii_lowercase().ends_with(".pdf"))
        .collect();
    assert!(pdfs.len() > 100, "only {} .pdf found", pdfs.len());

    // Reaching the end of this loop at all is most of the test: `pdf-extract`
    // panics on input it dislikes, and this corpus contains input it dislikes.
    let mut read = 0;
    for path in &pdfs {
        let name = name_of(path);
        let Some(doc) = tga_docs::read(path, &name, posted) else {
            continue;
        };
        read += 1;
        // A document dated before Telegram existed is the shape the mistake
        // takes when a pattern reads an identifier as a year.
        if let Some(from) = doc.dates.from {
            assert!(from >= floor, "{name} claims a date of {from}");
        }
    }

    eprintln!("pdf: {read} of {} have a text layer", pdfs.len());
    // The rest are photographs of printed pages, and there is no OCR here.
    // Half is a floor against a regression, not a measurement -- it was 210 of
    // 238 when this was written.
    assert!(
        read * 2 > pdfs.len(),
        "only {read} of {} PDFs yielded text",
        pdfs.len()
    );
}

#[test]
fn the_documents_whose_dates_were_worked_out_by_hand_still_agree() {
    let Some(root) = corpus() else { return };
    let day = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).expect("a real date");

    // (path under the export, the message's own date, the answer, the rung)
    let cases: [(&str, chrono::NaiveDate, chrono::NaiveDate, &str); 3] = [
        // Its whole contents are "pocetak / fpn: ... / kraj". The only date
        // anywhere is `15.6` in the name, and the year is the post date's.
        (
            r"0018 - zapisnici\files\15.6.txt",
            day(2026, 6, 15),
            day(2026, 6, 15),
            "filename+posted",
        ),
        // Dated in the name *and* in the body, and they agree.
        (
            r"0001 - General\files\Rezultati glasanja - 2025-11-28.txt",
            day(2025, 11, 28),
            day(2025, 11, 28),
            "filename",
        ),
        // 180 lines of scraped URLs, several ending `-2026/`. None is a date.
        (
            r"0007 - carska diskusija\files\urls_promevent_rs_simplescraper.txt",
            day(2026, 4, 2),
            day(2026, 4, 2),
            "posted",
        ),
    ];

    for (rel, posted, want, rung) in cases {
        let path = root.join(rel);
        if !path.is_file() {
            eprintln!("SKIP: {rel} is not in this export");
            continue;
        }
        let doc = tga_docs::read(&path, &name_of(&path), posted).expect("read");
        assert_eq!(doc.dates.best, Some(want), "{rel}");
        assert_eq!(doc.dates.src.as_str(), rung, "{rel}");
    }
}
