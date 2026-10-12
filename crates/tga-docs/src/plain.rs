//! `.txt` and its neighbours: bytes off disk, decoded.

use std::path::Path;

/// Decode a plain text file, guessing the encoding.
///
/// The order is BOM, then UTF-8, then CP1251. A BOM is a statement and is
/// believed; UTF-8 is checked rather than assumed, because valid UTF-8 is a
/// narrow enough target that passing the check is strong evidence; and CP1251
/// is the last resort because it decodes *any* byte sequence, so trying it
/// earlier would mean never reaching the others.
pub fn extract(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;

    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some(String::from_utf8_lossy(rest).into_owned());
    }
    // UTF-16 in either order. Notepad's "Unicode" is the little-endian one and
    // it is still the default in some Save As dialogs.
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Some(encoding_rs::UTF_16LE.decode(&bytes).0.into_owned());
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Some(encoding_rs::UTF_16BE.decode(&bytes).0.into_owned());
    }

    match String::from_utf8(bytes) {
        Ok(text) => Some(text),
        // Not a `Result` from here on: CP1251 maps every byte, so this cannot
        // fail, only be wrong. Wrong is recoverable by a reader; absent is not.
        Err(e) => Some(
            encoding_rs::WINDOWS_1251
                .decode(e.as_bytes())
                .0
                .into_owned(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("tga-docs-plain-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write");
        path
    }

    #[test]
    fn a_utf8_bom_is_stripped_rather_than_kept_as_a_character() {
        // Left in, it arrives as U+FEFF at the head of the first line and
        // shows up in the digest as an invisible character before the text.
        let path = write("bom.txt", b"\xEF\xBB\xBFzapisnik");
        assert_eq!(extract(&path).as_deref(), Some("zapisnik"));
    }

    #[test]
    fn utf16_little_endian_is_decoded() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "ćao".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let path = write("u16.txt", &bytes);
        assert_eq!(extract(&path).as_deref(), Some("ćao"));
    }

    #[test]
    fn plain_utf8_survives_untouched() {
        let path = write("utf8.txt", "Резултати гласања".as_bytes());
        assert_eq!(extract(&path).as_deref(), Some("Резултати гласања"));
    }

    #[test]
    fn bytes_that_are_not_utf8_fall_back_rather_than_vanish() {
        // 0xC7 0xE0 is "За" in CP1251 and is not valid UTF-8.
        let path = write("cp1251.txt", &[0xC7, 0xE0]);
        assert_eq!(extract(&path).as_deref(), Some("За"));
    }
}
