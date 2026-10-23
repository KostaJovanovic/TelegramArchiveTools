//! A fetched page's text and metadata: ReVision's `html_card`.
//!
//! ReVision reads an article through trafilatura and its preview through
//! the page's Open Graph tags; here both are `dom_smoothie`, a port of
//! Mozilla's Readability, whose metadata reads the same tags (Open Graph,
//! Twitter cards, JSON-LD, `<title>`).

use crate::fetch::Failed;
use crate::{Card, Status};

/// The page's text, in its own encoding: the `Content-Type`'s charset, else
/// a `<meta charset>` near the top, else UTF-8. A Serbian site in
/// windows-1250 is not rare.
pub(crate) fn decode(body: &[u8], charset: Option<&str>) -> String {
    let named = charset
        .map(str::to_string)
        .or_else(|| meta_charset(body))
        .and_then(|c| encoding_rs::Encoding::for_label(c.trim().as_bytes()));
    let encoding = named.unwrap_or(encoding_rs::UTF_8);
    let (text, _, _) = encoding.decode(body);
    text.into_owned()
}

/// `charset=…` inside a `<meta>` in the first 4 KB.
fn meta_charset(body: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(&body[..body.len().min(4096)]).to_ascii_lowercase();
    let mut rest = head.as_str();
    while let Some(i) = rest.find("<meta") {
        rest = &rest[i + 5..];
        let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
        if let Some((_, after)) = tag.split_once("charset=") {
            let v: String = after
                .trim_start_matches(['"', '\''])
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// What a page says: its metadata, and its main text unless `preview_only`.
/// A page with neither a title nor any text failed.
pub(crate) fn card(url: &str, html: &str, preview_only: bool) -> Result<Card, Failed> {
    let config = dom_smoothie::Config {
        text_mode: dom_smoothie::TextMode::Formatted,
        ..Default::default()
    };
    let mut r =
        dom_smoothie::Readability::new(html, Some(url), Some(config)).map_err(|_| Failed {
            status: Status::Failed,
            why: "not a page".into(),
        })?;
    let meta = r.get_article_metadata(r.parse_json_ld());
    let mut card = Card {
        url: url.to_string(),
        title: meta.title.trim().to_string(),
        site: meta.site_name.unwrap_or_default().trim().to_string(),
        author: meta.byline.unwrap_or_default().trim().to_string(),
        date: day(meta.published_time.as_deref()),
        description: meta.excerpt.unwrap_or_default().trim().to_string(),
        ..Card::default()
    };
    if !preview_only {
        if let Ok(a) = r.parse() {
            let set = |field: &mut String, v: &str| {
                if !v.trim().is_empty() {
                    *field = v.trim().to_string();
                }
            };
            set(&mut card.title, &a.title);
            set(&mut card.site, a.site_name.as_deref().unwrap_or_default());
            set(&mut card.author, a.byline.as_deref().unwrap_or_default());
            set(&mut card.date, &day(a.published_time.as_deref()));
            card.text = a.text_content.trim().to_string();
        }
    }
    if card.text.is_empty() {
        card.text = card.description.clone();
    }
    if card.title.is_empty() && card.text.is_empty() {
        return Err(Failed {
            status: Status::Failed,
            why: "nothing on the page".into(),
        });
    }
    Ok(card)
}

/// `2024-12-02T10:00:00Z` -> `2024-12-02`; anything not starting with a day
/// is dropped.
fn day(t: Option<&str>) -> String {
    let t = t.unwrap_or_default().trim();
    let d: String = t.chars().take(10).collect();
    let shaped = d.len() == 10
        && d.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    if shaped {
        d
    } else {
        String::new()
    }
}

/// An embed's HTML as text: a `<br>` a line, the tags off, the entities read.
pub(crate) fn tags_off(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let end = rest[i..].find('>').map_or(rest.len(), |n| i + n + 1);
        let tag = rest[i..end].to_ascii_lowercase();
        if tag.starts_with("<br") || tag.starts_with("</p") {
            out.push('\n');
        } else {
            out.push(' ');
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    let text = entities(&out);
    text.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The common named entities and every numeric one.
fn entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16)
                .ok()
                .and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARTICLE: &str = r#"<!doctype html><html><head>
<meta charset="utf-8"><title>Naslov | Portal</title>
<meta property="og:title" content="Studenti blokirali fakultet">
<meta property="og:site_name" content="Portal">
<meta property="og:description" content="Kratak opis.">
<meta property="article:published_time" content="2024-12-02T10:00:00+01:00">
</head><body><nav>Početna Vesti Sport</nav><article><h1>Studenti blokirali fakultet</h1>
<p>Studenti Filozofskog fakulteta su u ponedeljak ujutru blokirali zgradu i zatražili odgovore od uprave, navodeći da je rok istekao.</p>
<p>Plenum je doneo odluku da blokada traje do daljnjeg, a sledeći sastanak zakazan je za sredu u podne u velikom amfiteatru.</p>
<p>Uprava se do sada nije oglasila, a profesori su najavili podršku studentima na sednici veća koja će biti održana u četvrtak.</p>
</article><footer>© Portal</footer></body></html>"#;

    #[test]
    fn an_article_gives_its_title_site_day_and_text() {
        let c = card("https://portal.rs/vesti/1", ARTICLE, false).unwrap();
        assert_eq!(c.title, "Studenti blokirali fakultet");
        assert_eq!(c.site, "Portal");
        assert_eq!(c.date, "2024-12-02");
        assert!(c.text.contains("Plenum je doneo odluku"), "{}", c.text);
        assert!(!c.text.contains("Početna Vesti Sport"), "{}", c.text);
    }

    #[test]
    fn a_preview_is_the_description_only() {
        let c = card("https://portal.rs/vesti/1", ARTICLE, true).unwrap();
        assert_eq!(c.text, "Kratak opis.");
    }

    #[test]
    fn an_empty_page_failed() {
        let e = card("https://portal.rs/", "<html><body></body></html>", false).unwrap_err();
        assert_eq!(e.status, Status::Failed);
    }

    #[test]
    fn a_page_in_windows_1250_reads_right() {
        let (bytes, _, _) = encoding_rs::WINDOWS_1250
            .encode("<meta charset=windows-1250><p>Čačak, Šabac, Žabalj</p>");
        assert!(decode(&bytes, None).contains("Čačak, Šabac, Žabalj"));
        assert!(decode(&bytes, Some("windows-1250")).contains("Žabalj"));
    }

    #[test]
    fn an_embed_loses_its_tags_and_keeps_its_lines() {
        assert_eq!(
            tags_off(
                "<blockquote><p>Prvi red<br>drugi &amp; tre&#263;i</p>&mdash; Ana</blockquote>"
            ),
            "Prvi red\ndrugi & treći\n— Ana"
        );
    }
}
