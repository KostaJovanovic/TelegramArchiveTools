//! Notes kept between runs, one file per part.
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
        let Some(dir) = &self.dir else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let v = json!({ "notes": notes.iter().map(Note::to_json).collect::<Vec<_>>() });
        let _ = std::fs::write(dir.join(format!("{key}.json")), v.to_string());
    }
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
}
