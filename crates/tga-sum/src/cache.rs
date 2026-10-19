//! Notes kept between runs, one file per part — and each document's
//! explanation, one file per document.
//!
//! **Keyed on what the model read**, not on the dates: the part's rendered
//! text, plus the prompt's version, the model file and everything else that
//! shapes the answer. A sync that edits one message changes one part's key;
//! a range that overlaps an earlier one reuses every part the two share. A
//! stale entry is never wrong, only unused.
//!
//! The hash is FNV-1a, written out here. It needs to be stable across builds
//! and Rust versions, which `DefaultHasher` does not promise, and nothing here
//! is adversarial: a collision costs a wrong set of notes for one part of a
//! summary the reader can redo.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use crate::Note;

#[derive(Debug, Clone, Default)]
pub struct Cache {
    dir: Option<PathBuf>,
}

impl Cache {
    /// Notes kept in `dir`, created when first written.
    pub fn at(dir: &Path) -> Self {
        Cache {
            dir: Some(dir.to_path_buf()),
        }
    }

    /// Keep nothing.
    pub fn none() -> Self {
        Cache { dir: None }
    }

    pub fn get(&self, key: &str) -> Option<Vec<Note>> {
        let path = self.dir.as_ref()?.join(format!("{key}.json"));
        let text = std::fs::read_to_string(path).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        Some(Note::all_from(&v))
    }

    /// Keep `notes` under `key`. A cache that cannot be written is a slower
    /// next run, not a failure of this one.
    pub fn put(&self, key: &str, notes: &[Note]) {
        let v = json!({ "notes": notes.iter().map(Note::to_json).collect::<Vec<_>>() });
        self.keep(key, &v);
    }

    /// A text kept under `key`: a document's explanation.
    pub fn text(&self, key: &str) -> Option<String> {
        let path = self.dir.as_ref()?.join(format!("{key}.json"));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        Some(v.get("text")?.as_str()?.to_string())
    }

    pub fn put_text(&self, key: &str, text: &str) {
        self.keep(key, &json!({ "text": text }));
    }

    fn keep(&self, key: &str, v: &Value) {
        let Some(dir) = &self.dir else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let _ = replace(&dir.join(format!("{key}.json")), &v.to_string());
    }
}

/// Write `text` as the whole of `path`: beside it first, then renamed over
/// it, so a run killed mid-write leaves the old file or the new one and never
/// half of either. A half-written `photos.json` does not parse, and reads as
/// no readings at all — every photo in the chat read again.
///
/// The side file's name is unique to the write, so a summary and a question
/// keeping the same part at once do not write into one file.
pub(crate) fn replace(path: &Path, text: &str) -> std::io::Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let mut part = path.as_os_str().to_owned();
    part.push(format!(
        ".{}-{}.part",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let part = PathBuf::from(part);
    let written = std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

/// A key from every input that shapes an answer, in order.
pub fn key(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.bytes().chain(std::iter::once(0xff)) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_stable_and_every_input_moves_it() {
        // Pinned: a change here strands every cached part on disk.
        assert_eq!(key(&["a"]), key(&["a"]));
        assert_eq!(key(&[]), "cbf29ce484222325");
        assert_ne!(key(&["ab", "c"]), key(&["a", "bc"]));
        assert_ne!(key(&["x", "1"]), key(&["x", "2"]));
    }

    #[test]
    fn notes_come_back_as_they_went_in() {
        let dir = std::env::temp_dir().join(format!("tga-sum-cache-{}", std::process::id()));
        let cache = Cache::at(&dir);
        let notes = vec![Note {
            day: "2024-12-02".into(),
            weight: "major".into(),
            what: "A strike was called.".into(),
            ids: vec![1, 2],
        }];
        assert!(cache.get("k").is_none());
        cache.put("k", &notes);
        assert_eq!(cache.get("k"), Some(notes));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_replaced_file_is_the_new_text_and_leaves_nothing_beside_it() {
        let dir = std::env::temp_dir().join(format!("tga-sum-replace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("photos.json");
        replace(&path, "{\"a\":\"1\"}").unwrap();
        replace(&path, "{\"a\":\"2\"}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":\"2\"}");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, ["photos.json"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
