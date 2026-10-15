//! `.docx`: a zip holding `word/document.xml`, which holds the prose.

use std::io::{BufReader, Cursor, Read};

use quick_xml::events::Event;
use quick_xml::Reader;

/// The most of `word/document.xml` inflated, in bytes.
///
/// **`MAX_BYTES` bounds the zip, not what is in it.** A 60 MB archive can
/// declare an entry of many gigabytes, and the whole of it used to be inflated
/// and its text gathered into one string before [`crate::MAX_CHARS`] cut it.
/// Word's XML runs to a few hundred bytes of markup per sentence; this is
/// thousands of times what the longest zapisnik needs, and a truncated entry
/// is read as far as it goes, like any truncated `.docx`.
const MAX_XML: u64 = 256 * 1024 * 1024;

/// The most text gathered before the read stops, in bytes.
///
/// [`crate::MAX_CHARS`] is applied to the squeezed text, after blank lines and
/// indentation are gone, so the stop has to sit well past it or a document
/// padded with empty paragraphs would come out shorter than the cap allows.
/// Sixty-four bytes per character kept is more padding than any document
/// carries, and still bounds what one attachment can cost.
const MAX_TEXT: usize = crate::MAX_CHARS * 64;

/// The prose of a Word document, paragraph per line.
///
/// Only `word/document.xml` is read. Headers, footers, footnotes and comments
/// are separate parts of the archive and are left alone deliberately: in the
/// KRGM minutes they hold page numbers and the template's letterhead, which is
/// the same twelve words on every zapisnik and would be the most repeated text
/// in the whole digest.
pub fn extract(bytes: &[u8]) -> Option<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let entry = archive.by_name("word/document.xml").ok()?;

    // `take` and not `entry.size()`: the size is what the archive *says*, and
    // an archive built to be inflated is the one that would lie about it.
    let mut reader = Reader::from_reader(BufReader::new(entry.take(MAX_XML)));
    let mut buf = Vec::new();
    let mut out = String::new();
    // `w:t` is the only element whose text is the document's text. Everything
    // else -- `w:instrText` above all, which carries field codes like
    // `HYPERLINK "http://..."` -- would arrive as prose if this read every
    // text node it saw.
    let mut in_text = false;

    loop {
        // `local_name` rather than `name`, so a document written with a
        // different namespace prefix than `w:` still reads. Word itself is
        // consistent; Google Docs and LibreOffice exports are what made this
        // worth doing.
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if e.local_name().as_ref() == "t" {
                    in_text = true;
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                "t" => in_text = false,
                "p" => out.push('\n'),
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                "br" | "cr" => out.push('\n'),
                "tab" => out.push('\t'),
                _ => {}
            },
            Ok(Event::Text(e)) => {
                if in_text {
                    out.push_str(&e.xml10_content());
                }
            }
            // **Entity references are their own event.** quick-xml stopped
            // folding them into the surrounding `Text` -- a `w:t` handler that
            // matches only on `Event::Text` compiles, passes a test written
            // with plain ASCII, and silently drops every `&amp;` and every
            // `&#x2014;`. In these documents that is every ampersand and every
            // dash, so "RJMM &amp; RJIL &#x2014; predlog" arrives as
            // "RJMM  RJIL  predlog".
            Ok(Event::GeneralRef(e)) if in_text => match e.resolve_char_ref() {
                // `&#8212;` and `&#x2014;`.
                Ok(Some(ch)) => out.push(ch),
                // A name: `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`. Those
                // five are the only ones a `.docx` can use, because it carries
                // no DTD to define any others. Anything else is dropped.
                _ => {
                    if let Some(text) = quick_xml::escape::resolve_predefined_entity(&e) {
                        out.push_str(text);
                    }
                }
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            // Keep what was read rather than discarding the document. A
            // truncated `.docx` still yields its first pages, and the first
            // page of a zapisnik is the date and the attendance.
            Err(_) => break,
        }
        buf.clear();
        if out.len() > MAX_TEXT {
            break;
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// The bytes of a one-entry zip holding the given `document.xml`.
    fn docx(xml: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("entry");
        zip.write_all(xml.as_bytes()).expect("write");
        zip.finish().expect("finish").into_inner()
    }

    #[test]
    fn paragraphs_become_lines() {
        let path = docx(
            r#"<w:document xmlns:w="x"><w:body>
               <w:p><w:r><w:t>Zapisnik KRGM</w:t></w:r></w:p>
               <w:p><w:r><w:t>Prisutni: 14</w:t></w:r></w:p>
               </w:body></w:document>"#,
        );
        assert_eq!(
            extract(&path).as_deref(),
            Some("Zapisnik KRGM\nPrisutni: 14\n")
        );
    }

    #[test]
    fn runs_inside_one_paragraph_are_not_split() {
        // Word breaks a paragraph into a new run at every formatting change,
        // so a single sentence with one bolded word is three `w:t`s. Joined
        // with anything but nothing, "16. jul" comes out as "16 . jul".
        let path = docx(
            r#"<w:document xmlns:w="x"><w:body><w:p>
               <w:r><w:t>16</w:t></w:r><w:r><w:t>. </w:t></w:r>
               <w:r><w:t>jul</w:t></w:r></w:p></w:body></w:document>"#,
        );
        assert_eq!(extract(&path).as_deref(), Some("16. jul\n"));
    }

    #[test]
    fn field_codes_are_not_prose() {
        let path = docx(
            r#"<w:document xmlns:w="x"><w:body><w:p>
               <w:r><w:instrText> HYPERLINK "http://example.rs" </w:instrText></w:r>
               <w:r><w:t>zapisnik</w:t></w:r></w:p></w:body></w:document>"#,
        );
        assert_eq!(extract(&path).as_deref(), Some("zapisnik\n"));
    }

    #[test]
    fn entities_are_unescaped() {
        // The reason this crate parses XML instead of stripping tags.
        let path = docx(
            r#"<w:document xmlns:w="x"><w:body><w:p>
               <w:r><w:t>RJMM &amp; RJIL &#x2014; predlog</w:t></w:r>
               </w:p></w:body></w:document>"#,
        );
        assert_eq!(extract(&path).as_deref(), Some("RJMM & RJIL — predlog\n"));
    }

    #[test]
    fn a_document_far_past_the_cap_stops_being_read_rather_than_filling_memory() {
        // Three million characters of prose, deflated to almost nothing: the
        // shape of an archive built to be inflated. Read whole, it is all held
        // in one string before anything cuts it.
        let line = "a".repeat(100);
        let mut xml = String::from(r#"<w:document xmlns:w="x"><w:body>"#);
        for _ in 0..30_000 {
            xml.push_str(&format!("<w:p><w:r><w:t>{line}</w:t></w:r></w:p>"));
        }
        xml.push_str("</w:body></w:document>");
        let bytes = docx(&xml);
        assert!(bytes.len() < 1024 * 1024, "the fixture should be small");

        let text = extract(&bytes).expect("still read");
        assert!(text.len() <= MAX_TEXT + line.len() + 1, "{}", text.len());
        assert!(text.starts_with(&format!("{line}\n{line}\n")));
        // And what comes out is still cut at the cap and says so.
        let doc = crate::read_bytes(
            &bytes,
            "big.docx",
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .expect("a document");
        assert!(doc.cut);
        assert_eq!(doc.text.chars().count(), crate::MAX_CHARS);
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_none_rather_than_a_panic() {
        assert_eq!(extract(b"PK\x03\x04 and then nothing"), None);
    }
}
