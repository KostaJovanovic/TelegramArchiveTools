//! Fetching a link, by its kind: ReVision's `linkfetch.py`.
//!
//! **Polite**: several sites at once ([`WORKERS`]), but each host's links one
//! after another, one request at a time to any host, with [`GAP`] between
//! them, a timeout and a size cap. Nothing logs in anywhere.
//!
//! **Never this machine or its network.** A message can link anything,
//! `http://127.0.0.1:8080` — the model server — or a router's page included.
//! Every hop is checked before it is requested ([`public`]): an address that
//! is loopback, private, link-local or otherwise not on the internet is not
//! fetched, and redirects are followed here, one at a time, so a public page
//! cannot redirect into the local network either.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use url::Url;

use crate::find::{within, Kind, Link};
use crate::page;
use crate::{Card, Status};

/// A desktop browser: what most sites serve their article to.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like \
                  Gecko) Chrome/131.0 Safari/537.36";
/// What link previews use: social sites answer it with the public preview.
const PREVIEW_UA: &str =
    "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)";
const TIMEOUT: Duration = Duration::from_secs(25);
/// Sites fetched at once.
pub const WORKERS: usize = 4;
/// Between two requests to the same host.
const GAP: Duration = Duration::from_secs(2);
const PAGE_MAX: u64 = 10 << 20;
/// A Drive file or a linked file.
const FILE_MAX: u64 = 100 << 20;
const REDIRECTS: usize = 10;
/// A site's stock preview, not the post.
const PLACEHOLDERS: [&str; 2] = [
    "Visit TikTok to discover videos!",
    "Visit TikTok to discover videos",
];

/// Why a link gave no card.
#[derive(Debug)]
pub(crate) struct Failed {
    pub status: Status,
    pub why: String,
}

fn failed(status: Status, why: impl Into<String>) -> Failed {
    Failed {
        status,
        why: why.into(),
    }
}

/// What a request brought back.
pub(crate) struct Got {
    /// Where it ended, after redirects.
    pub url: String,
    /// The type, lower case, without its parameters.
    pub ctype: String,
    pub charset: Option<String>,
    pub disposition: String,
    pub body: Vec<u8>,
}

/// Reads a downloaded document's text: `tga-docs`, from the caller, so this
/// crate links no PDF parser. Given the extension (`pdf`, `docx`, `txt`,
/// `md`, `csv`) and the bytes.
pub type ReadDoc = dyn Fn(&str, &[u8]) -> String + Send + Sync;

pub struct Fetcher {
    agent: ureq::Agent,
    /// `Accept-Language`: the summary's language first, then English; none
    /// when there is no one language, and a site serves its own.
    lang: Option<String>,
    /// Per host, when its last request ended; its lock is held while a
    /// request to it is open, so a host gets one at a time.
    hosts: Mutex<HashMap<String, Arc<Mutex<Option<Instant>>>>>,
    read_doc: Box<ReadDoc>,
}

impl Fetcher {
    /// `lang` is a language code (`sr`, `en`).
    pub fn new(lang: Option<&str>, read_doc: Box<ReadDoc>) -> Fetcher {
        let tls = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_global(Some(TIMEOUT))
            // Followed here, hop by hop, each checked by `public`.
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(UA)
            .build()
            .into();
        Fetcher {
            agent,
            lang: lang.map(|l| format!("{l},en;q=0.7")),
            hosts: Mutex::new(HashMap::new()),
            read_doc,
        }
    }

    /// Every link in `links`, several sites at once and each site's links one
    /// after another, the sites with most links first so that no one long
    /// site is left for the end. `done` gets each card as it is made, on the
    /// calling thread; a stop lets the links being fetched finish and starts
    /// no more.
    pub fn fetch_all(&self, links: Vec<Link>, cancel: &AtomicBool, done: &mut dyn FnMut(Card)) {
        let mut by_host: HashMap<String, Vec<Link>> = HashMap::new();
        for l in links {
            by_host.entry(l.host.clone()).or_default().push(l);
        }
        let mut sites: Vec<Vec<Link>> = by_host.into_values().collect();
        sites.sort_by(|a, b| {
            b.len()
                .cmp(&a.len())
                .then_with(|| a[0].host.cmp(&b[0].host))
        });
        let queue = Mutex::new(VecDeque::from(sites));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|s| {
            for _ in 0..WORKERS {
                let tx = tx.clone();
                let queue = &queue;
                s.spawn(move || loop {
                    let Some(site) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front()
                    else {
                        return;
                    };
                    for link in site {
                        if cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        if tx.send(self.fetch(&link)).is_err() {
                            return;
                        }
                    }
                });
            }
            drop(tx);
            for card in rx {
                done(card);
            }
        });
    }

    /// One link's card. One bad page never stops the others: a panic in a
    /// page's reader is a failed link.
    pub fn fetch(&self, link: &Link) -> Card {
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.by_kind(link)))
            .unwrap_or_else(|_| Err(failed(Status::Failed, "the page could not be read")));
        let mut card = match read {
            Ok(card) => card,
            Err(f) => Card {
                status: f.status,
                why: f.why,
                ..Card::default()
            },
        };
        card.link = link.url.clone();
        card.kind = link.kind;
        if card.url.is_empty() {
            card.url = link.url.clone();
        }
        card.at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        card
    }

    fn by_kind(&self, link: &Link) -> Result<Card, Failed> {
        let mut card = match link.kind {
            Kind::Meet | Kind::Pay | Kind::Telegram => {
                return Err(failed(Status::Skip, "nothing to read"));
            }
            Kind::Gdoc => self.gdoc(&link.url)?,
            Kind::Drive => self.drive(&link.url)?,
            // ReVision reads a video's subtitles through yt-dlp, and its
            // fallback for what yt-dlp cannot read is the preview.
            Kind::Video | Kind::Social => self.social(&link.url, &link.host)?,
            Kind::Page => self.page(&link.url)?,
        };
        card.status = Status::Read;
        Ok(card)
    }

    /// `(response, body)`, one host one request at a time.
    pub(crate) fn get(&self, url: &str, ua: Option<&str>, cap: u64) -> Result<Got, Failed> {
        let host = Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        let slot = self
            .hosts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(host)
            .or_default()
            .clone();
        let mut last = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = *last {
            let wait = (t + GAP).saturating_duration_since(Instant::now());
            std::thread::sleep(wait);
        }
        let got = self.request(url, ua, cap);
        *last = Some(Instant::now());
        got
    }

    fn request(&self, url: &str, ua: Option<&str>, cap: u64) -> Result<Got, Failed> {
        let mut at = Url::parse(url).map_err(|_| failed(Status::Failed, "not an address"))?;
        let mut hops = 0;
        let mut response = loop {
            public(&at)?;
            let mut req = self.agent.get(at.as_str());
            if let Some(lang) = &self.lang {
                req = req.header("Accept-Language", lang);
            }
            if let Some(ua) = ua {
                req = req.header("User-Agent", ua);
            }
            let r = req.call().map_err(transport)?;
            let status = r.status().as_u16();
            let location = r
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            match location {
                Some(to) if (300..400).contains(&status) => {
                    hops += 1;
                    if hops > REDIRECTS {
                        return Err(failed(Status::Failed, "too many redirects"));
                    }
                    at = at
                        .join(&to)
                        .map_err(|_| failed(Status::Failed, "a bad redirect"))?;
                }
                _ => break r,
            }
        };
        let status = response.status().as_u16();
        let host = at.host_str().unwrap_or_default().to_string();
        match status {
            404 | 410 => return Err(failed(Status::Gone, format!("HTTP {status}"))),
            // Google: a private file; elsewhere a login or a wall against
            // programs.
            401 | 403 if host.contains("google") => {
                return Err(failed(Status::NoAccess, format!("HTTP {status}")))
            }
            401 | 403 => return Err(failed(Status::Login, format!("HTTP {status}"))),
            s if s >= 400 => return Err(failed(Status::Failed, format!("HTTP {status}"))),
            _ => {}
        }
        if host == "accounts.google.com" {
            return Err(failed(Status::NoAccess, "sign-in page"));
        }
        let path = at.path().to_ascii_lowercase();
        if path.contains("/login") || path.contains("/accounts/login") {
            return Err(failed(Status::Login, "sign-in page"));
        }
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string()
        };
        let content_type = header("content-type");
        let disposition = header("content-disposition");
        let (ctype, params) = content_type
            .split_once(';')
            .unwrap_or((content_type.as_str(), ""));
        let charset = params.split(';').find_map(|p| {
            let (k, v) = p.split_once('=')?;
            (k.trim().eq_ignore_ascii_case("charset"))
                .then(|| v.trim().trim_matches('"').to_string())
        });
        let body = response
            .body_mut()
            .with_config()
            .limit(cap)
            .read_to_vec()
            .map_err(|e| match e {
                ureq::Error::BodyExceedsLimit(_) => {
                    failed(Status::Failed, format!("over {} MB", cap >> 20))
                }
                e => transport(e),
            })?;
        Ok(Got {
            url: at.to_string(),
            ctype: ctype.trim().to_ascii_lowercase(),
            charset,
            disposition,
            body,
        })
    }

    // -- by kind -------------------------------------------------------------

    fn page(&self, url: &str) -> Result<Card, Failed> {
        let url = unwrap_google(url);
        let got = self.get(&url, None, PAGE_MAX)?;
        if !got.ctype.contains("html") && !got.ctype.contains("xml") {
            return Ok(self.file_card(&got, None));
        }
        let html = page::decode(&got.body, got.charset.as_deref());
        page::card(&got.url, &html, false)
    }

    fn social(&self, url: &str, host: &str) -> Result<Card, Failed> {
        if within(host, "x.com") || within(host, "twitter.com") {
            let mut embed =
                Url::parse("https://publish.twitter.com/oembed").expect("a fixed address");
            embed
                .query_pairs_mut()
                .append_pair("omit_script", "1")
                .append_pair("url", url);
            let got = self.get(embed.as_str(), None, PAGE_MAX)?;
            let o: serde_json::Value = serde_json::from_slice(&got.body)
                .map_err(|_| failed(Status::Failed, "not an embed"))?;
            let html = o["html"].as_str().unwrap_or_default();
            return Ok(Card {
                url: url.to_string(),
                title: o["author_name"].as_str().unwrap_or_default().to_string(),
                site: "X".into(),
                text: page::tags_off(html),
                ..Card::default()
            });
        }
        let got = self.get(url, Some(PREVIEW_UA), PAGE_MAX)?;
        let html = page::decode(&got.body, got.charset.as_deref());
        let card = page::card(&got.url, &html, true)?;
        let stock = |s: &str| {
            PLACEHOLDERS
                .iter()
                .any(|p| s.trim().eq_ignore_ascii_case(p))
        };
        if card.description.is_empty() || stock(&card.description) || stock(&card.title) {
            // Only the site's name or its stock line: a login wall, a private
            // post, a page only the app shows.
            return Err(failed(Status::Login, "no public preview"));
        }
        Ok(card)
    }

    fn gdoc(&self, url: &str) -> Result<Card, Failed> {
        let parsed = Url::parse(url).map_err(|_| failed(Status::Failed, "not an address"))?;
        let parts: Vec<&str> = parsed.path().split('/').filter(|p| !p.is_empty()).collect();
        let what = parts.first().copied().unwrap_or_default();
        let published = parts.contains(&"e");
        let id = parts
            .iter()
            .position(|p| *p == "d")
            .and_then(|i| parts.get(i + 1 + usize::from(published)))
            .copied();
        let export = match what {
            "document" => "export?format=txt",
            "spreadsheets" => "export?format=csv",
            "presentation" => "export/txt",
            "drawings" => "export/png",
            _ => return self.page(url),
        };
        let Some(id) = id.filter(|_| !published) else {
            return self.page(url);
        };
        let got = self.get(
            &format!("https://docs.google.com/{what}/d/{id}/{export}"),
            None,
            FILE_MAX,
        )?;
        if got.ctype.contains("html") {
            return Err(failed(Status::NoAccess, "not public"));
        }
        let name = filename(&got.disposition).unwrap_or_else(|| id.to_string());
        if what == "drawings" {
            return Ok(self.file_card(&got, Some(stem(&name))));
        }
        let text = String::from_utf8_lossy(&got.body);
        Ok(Card {
            url: url.to_string(),
            title: stem(&name),
            site: "Google Docs".into(),
            text: text.trim_start_matches('\u{feff}').trim().to_string(),
            ..Card::default()
        })
    }

    fn drive(&self, url: &str) -> Result<Card, Failed> {
        let Some(id) = url
            .split_once("/file/d/")
            .map(|(_, id)| id.trim_matches('/'))
        else {
            return Err(failed(Status::Folder, "a Drive folder"));
        };
        let got = self.get(
            &format!(
                "https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t"
            ),
            None,
            FILE_MAX,
        )?;
        if got.ctype.contains("html") {
            return Err(failed(Status::NoAccess, "not public"));
        }
        let mut card = self.file_card(&got, None);
        card.url = url.to_string();
        card.site = "Google Drive".into();
        Ok(card)
    }

    /// A downloaded file: a document's text read, anything else only named.
    fn file_card(&self, got: &Got, title: Option<String>) -> Card {
        let name = filename(&got.disposition)
            .or_else(|| {
                Url::parse(&got.url)
                    .ok()?
                    .path_segments()?
                    .next_back()
                    .map(|s| {
                        percent_encoding::percent_decode_str(s)
                            .decode_utf8_lossy()
                            .into_owned()
                    })
            })
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "file".into());
        let mut ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if got.ctype == "application/pdf" {
            ext = "pdf".into();
        }
        let text = if matches!(ext.as_str(), "pdf" | "docx" | "txt" | "md" | "csv") {
            (self.read_doc)(&ext, &got.body).trim().to_string()
        } else {
            String::new()
        };
        Card {
            url: got.url.clone(),
            title: title.unwrap_or_else(|| stem(&name)),
            description: name,
            text,
            ..Card::default()
        }
    }
}

/// Google's redirect and AMP wrapper: the page they point to.
fn unwrap_google(url: &str) -> String {
    let Ok(u) = Url::parse(url) else {
        return url.to_string();
    };
    let host = u.host_str().unwrap_or_default();
    let google = host.starts_with("google.") || host.contains(".google.");
    if google && u.path() == "/url" {
        if let Some((_, to)) = u.query_pairs().find(|(k, _)| k == "q" || k == "url") {
            return to.into_owned();
        }
    }
    if let Some(rest) = u.path().strip_prefix("/amp/s/").filter(|_| google) {
        return format!("https://{rest}");
    }
    url.to_string()
}

/// The name a `Content-Disposition` gives.
fn filename(disposition: &str) -> Option<String> {
    if let Some((_, rest)) = disposition.split_once("filename*=UTF-8''") {
        let v = rest.split(';').next().unwrap_or(rest).trim();
        return Some(
            percent_encoding::percent_decode_str(v)
                .decode_utf8_lossy()
                .trim()
                .to_string(),
        );
    }
    let (_, rest) = disposition.split_once("filename=")?;
    let v = rest
        .split(';')
        .next()
        .unwrap_or(rest)
        .trim()
        .trim_matches('"');
    Some(
        percent_encoding::percent_decode_str(v)
            .decode_utf8_lossy()
            .trim()
            .to_string(),
    )
}

fn stem(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_string()
}

fn transport(e: ureq::Error) -> Failed {
    let why = match e {
        ureq::Error::Timeout(_) => "timeout".to_string(),
        ureq::Error::HostNotFound => "host not found".to_string(),
        ureq::Error::ConnectionFailed => "connection failed".to_string(),
        e => {
            let s = e.to_string();
            s.chars().take(200).collect()
        }
    };
    failed(Status::Failed, why)
}

/// Whether `url` is on the internet, and not this machine or its network.
/// A name is resolved first, and every address it gives must be public.
pub(crate) fn public(url: &Url) -> Result<(), Failed> {
    let private = || failed(Status::Skip, "not on the internet");
    match url.host() {
        Some(url::Host::Ipv4(ip)) if !global(IpAddr::V4(ip)) => Err(private()),
        Some(url::Host::Ipv6(ip)) if !global(IpAddr::V6(ip)) => Err(private()),
        Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => Ok(()),
        Some(url::Host::Domain(name)) => {
            let name = name.to_ascii_lowercase();
            const LOCAL: [&str; 6] = [
                "localhost",
                "local",
                "internal",
                "lan",
                "home.arpa",
                "intranet",
            ];
            if !name.contains('.') || LOCAL.iter().any(|l| within(&name, l)) {
                return Err(private());
            }
            let port = url.port_or_known_default().unwrap_or(443);
            let addrs: Vec<_> = (name.as_str(), port)
                .to_socket_addrs()
                .map_err(|_| failed(Status::Failed, "host not found"))?
                .collect();
            if addrs.is_empty() {
                return Err(failed(Status::Failed, "host not found"));
            }
            if addrs.iter().any(|a| !global(a.ip())) {
                return Err(private());
            }
            Ok(())
        }
        None => Err(failed(Status::Failed, "not an address")),
    }
}

/// An address on the public internet: not loopback, private, link-local,
/// shared (CGNAT), documentation, broadcast or unspecified.
fn global(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => global_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return global_v4(v4);
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || v6 == Ipv6Addr::LOCALHOST)
        }
    }
}

fn global_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        || a >= 240)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_machine_and_its_network_are_never_fetched() {
        for u in [
            "http://127.0.0.1:8080/completion",
            "http://192.168.1.1/",
            "http://10.0.0.5/admin",
            "http://169.254.169.254/latest",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://100.64.0.1/",
            "http://printer.local/",
            "http://router/",
        ] {
            let e = public(&Url::parse(u).unwrap()).unwrap_err();
            assert_eq!(e.status, Status::Skip, "{u}");
        }
        assert!(public(&Url::parse("http://93.184.215.14/").unwrap()).is_ok());
    }

    #[test]
    fn google_wrappers_give_the_page_they_wrap() {
        assert_eq!(
            unwrap_google("https://www.google.com/url?q=https://n1info.rs/a&sa=D"),
            "https://n1info.rs/a"
        );
        assert_eq!(
            unwrap_google("https://www.google.com/amp/s/n1info.rs/a/amp"),
            "https://n1info.rs/a/amp"
        );
        assert_eq!(
            unwrap_google("https://n1info.rs/url?q=x"),
            "https://n1info.rs/url?q=x"
        );
    }

    #[test]
    fn a_files_name_comes_from_its_disposition() {
        assert_eq!(
            filename("attachment; filename*=UTF-8''Zapisnik%2001.12.docx").as_deref(),
            Some("Zapisnik 01.12.docx")
        );
        assert_eq!(
            filename("inline; filename=\"Statut.pdf\"").as_deref(),
            Some("Statut.pdf")
        );
        assert_eq!(filename("inline"), None);
    }

    /// Goes out to the internet, so it is not in the suite:
    /// `cargo test -p tga-links -- --ignored`. Proves the TLS stack, the
    /// redirect walk and the reader on one public page that names nobody.
    #[test]
    #[ignore]
    fn a_real_page_is_fetched_and_read() {
        let f = Fetcher::new(Some("en"), Box::new(|_, _| String::new()));
        let card = f.fetch(&crate::find::link("http://example.com/").unwrap());
        assert_eq!(card.status, Status::Read, "{}", card.why);
        assert_eq!(card.title, "Example Domain");
        assert!(card.text.contains("document"), "{}", card.text);
    }

    #[test]
    fn a_link_with_nothing_to_read_is_skipped_without_a_request() {
        let f = Fetcher::new(Some("sr"), Box::new(|_, _| String::new()));
        let card = f.fetch(&crate::find::link("https://t.me/+invite").unwrap());
        assert_eq!(card.status, Status::Skip);
        assert_eq!(card.link, "https://t.me/+invite");
    }
}
