//! A chat title's search and sort form: lower case, one alphabet.
//!
//! **These chats are Serbian, and Serbian is written in two alphabets.** A
//! group named `Ђорђе` is looked for as `djordje`, and one named `Đorđe` as
//! `ђорђе`; a filter that compared the letters as typed found neither, and a
//! sort put every Cyrillic title after every Latin one. So both sides go
//! through [`fold`]: Cyrillic is written in Latin, and the Latin letters with
//! marks lose them — `č`, `ć` and `ч`, `ћ` all become `c` — because someone
//! typing on an English keyboard types the bare letter.
//!
//! Russian and Ukrainian letters are folded too (`я` is `ja`, `щ` is `sc`),
//! the way a Serbian reader would write them; what matters is that a title
//! and a filter fold the same way, not that the result is a transliteration
//! anyone would publish.

/// `text` lower-cased, in Latin, without diacritics.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match latin(c) {
            Some(s) => out.push_str(s),
            None => out.push(c),
        }
    }
    out
}

/// Whether `title` contains `needle`, both folded. An empty needle matches
/// everything.
pub fn matches(title: &str, needle: &str) -> bool {
    let needle = fold(needle.trim());
    needle.is_empty() || fold(title).contains(&needle)
}

/// One lower-case letter in folded Latin, or `None` to keep it as it is.
fn latin(c: char) -> Option<&'static str> {
    Some(match c {
        // Serbian Cyrillic.
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'ђ' => "dj",
        'е' => "e",
        'ж' => "z",
        'з' => "z",
        'и' => "i",
        'ј' => "j",
        'к' => "k",
        'л' => "l",
        'љ' => "lj",
        'м' => "m",
        'н' => "n",
        'њ' => "nj",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'ћ' => "c",
        'у' => "u",
        'ф' => "f",
        'х' => "h",
        'ц' => "c",
        'ч' => "c",
        'џ' => "dz",
        'ш' => "s",
        // Russian and Ukrainian.
        'й' => "j",
        'ы' => "y",
        'э' => "e",
        'ю' => "ju",
        'я' => "ja",
        'ё' => "e",
        'щ' => "sc",
        'ъ' | 'ь' => "",
        'і' => "i",
        'ї' => "ji",
        'є' => "je",
        'ґ' => "g",
        // Serbian Latin's marks, and the commonest others.
        'č' | 'ć' => "c",
        'š' => "s",
        'ž' => "z",
        'đ' => "dj",
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => "a",
        'é' | 'è' | 'ê' | 'ë' => "e",
        'í' | 'ì' | 'î' | 'ï' => "i",
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => "o",
        'ú' | 'ù' | 'û' | 'ü' => "u",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyrillic_and_latin_fold_to_the_same_text() {
        assert_eq!(fold("Ђорђе Чубрило"), fold("Đorđe Čubrilo"));
        assert_eq!(fold("Ђорђе"), "djordje");
        assert_eq!(fold("Љубиша Њ Џ"), "ljubisa nj dz");
        assert_eq!(fold("DŽEMAL"), fold("Џемал"));
    }

    #[test]
    fn a_filter_finds_a_title_in_either_alphabet_and_without_marks() {
        assert!(matches("Кровна радна група", "krovna"));
        assert!(matches("Krovna radna grupa", "КРОВНА"));
        assert!(matches("Šumadija", "sumad"));
        assert!(matches("Ćuprija", "ћуп"));
        assert!(matches("anything", "  "));
        assert!(!matches("Кровна", "radna"));
    }

    #[test]
    fn ukrainian_and_russian_letters_fold_as_well() {
        assert!(matches("Україна", "ukrajina"));
        assert!(matches("Москва", "moskva"));
    }
}
