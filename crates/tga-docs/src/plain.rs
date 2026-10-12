//! `.txt` and its neighbours: bytes, decoded.

/// Decode a plain text file, guessing the encoding.
///
/// The order is BOM, then UTF-8, then CP1251. A BOM is a statement and is
/// believed; UTF-8 is checked rather than assumed, because valid UTF-8 is a
/// narrow enough target that passing the check is strong evidence; and CP1251
/// is the last resort because it decodes *any* byte sequence, so trying it
/// earlier would mean never reaching the others.
pub fn extract(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some(String::from_utf8_lossy(rest).into_owned());
    }
    // UTF-16 in either order. Notepad's "Unicode" is the little-endian one and
    // it is still the default in some Save As dialogs.
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Some(encoding_rs::UTF_16LE.decode(bytes).0.into_owned());
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Some(encoding_rs::UTF_16BE.decode(bytes).0.into_owned());
    }

    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_string()),
        // Not a `Result` from here on: CP1251 maps every byte, so this cannot
        // fail, only be wrong. Wrong is recoverable by a reader; absent is not.
        Err(_) => Some(encoding_rs::WINDOWS_1251.decode(bytes).0.into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_utf8_bom_is_stripped_rather_than_kept_as_a_character() {
        // Left in, it arrives as U+FEFF at the head of the first line and
        // shows up in the digest as an invisible character before the text.
        assert_eq!(
            extract(b"\xEF\xBB\xBFzapisnik").as_deref(),
            Some("zapisnik")
        );
    }

    #[test]
    fn utf16_little_endian_is_decoded() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "ćao".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(extract(&bytes).as_deref(), Some("ćao"));
    }

    #[test]
    fn plain_utf8_survives_untouched() {
        assert_eq!(
            extract("Резултати гласања".as_bytes()).as_deref(),
            Some("Резултати гласања")
        );
    }

    #[test]
    fn bytes_that_are_not_utf8_fall_back_rather_than_vanish() {
        // 0xC7 0xE0 is "За" in CP1251 and is not valid UTF-8.
        assert_eq!(extract(&[0xC7, 0xE0]).as_deref(), Some("За"));
    }
}
