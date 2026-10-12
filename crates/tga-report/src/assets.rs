//! The type system, base64'd into the stylesheet.
//!
//! Ported from `analyser/report.py`'s `font_css`.
//!
//! Self-contained means one file: no folder beside it and no request to
//! anything. Three faces, not five — regular and medium for text and one mono.
//! Each is about 65 KB, so the whole type system costs ~200 KB of the report,
//! less than one photo in the export it describes.
//!
//! The bytes are `include_bytes!`'d rather than read at run time, for the same
//! reason `tgx-ui` embeds its own: a binary you can copy anywhere and it still
//! works. The Python reads them off disk from `app/ui/fonts`, and `fonts/` here
//! holds byte-identical copies of the same three files, so the base64 the two
//! emit is the same string.

/// `(file name, family, weight, bytes)`, in emission order.
///
/// Order is observable — it is the order the `@font-face` rules appear in — and
/// the parity diff compares the stylesheet character for character.
type Face = (&'static str, &'static str, u16, &'static [u8]);

const FACES: &[Face] = &[
    (
        "Geist-Regular.ttf",
        "Geist",
        400,
        include_bytes!("../fonts/Geist-Regular.ttf"),
    ),
    (
        "Geist-Medium.ttf",
        "Geist",
        500,
        include_bytes!("../fonts/Geist-Medium.ttf"),
    ),
    (
        "GeistMono-Regular.ttf",
        "Geist Mono",
        400,
        include_bytes!("../fonts/GeistMono-Regular.ttf"),
    ),
];

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding, as Python's `b64encode` produces.
///
/// Hand-rolled rather than pulled in as a dependency: it is twenty lines, it is
/// on the critical path of exactly one feature, and the alternative is a crate
/// in the tree of something whose whole job is to write a text file.
pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18 & 0x3f) as usize] as char);
        out.push(ALPHABET[(triple >> 12 & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6 & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// `@font-face` rules with the TTFs base64'd in.
///
/// `embed` false is the `--no-fonts` escape hatch: a much smaller file that
/// looks right only where Geist is installed. It emits nothing at all rather
/// than a `src:local(...)` fallback, because the stack in the stylesheet
/// already names `Segoe UI` and `system-ui` behind Geist.
pub fn font_css(embed: bool) -> String {
    if !embed {
        return String::new();
    }
    FACES
        .iter()
        .map(|(_, family, weight, data)| {
            format!(
                "@font-face{{font-family:'{family}';font-style:normal;\
                 font-weight:{weight};font-display:swap;\
                 src:url(data:font/ttf;base64,{}) format('truetype')}}",
                base64(data)
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_reference_vectors_including_the_padding() {
        // RFC 4648's own test vectors. The padding is the half that is easy to
        // get wrong and that a browser rejects silently, leaving the report in
        // the fallback face with no error anywhere.
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_covers_the_high_bytes_the_alphabet_ends_on() {
        // The last two alphabet entries, `+` and `/`, only appear for byte
        // patterns a text vector never produces.
        assert_eq!(base64(&[0xfb, 0xff, 0xbf]), "+/+/");
        assert_eq!(base64(&[0xff, 0xff, 0xff]), "////");
    }

    #[test]
    fn three_faces_are_embedded_and_the_families_are_the_two_the_css_names() {
        let css = font_css(true);
        assert_eq!(css.matches("@font-face").count(), 3);
        assert_eq!(css.matches("font-family:'Geist'").count(), 2);
        assert_eq!(css.matches("font-family:'Geist Mono'").count(), 1);
        assert_eq!(css.matches("font-weight:500").count(), 1);
        assert!(css.contains("format('truetype')"));
    }

    #[test]
    fn the_type_system_stays_inside_its_share_of_the_budget() {
        // PLAN.md budgets ~260 KB for type out of a 1 MB target. Base64 is 4/3
        // of the bytes, so the three faces land near 270 KB; a fourth face
        // added without thinking would show up here.
        let css = font_css(true);
        assert!(
            css.len() < 300_000,
            "the embedded type is {} bytes",
            css.len()
        );
    }

    #[test]
    fn no_fonts_embeds_nothing_rather_than_a_broken_reference() {
        // The point of the flag is a smaller file, not a file that asks the
        // network for a font it cannot reach.
        assert_eq!(font_css(false), "");
    }
}
