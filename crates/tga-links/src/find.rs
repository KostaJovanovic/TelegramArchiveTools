//! Every link in a text, cleaned: ReVision's `links.py`. No network here.
//!
//! A link is its address cleaned ([`clean`]): tracking parameters, `www.` /
//! `m.`, `youtu.be` and Google's share suffixes off, so the same page shared
//! several times is one link, fetched once.

use url::Url;

/// What a link is, by its host: which way it is read, or that it is not.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// An article or any other page: its text.
    #[default]
    Page,
    /// A public Google Doc, Sheet or Slides, exported as text.
    Gdoc,
    /// A public Drive file, downloaded and read where it is a document.
    Drive,
    /// A video page: the public preview. ReVision reads YouTube's subtitles
    /// through yt-dlp, which is Python; there is no such reader here.
    Video,
    /// The public preview any visitor sees; X through its public embed.
    Social,
    /// A meeting room: nothing to read.
    Meet,
    /// A payment page: nothing to read.
    Pay,
    /// A Telegram link: an invite is a credential, and a message link reads
    /// nothing a summary does not already have. Never fetched (2026-10-20,
    /// the account holder's choice).
    Telegram,
}

impl Kind {
    /// Whether a link of this kind is ever fetched.
    pub fn fetched(self) -> bool {
        !matches!(self, Kind::Meet | Kind::Pay | Kind::Telegram)
    }
}

/// One link: its cleaned address, which is what it is known by, and its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub host: String,
    pub kind: Kind,
}

/// The characters a link ends before, as ReVision's `URL` pattern has them.
fn ends_link(c: char) -> bool {
    c.is_whitespace() || "<>\"'()[]{}|\\^`".contains(c)
}

/// What is trimmed off a link's end: sentence punctuation and quotes.
const TRAIL: &str = ".,;:!?…»«“”‘’'\"*_";

/// Query parameters that only say who shared the link or from where.
const TRACKING: [&str; 21] = [
    "fbclid", "gclid", "dclid", "igsh", "igshid", "si", "feature", "ref", "ref_src", "ref_url",
    "s", "t", "share_id", "_r", "smid", "cmpid", "ocid", "usp", "from", "lang", "mibextid",
];

fn tracking(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("utm_")
        || (key.starts_with("mc_")
            && key.len() > 3
            && key[3..].chars().all(|c| c.is_ascii_lowercase()))
        || TRACKING.contains(&key.as_str())
}

/// Query parameters a site needs to know which page it is.
fn kept_query(host: &str) -> &'static [&'static str] {
    if within(host, "youtube.com") {
        &["v", "list"]
    } else if within(host, "google.com") {
        &["id", "q"]
    } else {
        &[]
    }
}

/// `host` is `domain` or one of its subdomains.
pub(crate) fn within(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.ends_with('.'))
}

/// `www.`, `www2.`, `m.`, `mobile.` and `amp.` off the front.
fn strip_host(host: &str) -> &str {
    let (head, rest) = host.split_once('.').unwrap_or((host, ""));
    let www = head
        .strip_prefix("www")
        .is_some_and(|n| n.len() <= 1 && n.chars().all(|c| c.is_ascii_digit()));
    if !rest.is_empty() && (www || matches!(head, "m" | "mobile" | "amp")) {
        rest
    } else {
        host
    }
}

const KINDS: [(Kind, &[&str]); 7] = [
    (
        Kind::Meet,
        &[
            "meet.google.com",
            "zoom.us",
            "teams.microsoft.com",
            "meet.jit.si",
        ],
    ),
    (Kind::Telegram, &["t.me", "telegram.me", "telegram.dog"]),
    (Kind::Gdoc, &["docs.google.com"]),
    (Kind::Drive, &["drive.google.com"]),
    (
        Kind::Video,
        &["youtube.com", "youtu.be", "tiktok.com", "vimeo.com"],
    ),
    (
        Kind::Social,
        &[
            "instagram.com",
            "x.com",
            "twitter.com",
            "facebook.com",
            "fb.com",
            "fb.watch",
            "reddit.com",
            "threads.net",
            "linkedin.com",
            "bsky.app",
        ],
    ),
    (Kind::Pay, &["paypal.com", "paypal.me", "patreon.com"]),
];

/// The kind a host's links are; the first match wins.
pub fn kind_of(host: &str) -> Kind {
    KINDS
        .iter()
        .find(|(_, hosts)| hosts.iter().any(|h| within(host, h)))
        .map_or(Kind::Page, |(k, _)| *k)
}

/// The cleaned address and its host, or `None` for something that is not a
/// web address. The same page shared from different places gets the same
/// cleaned address.
pub fn clean(url: &str) -> Option<(String, String)> {
    let url = url.trim_end_matches(|c| TRAIL.contains(c));
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = strip_host(&parsed.host_str()?.to_lowercase()).to_string();
    if !host.contains('.') {
        return None;
    }
    let mut host = host;
    let mut path = parsed.path().trim_end_matches('/').to_string();
    let mut query: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    // youtu.be/ID and /shorts/ID -> youtube.com/watch?v=ID
    if host == "youtu.be" && path.len() > 1 {
        query = vec![("v".into(), path.trim_matches('/').to_string())];
        host = "youtube.com".into();
        path = "/watch".into();
    }
    if host == "youtube.com" {
        if let Some(id) = path.strip_prefix("/shorts/") {
            query = vec![("v".into(), id.split('/').next().unwrap_or(id).to_string())];
            path = "/watch".into();
        }
    }
    // .../d/<id>/edit?usp=sharing -> .../d/<id>
    if host == "docs.google.com" {
        if let Some(cut) = gdoc_path(&path) {
            path = cut;
        }
        query.clear();
    }
    // open?id=<id> and file/d/<id>/view -> file/d/<id>
    if host == "drive.google.com" {
        let folder = path.contains("/folders/");
        let id = ["/file/d/", "/folders/"]
            .iter()
            .find_map(|p| path.split_once(p).map(|(_, rest)| rest))
            .map(|rest| rest.split('/').next().unwrap_or(rest).to_string())
            .or_else(|| {
                query
                    .iter()
                    .find(|(k, _)| k == "id")
                    .map(|(_, v)| v.clone())
            });
        if let Some(id) = id.filter(|i| !i.is_empty()) {
            path = format!(
                "{}{id}",
                if folder {
                    "/drive/folders/"
                } else {
                    "/file/d/"
                }
            );
        }
        query.clear();
    }
    let keep = kept_query(&host);
    query.retain(|(k, _)| keep.contains(&k.as_str()) || !tracking(k));
    query.sort();
    let mut out = Url::parse(&format!("https://{host}")).ok()?;
    out.set_path(&path);
    if !query.is_empty() {
        out.query_pairs_mut().extend_pairs(query);
    }
    let mut s = out.to_string();
    if path.is_empty() && query_is_empty(&out) {
        // `Url` writes the root as `/`; ReVision's address has no slash.
        s = s.trim_end_matches('/').to_string();
    }
    Some((s, host))
}

fn query_is_empty(u: &Url) -> bool {
    u.query().is_none_or(str::is_empty)
}

/// A Google Docs path cut after the document's id.
fn gdoc_path(path: &str) -> Option<String> {
    let mut parts = path.split('/').skip(1);
    let what = parts.next()?;
    if !matches!(
        what,
        "document" | "spreadsheets" | "presentation" | "forms" | "drawings"
    ) {
        return None;
    }
    let mut out = format!("/{what}");
    let mut next = parts.next()?;
    if next == "u" {
        out.push_str(&format!("/u/{}", parts.next()?));
        next = parts.next()?;
    }
    if next != "d" {
        return None;
    }
    out.push_str("/d");
    let mut id = parts.next()?;
    if id == "e" {
        out.push_str("/e");
        id = parts.next()?;
    }
    out.push_str(&format!("/{id}"));
    Some(out)
}

/// One address, as a link: an entity's `href`, say.
pub fn link(url: &str) -> Option<Link> {
    let (url, host) = clean(url)?;
    let kind = kind_of(&host);
    Some(Link { url, host, kind })
}

/// The links in a text, each once, in the order they appear.
pub fn found(text: &str) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = next_start(&lower, at) {
        let end = text[i..].find(ends_link).map_or(text.len(), |n| i + n);
        if let Some(l) = link(&text[i..end]) {
            if !out.iter().any(|o| o.url == l.url) {
                out.push(l);
            }
        }
        at = end.max(i + 1);
    }
    out
}

/// Where the next `http://` or `https://` starts, at or after `at`.
fn next_start(lower: &str, at: usize) -> Option<usize> {
    let rest = lower.get(at..)?;
    let a = rest.find("http://");
    let b = rest.find("https://");
    match (a, b) {
        (Some(a), Some(b)) => Some(at + a.min(b)),
        (Some(a), None) => Some(at + a),
        (None, Some(b)) => Some(at + b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(u: &str) -> String {
        clean(u).unwrap().0
    }

    #[test]
    fn tracking_and_mobile_prefixes_are_cleaned_off() {
        assert_eq!(
            c("http://www.novosti.rs/vesti/123/?utm_source=fb&fbclid=x&id=4"),
            "https://novosti.rs/vesti/123?id=4"
        );
        assert_eq!(c("https://m.vreme.com/a/b."), "https://vreme.com/a/b");
        assert_eq!(c("https://Example.com/"), "https://example.com");
    }

    #[test]
    fn the_same_video_shared_three_ways_is_one_link() {
        let want = "https://youtube.com/watch?v=abc123";
        assert_eq!(c("https://youtu.be/abc123?si=xyz"), want);
        assert_eq!(
            c("https://www.youtube.com/watch?v=abc123&feature=share"),
            want
        );
        assert_eq!(c("https://youtube.com/shorts/abc123"), want);
    }

    #[test]
    fn google_documents_and_drive_files_lose_their_share_suffixes() {
        assert_eq!(
            c("https://docs.google.com/document/d/1AbC_d-9/edit?usp=sharing"),
            "https://docs.google.com/document/d/1AbC_d-9"
        );
        assert_eq!(
            c("https://drive.google.com/file/d/XYZ/view?usp=drive_link"),
            "https://drive.google.com/file/d/XYZ"
        );
        assert_eq!(
            c("https://drive.google.com/open?id=XYZ"),
            "https://drive.google.com/file/d/XYZ"
        );
    }

    #[test]
    fn a_link_is_found_in_text_once_without_its_punctuation() {
        let found = found(
            "Vidi (https://n1info.rs/vesti/a1), i HTTPS://n1info.rs/vesti/a1. A t.me: https://t.me/+abc",
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].url, "https://n1info.rs/vesti/a1");
        assert_eq!(found[0].kind, Kind::Page);
        assert_eq!(found[1].kind, Kind::Telegram);
        assert!(!found[1].kind.fetched());
    }

    #[test]
    fn kinds_follow_the_host() {
        assert_eq!(kind_of("docs.google.com"), Kind::Gdoc);
        assert_eq!(kind_of("old.reddit.com"), Kind::Social);
        assert_eq!(kind_of("meet.google.com"), Kind::Meet);
        assert_eq!(kind_of("notx.com"), Kind::Page);
        assert!(clean("https://localhost/a").is_none());
        assert!(clean("mailto:a@b.rs").is_none());
    }
}
