//! When a document is actually from.
//!
//! Three places a date can come from, and none of them is reliable alone:
//!
//! - **The filesystem.** Useless. Every file in an export carries the mtime of
//!   the moment the exporter wrote it; all 44,000 in the KRGM corpus say
//!   `2026-08-27`. Not consulted at all.
//! - **The message it was posted in.** Right whenever somebody shares a
//!   document the day they wrote it, and wrong by months whenever they empty a
//!   folder into the chat. Five KRGM minutes from 2024-12-30 through
//!   2025-02-04 were all posted on 2025-09-24, between 232 and 268 days late.
//! - **The filename and the text.** Where the real date usually is -- and where
//!   it is written eight different ways, half of them without a year.
//!
//! So: read the name and the text, rank what turns up, and *say which rung it
//! came from*, because a reader who cannot tell an inferred year from a written
//! one has no way to discount it.
//!
//! **Day comes first.** `06.07.2026` is the sixth of July. Both corpora are
//! Serbian and so is every document in them; there is no mixed-locale case to
//! get wrong, and reading it American would silently move a third of the
//! zapisnici to a different month.

use std::sync::OnceLock;

use chrono::{Datelike, NaiveDate};
use regex::{Captures, Regex};

/// Which rung of the ladder the date came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// A complete date in the filename. `KRGM Zapisnik 06.07.2026.docx`.
    Filename,
    /// A date written inside the document.
    Content,
    /// A day and month in the filename; the year is the post date's.
    /// `15.6.txt`, posted 2026-06-15.
    FilenameYearGuessed,
    /// Nothing found anywhere, so this is only when it was posted.
    #[default]
    Posted,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Filename => "filename",
            Source::Content => "content",
            Source::FilenameYearGuessed => "filename+posted",
            Source::Posted => "posted",
        }
    }

    /// Whether this rung involved a guess the reader should discount.
    pub fn is_guess(self) -> bool {
        matches!(self, Source::FilenameYearGuessed | Source::Posted)
    }
}

/// What a document says about its own date.
#[derive(Debug, Clone, Default)]
pub struct Dates {
    /// The best single answer. Always set once [`infer`] has run: the post
    /// date is the floor.
    pub best: Option<NaiveDate>,
    pub src: Source,
    /// Earliest and latest of every date found in the *text*, and how many.
    ///
    /// Kept beside `best` rather than folded into it because they answer a
    /// different question: `best` is when the document is from, the range is
    /// what period it talks about. A media-monitoring report written on one
    /// day covers the fortnight before it.
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub n: usize,
}

/// Work out a document's date from its name, its text and when it was posted.
pub fn infer(name: &str, text: &str, posted: NaiveDate) -> Dates {
    let stem = strip_extension(name);
    let body = without_urls(text);

    // Reading order, not chronological. A document leads with its own date --
    // "Zapisnik sa sastanka odrzanog 6.7.2026." is the first line -- whereas
    // the *earliest* date in a media report is whichever old article it
    // happened to cite.
    let written = full_dates(&body, posted);
    let first_written = written.first().copied();

    let mut span = written;
    span.sort();
    span.dedup();

    let (best, src) = if let Some(d) = full_dates(&stem, posted).first().copied() {
        (d, Source::Filename)
    } else if let Some(d) = first_written {
        (d, Source::Content)
    } else if let Some(d) = partial_date(&stem, posted) {
        (d, Source::FilenameYearGuessed)
    } else {
        (posted, Source::Posted)
    };

    Dates {
        best: Some(best),
        src,
        from: span.first().copied(),
        to: span.last().copied(),
        n: span.len(),
    }
}

// ---------------------------------------------------------------------------
// the patterns

/// Serbian month names, Latin and Cyrillic.
///
/// Matched as a prefix with a trailing `\w*`, so `jun` also catches `juna`,
/// `juni` and `junu` -- the case ending varies with the sentence and a document
/// title uses whichever one reads right. The longer stem of each pair comes
/// first (`septembar` before `septembr`) because the lookup takes the first
/// entry the capture starts with, and the shorter one would otherwise always
/// win and leave a stray letter.
const MONTHS: &[(&str, u32)] = &[
    ("januar", 1),
    ("februar", 2),
    ("mart", 3),
    ("april", 4),
    ("maj", 5),
    ("jun", 6),
    ("jul", 7),
    ("avgust", 8),
    ("septembar", 9),
    ("septembr", 9),
    ("oktobar", 10),
    ("oktobr", 10),
    ("novembar", 11),
    ("novembr", 11),
    ("decembar", 12),
    ("decembr", 12),
    ("јануар", 1),
    ("фебруар", 2),
    ("март", 3),
    ("април", 4),
    ("мај", 5),
    ("јун", 6),
    ("јул", 7),
    ("август", 8),
    ("септембар", 9),
    ("септембр", 9),
    ("октобар", 10),
    ("октобр", 10),
    ("новембар", 11),
    ("новембр", 11),
    ("децембар", 12),
    ("децембр", 12),
];

struct Patterns {
    iso: Regex,
    compact: Regex,
    day_first: Regex,
    month_name: Regex,
    ddmm_range: Regex,
    day_month: Regex,
    url: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let names: Vec<&str> = MONTHS.iter().map(|(n, _)| *n).collect();
        let alternation = names.join("|");
        Patterns {
            // `2025-11-28`, `2025.11.28`.
            iso: Regex::new(r"(?:^|[^0-9])(\d{4})[-.](\d{1,2})[-.](\d{1,2})(?:[^0-9]|$)")
                .expect("iso"),
            // `DOC-20260528-WA0003`, `Screenshot_20250126-012619`. The digit
            // guards on both sides are what stop it eating the front of
            // `lv_0_20240918192332`, where the six trailing digits say the
            // number is a timestamp and not a date.
            compact: Regex::new(r"(?:^|[^0-9])(20\d{2})(\d{2})(\d{2})(?:[^0-9]|$)")
                .expect("compact"),
            // `06.07.2026`, `08-06-2026`, `12_3_2025`, and `1. 11. 2025` --
            // which is how it is written in running Serbian prose, spaces and
            // all. Two corpus documents are titled "Predlog povodom protesta
            // 1. 11. 2025" and were dated by their post date until the spaces
            // were allowed for.
            //
            // `[ \t]` and not `\s`, which would match a newline. A zapisnik is
            // a numbered list, and across three lines "1." "2." "2025" is an
            // enumeration, not the first of February.
            day_first: Regex::new(
                r"(?:^|[^0-9])(\d{1,2})[ \t]{0,2}[._\-][ \t]{0,2}(\d{1,2})[ \t]{0,2}[._\-][ \t]{0,2}(\d{4})(?:[^0-9]|$)",
            )
            .expect("day_first"),
            // `13. april 2026`, `2-9. jul 2026`, `4. фебруара 2025`.
            month_name: Regex::new(&format!(
                r"(?i)(\d{{1,2}})\s*\.?\s*({alternation})\w*\s*(\d{{4}})"
            ))
            .expect("month_name"),
            // `Izvestaj_RJMM_0405_1005` -- 4 May to 10 May, no year anywhere.
            //
            // Tried before anything reads `2004_0305` as the year 2004: that
            // filename is 20 April to 3 May, and taken as a year it dates the
            // document 8,108 days before it was posted. It is only reached
            // from `partial_date`, after every full-date pattern has failed,
            // which is what keeps a real `2004-03-05` safe.
            ddmm_range: Regex::new(r"(?:^|[^0-9])(\d{2})(\d{2})[_\-](\d{2})(\d{2})(?:[^0-9]|$)")
                .expect("ddmm_range"),
            // `15.6`, and nothing else in the name.
            day_month: Regex::new(r"(?:^|[^0-9])(\d{1,2})[.](\d{1,2})(?:[^0-9]|$)")
                .expect("day_month"),
            url: Regex::new(r"https?://\S+").expect("url"),
        }
    })
}

/// Blank out URLs before looking for dates.
///
/// `https://promevent.rs/matursko-vece-ff-ucenici-2026/` is a slug, not a
/// date, and one scraped link dump in the corpus is 180 lines of them. Replaced
/// with a space rather than removed, so nothing either side of a link runs
/// together into a number that was never there.
fn without_urls(text: &str) -> String {
    patterns().url.replace_all(text, " ").into_owned()
}

/// `Zapisnik-KRGM-16-07-2026 (2).docx` -> `Zapisnik-KRGM-16-07-2026 (2)`.
///
/// Only the last extension: `krgm_12_3_2025.docx.pdf` keeps its `.docx`, which
/// costs nothing, and stripping every one of them would take the `.2025` off a
/// name that ends in a date.
fn strip_extension(name: &str) -> String {
    match name.rsplit_once('.') {
        // A tail with a space in it is not an extension. The corpus has files
        // whose names end `... 13. januar 2026`, with none at all.
        Some((stem, ext)) if ext.len() <= 5 && !ext.contains(' ') => stem.to_string(),
        _ => name.to_string(),
    }
}

// ---------------------------------------------------------------------------
// the ladder

/// Is this a date a document in this export could plausibly carry?
///
/// Telegram did not exist in 1997, and a document cannot be written long after
/// it was posted -- a year of slack, for a clock that is wrong or a filename
/// that names next year's plan.
fn plausible(date: NaiveDate, posted: NaiveDate) -> bool {
    date.year() >= 2000 && date <= posted + chrono::Duration::days(366)
}

fn ymd(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(y, m, d)
}

fn num(c: &Captures, i: usize) -> u32 {
    c.get(i).and_then(|m| m.as_str().parse().ok()).unwrap_or(0)
}

/// Every complete date -- year included -- in `text`, in the order written.
fn full_dates(text: &str, posted: NaiveDate) -> Vec<NaiveDate> {
    let p = patterns();
    let mut hits: Vec<(usize, NaiveDate)> = Vec::new();
    let mut push = |at: usize, date: Option<NaiveDate>| {
        if let Some(d) = date.filter(|d| plausible(*d, posted)) {
            hits.push((at, d));
        }
    };

    for c in p.iso.captures_iter(text) {
        let at = c.get(1).map(|m| m.start()).unwrap_or(0);
        push(at, ymd(num(&c, 1) as i32, num(&c, 2), num(&c, 3)));
    }
    for c in p.compact.captures_iter(text) {
        let at = c.get(1).map(|m| m.start()).unwrap_or(0);
        push(at, ymd(num(&c, 1) as i32, num(&c, 2), num(&c, 3)));
    }
    for c in p.day_first.captures_iter(text) {
        let at = c.get(1).map(|m| m.start()).unwrap_or(0);
        // Day first, always -- see the note at the top of the file. An
        // impossible pair such as `32.13.2026` is a typo and falls out of
        // `from_ymd_opt` as `None` rather than being reinterpreted.
        push(at, ymd(num(&c, 3) as i32, num(&c, 2), num(&c, 1)));
    }
    for c in p.month_name.captures_iter(text) {
        let Some(day) = c.get(1) else { continue };
        let name = c
            .get(2)
            .map(|m| m.as_str().to_lowercase())
            .unwrap_or_default();
        let Some((_, month)) = MONTHS.iter().find(|(n, _)| name.starts_with(n)) else {
            continue;
        };
        push(
            day.start(),
            ymd(num(&c, 3) as i32, *month, day.as_str().parse().unwrap_or(0)),
        );
    }

    hits.sort_by_key(|(at, _)| *at);
    hits.into_iter().map(|(_, d)| d).collect()
}

/// A day and month with no year, from a filename.
///
/// The year is the post date's, rolled back one only when the date is further
/// ahead than [`AHEAD`] -- minutes named `28.12` posted on the 3rd of January
/// are last year's, but a notice for the 1st of October posted in late
/// September is not.
fn partial_date(text: &str, posted: NaiveDate) -> Option<NaiveDate> {
    let p = patterns();

    // The range form first, for the reason in its pattern's comment.
    if let Some(c) = p.ddmm_range.captures(text) {
        // The start of the range: a report covering 4-10 May is about the week
        // beginning on the 4th, and filing it under the 10th would put it in
        // the wrong week whenever the range straddles one.
        if let Some(d) = with_year(num(&c, 1), num(&c, 2), posted) {
            return Some(d);
        }
    }
    let c = p.day_month.captures(text)?;
    with_year(num(&c, 1), num(&c, 2), posted)
}

/// How far ahead of the post date a year-less date may still be this year.
///
/// **Not "any day after it is last year's".** That was the first rule and it
/// was wrong by a year on every announcement: `ФМК X ФПН - Шетња 01.10.pdf`
/// was posted on 2025-09-28 to advertise a walk three days later, and rolling
/// it back dated a flyer for October 2025 to October 2024.
///
/// Half a year is the split that separates the two cases with room to spare.
/// A notice goes out days or weeks ahead; `28.12` posted on the 3rd of January
/// is 359 days ahead and obviously December just gone.
const AHEAD: i64 = 183;

fn with_year(day: u32, month: u32, posted: NaiveDate) -> Option<NaiveDate> {
    match ymd(posted.year(), month, day) {
        Some(d) if d <= posted + chrono::Duration::days(AHEAD) => Some(d),
        Some(_) => ymd(posted.year() - 1, month, day),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("a real date")
    }

    /// Every one of these is a real filename from the KRGM corpus.
    #[test]
    fn full_dates_in_filenames() {
        let posted = on(2026, 8, 27);
        let cases = [
            ("KRGM Zapisnik 06.07.2026.docx", on(2026, 7, 6)),
            ("krgm-zapisnik-08-06-2026.docx", on(2026, 6, 8)),
            ("Zapisnik-KRGM-16-07-2026 (2).docx", on(2026, 7, 16)),
            ("Zapisnik_KRGZM_12.1.2025..docx", on(2025, 1, 12)),
            ("ZAPISNIK 30.12.2024..docx", on(2024, 12, 30)),
            ("HHBB krgm 23.10.2024..docx", on(2024, 10, 23)),
            ("krgm_12_3_2025.docx.pdf", on(2025, 3, 12)),
            ("Rezultati glasanja - 2025-11-28.txt", on(2025, 11, 28)),
            ("DOC-20260528-WA0003..pdf", on(2026, 5, 28)),
            ("Screenshot_20250126-012619.png", on(2025, 1, 26)),
            ("Zapisnik KRGM 13. april 2026.docx", on(2026, 4, 13)),
            ("Ревидиран записник КРГМ 04.02.2025..docx", on(2025, 2, 4)),
        ];
        for (name, want) in cases {
            let got = infer(name, "", posted);
            assert_eq!(got.best, Some(want), "{name}");
            assert_eq!(got.src, Source::Filename, "{name}");
            assert!(!got.src.is_guess(), "{name}");
        }
    }

    #[test]
    fn a_ddmm_range_is_not_a_year() {
        // `Izveštaj_RJMM_2004_0305.pdf` was posted 2026-05-16. Read as a year,
        // `2004` dates it 8,108 days earlier. It is 20 April to 3 May.
        let got = infer("Izveštaj_RJMM_2004_0305.pdf", "", on(2026, 5, 16));
        assert_eq!(got.best, Some(on(2026, 4, 20)));
        assert_eq!(got.src, Source::FilenameYearGuessed);
    }

    #[test]
    fn the_other_ddmm_ranges_land_on_the_day_the_week_opens() {
        let posted = on(2026, 5, 16);
        // 04.05 - 10.05, and 13.04 - 19.04.
        assert_eq!(
            infer("Izveštaj_RJMM_0405_1005.docx", "", posted).best,
            Some(on(2026, 5, 4)),
        );
        assert_eq!(
            infer("rjmm_izvestaj_1304_1904 (1).pdf", "", posted).best,
            Some(on(2026, 4, 13)),
        );
    }

    #[test]
    fn a_day_and_month_takes_its_year_from_the_post_date() {
        // `15.6.txt`, posted 2026-06-15. Its contents are "pocetak / fpn: ...
        // / kraj" -- there is no date anywhere in the file itself.
        let got = infer(
            "15.6.txt",
            "pocetak\nfpn: procitajte predlog i prenesite plenumima\nkraj",
            on(2026, 6, 15),
        );
        assert_eq!(got.best, Some(on(2026, 6, 15)));
        assert_eq!(got.src, Source::FilenameYearGuessed);
        assert!(got.src.is_guess());
    }

    #[test]
    fn a_day_and_month_long_after_the_post_date_belongs_to_last_year() {
        let got = infer("28.12.txt", "", on(2026, 1, 3));
        assert_eq!(got.best, Some(on(2025, 12, 28)));
    }

    #[test]
    fn a_notice_for_next_week_is_not_dated_a_year_ago() {
        // `ФМК X ФПН - Шетња 01.10.pdf`, posted 2025-09-28 to announce a walk
        // on the 1st of October. Under the first rule -- anything after the
        // post date is last year's -- this came out as October 2024, and the
        // text inside says "11 months since Novi Sad", which pins it to 2025.
        let got = infer("ФМК X ФПН - Шетња 01.10.pdf", "", on(2025, 9, 28));
        assert_eq!(got.best, Some(on(2025, 10, 1)));
        assert_eq!(got.src, Source::FilenameYearGuessed);
    }

    #[test]
    fn a_date_in_the_text_beats_a_filename_with_none() {
        let text = "Укупан број високошколских установа: 33\n\
                    PRIJAVA PRISUSTVA NA DAN I VREME: 2025-11-28 19:52:14.432957";
        let got = infer("rezultati.txt", text, on(2026, 3, 1));
        assert_eq!(got.best, Some(on(2025, 11, 28)));
        assert_eq!(got.src, Source::Content);
    }

    #[test]
    fn the_first_date_written_wins_not_the_earliest() {
        // A zapisnik leads with its own date and then refers back. Sorting
        // chronologically would date it by the oldest thing it mentions.
        let text = "Zapisnik sa sastanka 06.07.2026.\n\
                    Nastavak rasprave od 21.05.2026.";
        let got = infer("zapisnik.docx", text, on(2026, 7, 6));
        assert_eq!(got.best, Some(on(2026, 7, 6)));
        assert_eq!(got.src, Source::Content);
        assert_eq!(
            got.from,
            Some(on(2026, 5, 21)),
            "the range still spans both"
        );
        assert_eq!(got.to, Some(on(2026, 7, 6)));
        assert_eq!(got.n, 2);
    }

    #[test]
    fn years_in_url_slugs_are_not_dates() {
        // One corpus file is 180 lines of these.
        let text = "https://promevent.rs/matursko-vece-ff-ucenici-2026/\n\
                    https://promevent.rs/mala-matura-os-starina-novak-ucenici-2026/";
        let got = infer("urls_promevent_rs_simplescraper.txt", text, on(2026, 4, 2));
        assert_eq!(got.n, 0);
        assert_eq!(got.src, Source::Posted);
        assert_eq!(got.best, Some(on(2026, 4, 2)));
    }

    #[test]
    fn nothing_anywhere_falls_back_to_the_post_date_and_says_so() {
        let got = infer(
            "avione slomiću ti krila.txt",
            "kupovina svih sedišta u avionu a320\ncena po sedištu: 3400rsd",
            on(2026, 6, 20),
        );
        assert_eq!(got.best, Some(on(2026, 6, 20)));
        assert_eq!(got.src, Source::Posted);
        assert!(got.src.is_guess());
    }

    #[test]
    fn a_time_of_day_is_not_a_date() {
        let got = infer("x.txt", "sastanak u 19:52:14 danas", on(2026, 6, 20));
        assert_eq!(got.n, 0);
    }

    #[test]
    fn an_impossible_date_is_dropped_rather_than_reinterpreted() {
        // 32.13.2026 is a typo. Read month-first it would become 13 December
        // 2026, which is worse than admitting there is no date here.
        let got = infer("x.txt", "32.13.2026", on(2026, 6, 20));
        assert_eq!(got.n, 0);
    }

    #[test]
    fn a_year_far_after_the_post_date_is_not_believed() {
        // The year is discarded; the day and month are still usable, so this
        // lands on the guessed rung rather than on 2099.
        let got = infer("plan-01.01.2099.docx", "", on(2026, 6, 20));
        assert_ne!(got.best, Some(on(2099, 1, 1)));
        assert!(got.src.is_guess());
    }

    #[test]
    fn a_date_written_with_spaces_around_the_dots_is_still_a_date() {
        // `Предлог поводом протеста 1. 11. 2025` -- a real corpus document,
        // dated by its post date until this was allowed for.
        let got = infer(
            "4_5990189809694415438.pdf",
            "Предлог поводом протеста 1. 11. 2025. Уколико студенти",
            on(2025, 10, 30),
        );
        assert_eq!(got.best, Some(on(2025, 11, 1)));
        assert_eq!(got.src, Source::Content);
    }

    #[test]
    fn an_enumeration_across_lines_is_not_a_date() {
        // The reason the spaces above are `[ \t]` and not `\s`. Minutes are
        // numbered lists, and this is points 1 and 2 followed by a year.
        let got = infer("zapisnik.docx", "1.\n2.\n2025 godina", on(2026, 1, 1));
        assert_eq!(got.n, 0);
    }

    #[test]
    fn cyrillic_month_names_read() {
        let got = infer(
            "zapisnik.docx",
            "Састанак одржан 4. фебруара 2025. године",
            on(2026, 1, 1),
        );
        assert_eq!(got.best, Some(on(2025, 2, 4)));
        assert_eq!(got.src, Source::Content);
    }

    #[test]
    fn a_day_range_with_a_month_name_takes_the_day_it_was_written_up() {
        // `Izveštaj o medijskom praćenju 2-9. jul 2026.pdf` covers a week. The
        // month-name pattern needs the day adjacent to the name, so it reads
        // the 9th -- which is the day the report closes, and the right one.
        let got = infer(
            "Izveštaj o medijskom praćenju 2-9. jul 2026.pdf",
            "",
            on(2026, 7, 10),
        );
        assert_eq!(got.best, Some(on(2026, 7, 9)));
    }

    #[test]
    fn a_timestamp_filename_is_not_read_as_a_date() {
        // `lv_0_20240918192332.gif.mp4`: the digits after the eighth say this
        // is a timestamp, and a partial read of it would give 18 September.
        let got = infer("lv_0_20240918192332.gif", "", on(2026, 5, 18));
        assert_eq!(got.src, Source::Posted);
    }
}
