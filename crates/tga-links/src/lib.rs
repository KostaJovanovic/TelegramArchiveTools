//! The links a chat shared, read for a summary: ReVision's links system
//! (`studio/links.py`, `studio/linkfetch.py`), ported.
//!
//! 1. **Find** ([`find`]): every link in a message, cleaned, so the same page
//!    shared five times is one link. No network.
//! 2. **Fetch** ([`Fetcher`]): by its kind — a page's text and metadata, a
//!    public Google Doc as text, a public Drive file and the text inside it,
//!    the public preview of a social or video page. Polite, never this
//!    machine's network, nothing logs in.
//! 3. **Keep** ([`Pages`]): one [`Card`] per link, per chat, beside the
//!    summaries. A link is fetched once; only one that [`Status::Failed`]
//!    for a reason that may pass is tried again.
//!
//! **This is the analyser's only way out to the internet,** and it is taken
//! only when Settings › Summaries › Read linked pages is on. What it reads
//! goes into the message's row, and the model reads it beside the message.
//!
//! **Narration names counts, never a link**: an address is part of what
//! somebody wrote.

pub mod fetch;
pub mod find;
mod page;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use fetch::{Fetcher, ReadDoc};
pub use find::{found, link, Kind, Link};

/// What fetching a link came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Read: the card has what the page says.
    Read,
    /// A timeout, a refused connection, a server error: may pass, so a later
    /// run tries again.
    #[default]
    Failed,
    /// A login wall, or a page that shows programs nothing.
    Login,
    /// A private Google file.
    #[serde(rename = "noaccess")]
    NoAccess,
    /// 404 or 410.
    Gone,
    /// A Drive folder: only marked.
    Folder,
    /// Nothing to read (a meeting room, a payment page, a Telegram link), or
    /// an address that is not on the internet.
    Skip,
}

impl Status {
    /// The word for it, as it is kept.
    pub fn word(self) -> &'static str {
        match self {
            Status::Read => "read",
            Status::Failed => "failed",
            Status::Login => "login",
            Status::NoAccess => "noaccess",
            Status::Gone => "gone",
            Status::Folder => "folder",
            Status::Skip => "skip",
        }
    }
}

/// One link, fetched: ReVision's page card, without the images.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Card {
    /// The cleaned address it is known by ([`find::clean`]).
    pub link: String,
    /// Where the page really is, after redirects.
    pub url: String,
    pub kind: Kind,
    pub status: Status,
    /// Why it is not read; empty when it is.
    pub why: String,
    pub title: String,
    pub site: String,
    pub author: String,
    /// When the page says it was published, `YYYY-MM-DD`.
    pub date: String,
    pub description: String,
    /// The main text; the description when there was no more.
    pub text: String,
    /// When it was fetched, local time to the minute.
    pub at: String,
}

impl Card {
    /// Whether there is anything here for the model to read.
    pub fn readable(&self) -> bool {
        self.status == Status::Read && !(self.title.is_empty() && self.text.is_empty())
    }
}

/// The cards kept between runs, one file per chat: a link is fetched once.
pub struct Pages {
    path: Option<PathBuf>,
    map: BTreeMap<String, Card>,
}

impl Pages {
    /// The cards kept at `path`; none yet if it is missing or unreadable.
    pub fn at(path: &Path) -> Pages {
        let map = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<BTreeMap<String, Card>>(&t).ok())
            .unwrap_or_default();
        Pages {
            path: Some(path.to_path_buf()),
            map,
        }
    }

    /// Keep nothing.
    pub fn none() -> Pages {
        Pages {
            path: None,
            map: BTreeMap::new(),
        }
    }

    pub fn get(&self, link: &str) -> Option<&Card> {
        self.map.get(link)
    }

    pub fn put(&mut self, card: Card) {
        self.map.insert(card.link.clone(), card);
    }

    /// Whether `link` is to be fetched: never tried, or failed for a reason
    /// that may pass. A login wall, a private file or a page that is gone
    /// stays as it is (ReVision's `AGAIN`).
    pub fn wants(&self, link: &Link) -> bool {
        link.kind.fetched()
            && self
                .map
                .get(&link.url)
                .is_none_or(|c| c.status == Status::Failed)
    }

    /// Write them back, whole or not at all: a temporary file renamed over
    /// the old one. A cache that cannot be written is a slower next run,
    /// not a failure of this one.
    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(text) = serde_json::to_string(&self.map) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_is_kept_and_only_a_failed_one_is_fetched_again() {
        let path = std::env::temp_dir().join(format!("tga-links-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut pages = Pages::at(&path);
        let read = link("https://portal.rs/a").unwrap();
        let gone = link("https://portal.rs/b").unwrap();
        let later = link("https://portal.rs/c").unwrap();
        assert!(pages.wants(&read));
        for (l, status) in [
            (&read, Status::Read),
            (&gone, Status::Gone),
            (&later, Status::Failed),
        ] {
            pages.put(Card {
                link: l.url.clone(),
                status,
                title: "T".into(),
                ..Card::default()
            });
        }
        pages.save();
        let pages = Pages::at(&path);
        assert!(pages.get(&read.url).unwrap().readable());
        assert!(!pages.wants(&read));
        assert!(!pages.wants(&gone));
        assert!(pages.wants(&later));
        assert!(!pages.wants(&link("https://t.me/x").unwrap()));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_status_is_kept_as_revisions_word() {
        let s = serde_json::to_string(&Status::NoAccess).unwrap();
        assert_eq!(s, "\"noaccess\"");
        assert_eq!(Status::NoAccess.word(), "noaccess");
    }
}
