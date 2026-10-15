//! The text in a photo, read by PaddleOCR-VL — ReVision's OCR, the same way.
//!
//! A screenshot of an announcement, a poster, a photographed page: in a chat
//! these say as much as the messages around them, and a summary that reads
//! `[photo]` misses all of it. So before Gemma reads a range, the range's
//! photos are read by a small vision model (0.9B, ~0.8 GB of the card, under a
//! second an image) on the same `llama-server`, and what it finds goes into
//! the message's line.
//!
//! Everything here follows ReVision's `studio/images.py`, which was measured
//! on 306 images: the prompt is `OCR:` at temperature 0, four images at once
//! (its profile has four slots), [`cut_loops`] for the model's habit of
//! repeating itself, [`fix_cyrillic`] for its habit of writing Serbian letters
//! as Russian ones, and fewer than [`MIN_LETTERS`] letters means no text.
//!
//! **Only JPEG and PNG are sent.** ReVision re-encodes WebP and anything over
//! 1920 px through PIL; this crate links no image decoder, so a WebP is
//! skipped and a large photo goes as it is. Telegram's photos are JPEGs of at
//! most 2560 px, which the model takes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::client::Client;
use crate::Error;

/// ReVision's `paddleocr-vl` profile: all on the card, four slots.
pub const OPTIONS: &str = "-c 32768 -ngl 99 --jinja -np 4";

/// Images read at once: the profile's four slots. ReVision measured 0.44 s an
/// image this way against 0.60 one at a time.
pub const AT_ONCE: usize = 4;

/// The longest reading of one image.
const ANSWER: usize = 1_500;

/// Fewer letters than this: the image has no text worth reading.
pub const MIN_LETTERS: usize = 25;

/// How a photo is sent, from its first bytes; `None` for what the server may
/// not decode.
pub fn mime_of(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else {
        None
    }
}

/// Whether a model file is the OCR's.
pub fn is_ocr(model: &str) -> bool {
    model.to_lowercase().contains("paddleocr")
}

/// The OCR's files in the models folder or one folder down, best in a
/// folder of their own (`paddleocr-vl\`): the model, its vision projector,
/// and the chat template if there is one.
pub fn files_in(models: &Path) -> Option<(PathBuf, PathBuf, Option<PathBuf>)> {
    let mut dirs = vec![models.to_path_buf()];
    if let Ok(list) = std::fs::read_dir(models) {
        let mut sub: Vec<PathBuf> = list
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        sub.sort();
        dirs.extend(sub);
    }
    dirs.into_iter().find_map(|dir| {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        let name = |p: &PathBuf| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };
        let gguf = |p: &&PathBuf| name(p).ends_with(".gguf") && is_ocr(&name(p));
        let model = files
            .iter()
            .filter(gguf)
            .find(|p| !name(p).contains("mmproj"))?;
        // Its projector by name; in a folder of its own, any projector, as
        // a download often names it just `mmproj-F16.gguf`.
        let projector = |p: &&PathBuf| name(p).ends_with(".gguf") && name(p).contains("mmproj");
        let mmproj = files.iter().filter(gguf).find(projector).or_else(|| {
            (dir != models)
                .then(|| files.iter().find(projector))
                .flatten()
        })?;
        let template = files.iter().find(|p| name(p).ends_with(".jinja")).cloned();
        Some((model.clone(), mmproj.clone(), template))
    })
}

/// One photo to read: what its reading is kept under, and its bytes.
pub struct Photo {
    pub key: String,
    pub bytes: Vec<u8>,
}

/// Read `photos`, [`AT_ONCE`] at a time. Each reading, cleaned — an empty
/// string for an image with no text, which is an answer worth keeping too.
/// A photo the server could not read is left out, to be tried another time.
pub fn read_all(
    client: &Client,
    photos: Vec<Photo>,
    cancel: &AtomicBool,
) -> Result<HashMap<String, String>, Error> {
    let queue = Mutex::new(photos.into_iter());
    let out = Mutex::new(HashMap::new());
    let failed: Mutex<Option<Error>> = Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..AT_ONCE {
            s.spawn(|| loop {
                if cancel.load(Ordering::Relaxed) || failed.lock().unwrap().is_some() {
                    return;
                }
                let Some(photo) = queue.lock().unwrap().next() else {
                    return;
                };
                match read(client, &photo.bytes, cancel) {
                    Ok(text) => {
                        out.lock().unwrap().insert(photo.key, text);
                    }
                    // One image the server refused is that image's problem.
                    Err(Error::Server(_)) => {}
                    Err(e) => {
                        let mut first = failed.lock().unwrap();
                        if first.is_none() {
                            *first = Some(e);
                        }
                    }
                }
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    if let Some(e) = failed.into_inner().unwrap() {
        return Err(e);
    }
    Ok(out.into_inner().unwrap())
}

/// One image's text, cleaned; empty when it has too little to count.
pub fn read(client: &Client, bytes: &[u8], cancel: &AtomicBool) -> Result<String, Error> {
    let Some(mime) = mime_of(bytes) else {
        return Ok(String::new());
    };
    let url = format!("data:{mime};base64,{}", base64(bytes));
    let body = json!({
        "messages": [{ "role": "user", "content": [
            { "type": "image_url", "image_url": { "url": url } },
            { "type": "text", "text": "OCR:" }
        ]}],
        "temperature": 0,
        "max_tokens": ANSWER
    });
    let (text, _) = client.complete(&body, cancel)?;
    Ok(clean(&text))
}

/// What the model wrote, made fit for a line: loops cut, Russian letters made
/// Serbian, whitespace collapsed, and nothing at all when it is not text.
pub fn clean(text: &str) -> String {
    let text = fix_cyrillic(&cut_loops(text));
    if text.chars().filter(|c| c.is_alphabetic()).count() < MIN_LETTERS {
        return String::new();
    }
    text
}

/// A small model sometimes repeats a word or a phrase until the answer limit:
/// a run of one to six words said three times or more is kept twice.
/// ReVision's `((?:\S+\s+){1,6}?)\1{2,}` → `\1\1`, on words.
pub fn cut_loops(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<&str> = Vec::with_capacity(words.len());
    let mut i = 0;
    'next: while i < words.len() {
        for n in 1..=6 {
            let unit = &words[i..(i + n).min(words.len())];
            if unit.len() < n {
                break;
            }
            let mut times = 1;
            while words.get(i + times * n..i + (times + 1) * n) == Some(unit) {
                times += 1;
            }
            if times >= 3 {
                out.extend_from_slice(unit);
                out.extend_from_slice(unit);
                i += times * n;
                continue 'next;
            }
        }
        out.push(words[i]);
        i += 1;
    }
    out.join(" ")
}

/// Letters that are Russian and not Serbian.
const RUSSIAN: &str = "йЙяЯьЬыЫэЭёЁъЪ";

/// PaddleOCR-VL writes some Serbian letters, mostly bold or capital ones, as
/// the nearest Russian ones. ReVision checked against Gemma's reading of 41
/// Cyrillic images: ЛЬ=Љ, НЬ=Њ, ТЬ=Ћ, Й=Ј, Э=Ј, Ё=ЈЕ, Ы=И, Ъ dropped, Я=Ј
/// before a vowel and ЈА otherwise, Ь dropped. Only inside Cyrillic words
/// that hold one of those letters; Serbian sources hardly ever hold Russian.
///
/// One difference from ReVision: a capital stays capital in a word that is not
/// all capitals (`Йован` is `Јован`, not `јован`).
pub fn fix_cyrillic(text: &str) -> String {
    if !text.chars().any(|c| RUSSIAN.contains(c)) {
        return text.to_string();
    }
    let cyrillic = |c: char| ('\u{0400}'..='\u{04ff}').contains(&c);
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    for c in text.chars().chain(std::iter::once('\0')) {
        if cyrillic(c) {
            word.push(c);
            continue;
        }
        if word.chars().any(|w| RUSSIAN.contains(w)) {
            out.push_str(&serbian(&word));
        } else {
            out.push_str(&word);
        }
        word.clear();
        if c != '\0' {
            out.push(c);
        }
    }
    out
}

/// One Cyrillic word with its Russian letters made Serbian.
fn serbian(word: &str) -> String {
    let chars: Vec<char> = word.chars().collect();
    let all_caps =
        chars.iter().any(|c| c.is_uppercase()) && !chars.iter().any(|c| c.is_lowercase());
    // `to` in the case the word or the replaced letter calls for.
    let cased = |to: &str, first: char| -> String {
        if all_caps {
            to.to_uppercase()
        } else if first.is_uppercase() {
            let mut t = to.chars();
            t.next()
                .map(|c| c.to_uppercase().chain(t).collect())
                .unwrap_or_default()
        } else {
            to.to_string()
        }
    };
    let lower: Vec<char> = chars.iter().flat_map(|c| c.to_lowercase()).collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = lower.get(i + 1).copied();
        let soft = next == Some('ь');
        let (to, used) = match lower[i] {
            'л' if soft => ("љ", 2),
            'н' if soft => ("њ", 2),
            'т' if soft => ("ћ", 2),
            'й' | 'э' => ("ј", 1),
            'ё' => ("је", 1),
            'ы' => ("и", 1),
            'ъ' | 'ь' => ("", 1),
            'я' if next.is_some_and(|n| "аеиоу".contains(n)) => ("ј", 1),
            'я' => ("ја", 1),
            _ => {
                out.push(c);
                i += 1;
                continue;
            }
        };
        out.push_str(&cased(to, c));
        i += used;
    }
    out
}

/// Standard base64, padded: what a `data:` URL carries.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.len();
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let v = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        for k in 0..4 {
            if k <= n {
                out.push(ABC[((v >> (18 - 6 * k)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Readings kept between runs, one file per chat: a photo is read once.
///
/// Keyed by the OCR model and the photo's own key (`tga-run` makes it from
/// the stored file's id, or a folder export's path and size), so a different
/// OCR reads everything again and a re-synced chat reads nothing twice.
pub struct Readings {
    path: Option<PathBuf>,
    map: HashMap<String, String>,
}

impl Readings {
    /// The readings kept at `path`; none yet if it is missing or unreadable.
    pub fn at(path: &Path) -> Self {
        let map = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .map(|o| {
                o.into_iter()
                    .filter_map(|(k, v)| Some((k, v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        Readings {
            path: Some(path.to_path_buf()),
            map,
        }
    }

    /// Keep nothing.
    pub fn none() -> Self {
        Readings {
            path: None,
            map: HashMap::new(),
        }
    }

    pub fn key(model: &str, photo: &str) -> String {
        crate::cache::key(&[model, photo])
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }

    pub fn extend(&mut self, read: HashMap<String, String>) {
        self.map.extend(read);
    }

    /// Write them back, whole or not at all (`cache::replace`): this one file
    /// holds every reading in the chat. A cache that cannot be written is a
    /// slower next run, not a failure of this one.
    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut keys: Vec<&String> = self.map.keys().collect();
        keys.sort();
        let mut o = serde_json::Map::new();
        for k in keys {
            o.insert(k.clone(), Value::String(self.map[k].clone()));
        }
        let _ = crate::cache::replace(path, &Value::Object(o).to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_is_kept_twice_and_the_rest_left_alone() {
        assert_eq!(cut_loops("a b b b b c"), "a b b c");
        assert_eq!(cut_loops("x  ne ne ne\nda da"), "x ne ne da da");
        assert_eq!(
            cut_loops("Prijava do 1. 12. do 1. 12. do 1. 12. do 1. 12. kraj"),
            "Prijava do 1. 12. do 1. 12. kraj"
        );
        assert_eq!(cut_loops("one two three"), "one two three");
    }

    #[test]
    fn russian_letters_become_serbian_only_where_they_appear() {
        assert_eq!(fix_cyrillic("ЛЬУДИ НЬЕГОВ ТЬЕРАТИ"), "ЉУДИ ЊЕГОВ ЋЕРАТИ");
        assert_eq!(fix_cyrillic("Йован и Эелена"), "Јован и Јелена");
        assert_eq!(fix_cyrillic("моя заставa"), "моја заставa");
        assert_eq!(fix_cyrillic("ЯВНИ ЯЕДНО"), "ЈАВНИ ЈЕДНО");
        assert_eq!(fix_cyrillic("Ёсте"), "Јесте");
        // No Russian letter: nothing changes, Latin included.
        assert_eq!(fix_cyrillic("Ђорђе and Ljubiša"), "Ђорђе and Ljubiša");
    }

    #[test]
    fn too_few_letters_is_no_text() {
        assert_eq!(clean("12:45  ✓✓  ok"), "");
        let poster = "Protest ispred Rektorata u petak u podne, svi dobrodošli";
        assert_eq!(clean(poster), poster);
    }

    #[test]
    fn only_jpeg_and_png_are_sent() {
        assert_eq!(mime_of(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(mime_of(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(mime_of(b"RIFF\0\0\0\0WEBP"), None);
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
    }

    #[test]
    fn the_ocr_is_found_beside_its_projector_and_template() {
        let dir = std::env::temp_dir().join(format!("tga-sum-ocr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sub = dir.join("paddleocr-vl");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("gemma-4-26B-A4B-it-qat-UD-Q4_K_XL.gguf"), b"").unwrap();
        assert!(files_in(&dir).is_none());
        for name in [
            "PaddleOCR-VL-1.5.gguf",
            "PaddleOCR-VL-1.5-mmproj.gguf",
            "chat_template.jinja",
        ] {
            std::fs::write(sub.join(name), b"").unwrap();
        }
        let (model, mmproj, template) = files_in(&dir).unwrap();
        assert!(model.ends_with("PaddleOCR-VL-1.5.gguf"));
        assert!(mmproj.ends_with("PaddleOCR-VL-1.5-mmproj.gguf"));
        assert!(template.unwrap().ends_with("chat_template.jinja"));

        // A download's projector named for nothing but itself: in the OCR's
        // own folder, it is the OCR's; beside another model's, it is not.
        std::fs::remove_file(sub.join("PaddleOCR-VL-1.5-mmproj.gguf")).unwrap();
        std::fs::write(sub.join("mmproj-F16.gguf"), b"").unwrap();
        let (_, mmproj, _) = files_in(&dir).unwrap();
        assert!(mmproj.ends_with("mmproj-F16.gguf"));
        std::fs::remove_dir_all(&sub).unwrap();
        for name in ["PaddleOCR-VL-1.5.gguf", "mmproj-gemma.gguf"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        assert!(files_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn readings_survive_a_round_trip() {
        let path =
            std::env::temp_dir().join(format!("tga-sum-readings-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut r = Readings::at(&path);
        let k = Readings::key("PaddleOCR-VL-1.5.gguf", "photos:77");
        assert_eq!(r.get(&k), None);
        r.extend(HashMap::from([(k.clone(), "Plakat".to_string())]));
        r.save();
        assert_eq!(Readings::at(&path).get(&k), Some("Plakat"));
        let _ = std::fs::remove_file(path);
    }
}
