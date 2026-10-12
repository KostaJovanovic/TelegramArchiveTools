//! `.docx`: a zip holding `word/document.xml`, which holds the prose.

use std::io::BufReader;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;

/// The prose of a Word document, paragraph per line.
///
/// Only `word/document.xml` is read. Headers, footers, footnotes and comments
/// are separate parts of the archive and are left alone deliberately: in the
/// KRGM minutes they hold page numbers and the template's letterhead, which is
/// the same twelve words on every zapisnik and would be the most repeated text
/// in the whole digest.
pub fn extract(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file)).ok()?;
    let entry = archive.by_name("word/document.xml").ok()?;

    let mut reader = Reader::from_reader(BufReader::new(entry));
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
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A one-entry zip holding the given `document.xml`, stored uncompressed.
    fn docx(xml: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("tga-docs-docx-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(format!("{:x}.docx", xml.len()));
        let file = std::fs::File::create(&path).expect("create");
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("entry");
        zip.write_all(xml.as_bytes()).expect("write");
        zip.finish().expect("finish");
        path
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
    fn a_file_that_is_not_a_zip_is_none_rather_than_a_panic() {
        let dir = std::env::temp_dir().join("tga-docs-docx-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("truncated.docx");
        std::fs::write(&path, b"PK\x03\x04 and then nothing").expect("write");
        assert_eq!(extract(&path), None);
    }
}
