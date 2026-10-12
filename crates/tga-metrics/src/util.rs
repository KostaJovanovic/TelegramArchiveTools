//! Small shared conversions, each of which has to agree with Python exactly.

use std::collections::HashMap;
use std::hash::Hash;

use chrono::NaiveDateTime;

/// `collections.Counter`, including the part that is easy to miss.
///
/// **Insertion order is load-bearing.** `most_common()` is a stable sort by
/// count descending, so equal counts come back in the order the keys were
/// first seen — and `most_common(n)` truncates *after* that. Ranking with a
/// plain `HashMap` puts ties in hash order, which differs between the two
/// implementations, differs between runs, and is invisible in every figure
/// except the one list that got truncated at 20.
#[derive(Debug, Clone, Default)]
pub struct Counter<K: Eq + Hash + Clone> {
    order: Vec<K>,
    counts: HashMap<K, i64>,
}

impl<K: Eq + Hash + Clone> Counter<K> {
    pub fn new() -> Self {
        Counter {
            order: Vec::new(),
            counts: HashMap::new(),
        }
    }

    pub fn add(&mut self, key: K, n: i64) {
        match self.counts.get_mut(&key) {
            Some(slot) => *slot += n,
            None => {
                self.order.push(key.clone());
                self.counts.insert(key, n);
            }
        }
    }

    pub fn bump(&mut self, key: K) {
        self.add(key, 1);
    }

    pub fn get(&self, key: &K) -> i64 {
        self.counts.get(key).copied().unwrap_or(0)
    }

    pub fn total(&self) -> i64 {
        self.counts.values().sum()
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.order.iter()
    }

    /// `Counter.most_common(n)` — count descending, ties in first-seen order.
    pub fn most_common(&self, n: Option<usize>) -> Vec<(K, i64)> {
        let mut items: Vec<(K, i64)> = self
            .order
            .iter()
            .map(|k| (k.clone(), self.counts[k]))
            .collect();
        // Stable, so ties keep insertion order — that is the whole point.
        items.sort_by_key(|item| std::cmp::Reverse(item.1));
        match n {
            Some(n) => items.into_iter().take(n).collect(),
            None => items,
        }
    }
}

/// `str.title()` — capitalise the first letter of every run of letters.
pub fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start_of_word = true;
    for ch in text.chars() {
        if ch.is_alphabetic() {
            if start_of_word {
                out.extend(ch.to_uppercase());
            } else {
                out.extend(ch.to_lowercase());
            }
            start_of_word = false;
        } else {
            out.push(ch);
            start_of_word = true;
        }
    }
    out
}

/// `round(x, 1)`, matching Python.
///
/// Python's `round` rounds half to even on the *decimal* expansion, and then
/// `json.dumps` writes the shortest float that round-trips. Rust's
/// `format!("{:.1}")` rounds the same way, so formatting and re-parsing lands
/// on the same f64 — where `(x * 10.0).round() / 10.0` would not, because that
/// rounds half away from zero and disagrees on every exact .x5.
pub fn round1(value: f64) -> f64 {
    format!("{value:.1}").parse().unwrap_or(value)
}

/// `datetime.isoformat(sep=" ", timespec="minutes")` — `2026-08-18 22:59`.
pub fn stamp_minutes(when: &NaiveDateTime) -> String {
    when.format("%Y-%m-%d %H:%M").to_string()
}

/// `str(Path(...))` — the separator the platform uses, not the one that was
/// typed.
///
/// `PurePath` keeps a path as parts and rejoins them with the OS separator, so
/// Python prints `N:\x` whether it was given `N:/x` or `N:\x`. Rust's
/// `PathBuf` preserves what it was handed, so the same folder reaches the
/// oracle spelled two ways depending on how the command line was written.
pub fn path_string(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text.replace('/', "\\")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_follows_python_not_the_schoolbook_rule() {
        // Every expected value here was read out of the Python interpreter,
        // not reasoned about — the rule is not "half rounds up" and it is not
        // purely "half rounds to even" either. What actually decides it is the
        // binary value the literal became: 49.45 is stored a hair above its
        // decimal and rounds up, 49.55 a hair below and rounds down, so two
        // numbers that look symmetrical both land on 49.5.
        assert_eq!(round1(0.25), 0.2);
        assert_eq!(round1(0.35), 0.3);
        assert_eq!(round1(49.54), 49.5);
        assert_eq!(round1(49.45), 49.5);
        assert_eq!(round1(49.55), 49.5);
        assert_eq!(round1(2.675), 2.7);
    }
}
