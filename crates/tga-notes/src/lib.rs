//! The layer a reader (or a model) adds on top of the numbers.
//!
//! Everything else in this workspace is derived: run it twice on the same
//! export and you get the same report. An **event** is the opposite — a
//! judgement about what a stretch of messages *meant*, which no amount of
//! counting produces. So events are not computed. They live in a file beside
//! the export, they are optional, and the report renders whatever is there.
//!
//! **Two layouts load, and that is deliberate rather than lax.** The Python
//! analyser's own shape (`date`, `summary`, `messages`, `confidence`) has never
//! been filled in for any export. `_timeline`'s shape (`d`, `t`, `text`, `w`,
//! `who`, `tags`, plus a `coverage` block) has 42 written notes in it covering
//! one month of the KRGM archive, and it is the only one with real content —
//! which is why PLAN.md picks it. Refusing the first would break the parity
//! harness; refusing the second would mean the one existing body of notes could
//! not be read. Both are accepted, field by field, and every field of both is
//! optional except a date and a title.
//!
//! **`coverage` is the field the Python layout has no way to express**, and it
//! is the one worth having: a timeline with markers over one month and nothing
//! over the next nine looks identical whether the rest was quiet or simply
//! unread. Saying which is which is the difference between an annotation and a
//! claim about the whole archive.
//!
//! **Nothing in a notes file is trusted.** It is written by something outside
//! this program, so every string is escaped on the way into the HTML like any
//! other attacker-controlled value, and a malformed entry is dropped rather
//! than failing the run — an unreadable annotation must not cost you the
//! report. That is why [`load`] returns no `Result`: there is no error it could
//! hand back that the caller should act on.

use std::path::{Path, PathBuf};

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value;

pub mod digest;

pub use digest::{write_brief, write_digest, Attachment, Row, BRIEF};

/// Where the analyser looks, in order. The first that exists wins.
pub const CANDIDATES: &[&str] = &["events.json", "analysis/events.json"];

pub const SCHEMA_VERSION: i64 = 1;

/// Kinds the report has names for. Anything else is kept and shown under its
/// own name — the vocabulary belongs to whoever writes the file, not to this
/// module. Both projects' sets are here, because a reader may use either.
pub const KNOWN_KINDS: &[&str] = &[
    // `_timeline`'s, which is the vocabulary with 42 notes written in it.
    "pokret",
    "organizacija",
    "drama",
    "produkcija",
    // The Python analyser's.
    "milestone",
    "decision",
    "conflict",
    "action",
    "arrival",
    "topic",
];

/// How much an event is claimed to matter. `_timeline`'s `w`.
pub const WEIGHTS: &[&str] = &["major", "medium", "minor"];

/// The most message ids one event may cite. Past this the citation line stops
/// being a citation and becomes a data dump.
const MAX_CITED: usize = 40;

/// The most names one event may credit. Same reasoning as [`MAX_CITED`]; the
/// existing notes top out at four.
const MAX_WHO: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub start: NaiveDate,
    pub end: Option<NaiveDate>,
    /// `HH:MM`, when the file gives one — `_timeline`'s `t`. Empty otherwise.
    ///
    /// Kept as the *string the file wrote* rather than parsed into a time: it
    /// is only ever printed, several of the existing notes give a time that is
    /// plainly approximate ("01:00"), and parsing it would invite sorting by a
    /// precision the writer never claimed.
    pub time: String,
    pub title: String,
    pub summary: String,
    pub kind: String,
    pub topic: Option<i64>,
    pub messages: Vec<i64>,
    pub confidence: String,
    /// `major` | `medium` | `minor`, or empty. `_timeline`'s `w`.
    pub weight: String,
    /// Names, spelled as they appear in the export. Checked against the
    /// statistics by the report, which says so when one matches nobody.
    pub who: Vec<String>,
    pub tags: Vec<String>,
}

impl Event {
    /// Whether this draws as a bar rather than as a single marker.
    pub fn spans(&self) -> bool {
        matches!(self.end, Some(end) if end > self.start)
    }

    /// The kind, if the report has a name for it.
    pub fn known_kind(&self) -> bool {
        KNOWN_KINDS.contains(&self.kind.as_str())
    }
}

/// What the notes claim to have read, and how closely.
///
/// Without this a timeline with markers over one month and nothing over the
/// next nine looks identical whether the rest was quiet or simply unread.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Coverage {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    /// How closely it was read — free text. `_timeline`'s existing file says
    /// `prekretnice`, i.e. turning points only.
    pub level: String,
    pub note: String,
}

impl Coverage {
    /// Whether it says anything at all. An empty block is the same as none.
    pub fn is_empty(&self) -> bool {
        self.from.is_none() && self.to.is_none() && self.level.is_empty() && self.note.is_empty()
    }
}

/// Everything a notes file carries.
#[derive(Debug, Clone, Default)]
pub struct Notes {
    pub events: Vec<Event>,
    pub coverage: Option<Coverage>,
    /// Where they came from, who wrote them, and what was dropped. Empty when
    /// there is no file, which is the ordinary case.
    pub source: String,
}

impl Notes {
    pub fn is_empty(&self) -> bool {
        self.events.is_empty() && self.coverage.is_none()
    }

    /// Every distinct `kind` present, in first-seen order.
    ///
    /// The report builds its filter row from this rather than from
    /// [`KNOWN_KINDS`], so a file using a vocabulary nobody anticipated still
    /// gets filters — and a file using three of the four does not get a dead
    /// control for the fourth.
    pub fn kinds(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for event in &self.events {
            if !event.kind.is_empty() && !out.contains(&event.kind) {
                out.push(event.kind.clone());
            }
        }
        out
    }
}

/// A date out of untrusted JSON, as a plain ISO date or as a datetime.
///
/// Anything else — a number, a null, a month name — is not a date, and saying
/// so by returning `None` is what makes the entry get dropped instead of
/// failing the run.
fn as_date(value: Option<&Value>) -> Option<NaiveDate> {
    let text = match value {
        Some(Value::String(s)) => s.trim().to_string(),
        // `if not value: return None` in the Python, which is false for 0 and
        // for the empty string as well as for a missing key.
        Some(Value::Number(n)) => n.to_string(),
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    NaiveDate::parse_from_str(&text, "%Y-%m-%d")
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S")
                .or_else(|_| NaiveDateTime::parse_from_str(&text, "%Y-%m-%d %H:%M:%S"))
                .or_else(|_| NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M"))
                .or_else(|_| NaiveDateTime::parse_from_str(&text, "%Y-%m-%d %H:%M"))
                .map(|dt| dt.date())
                .ok()
        })
}

/// The first of several keys that holds a non-empty trimmed string.
///
/// This is how the two layouts are accepted without a mode flag: `summary` and
/// `text` mean the same thing, so both are read and whichever is there wins. A
/// file mixing them is not an error — it is a file somebody edited by hand.
fn first_str(raw: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(text) = raw.get(*key).and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }
    String::new()
}

/// A list of strings, trimmed, empties dropped, capped.
fn string_list(raw: &Value, key: &str, cap: usize) -> Vec<String> {
    raw.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .take(cap)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// One entry, or `None` if it is not usable.
///
/// A date and a title are the whole requirement. A missing title means the
/// event cannot be named and a missing date means it cannot be placed, and an
/// event that can be neither named nor placed is not an event.
fn one(raw: &Value, index: usize) -> Option<Event> {
    if !raw.is_object() {
        return None;
    }
    // `date` is the Python's, `d` is `_timeline`'s, `start` is accepted because
    // the Python accepted it.
    let start = as_date(raw.get("date"))
        .or_else(|| as_date(raw.get("d")))
        .or_else(|| as_date(raw.get("start")))?;
    let title = first_str(raw, &["title"]);
    if title.is_empty() {
        return None;
    }

    // An `end` before the start is not a span, it is a typo. Dropping it leaves
    // a moment marker, which is still true.
    let end = as_date(raw.get("end")).filter(|end| *end >= start);

    let messages: Vec<i64> = raw
        .get("messages")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                // `isinstance(m, int)` — and in Python `True` is an int, but
                // serde_json keeps bools out of `as_i64`, which is the better
                // reading of the same intent.
                .filter_map(Value::as_i64)
                .take(MAX_CITED)
                .collect()
        })
        .unwrap_or_default();

    let kind = {
        let k = first_str(raw, &["kind"]).to_lowercase();
        if k.is_empty() {
            "milestone".to_string()
        } else {
            k
        }
    };

    let id = {
        let given = raw
            .get("id")
            .map(|v| match v {
                Value::String(s) => s.trim().to_string(),
                Value::Null => String::new(),
                other => other.to_string(),
            })
            .unwrap_or_default();
        if given.is_empty() {
            // The *input* index, taken before the sort, exactly as the Python
            // does — so an id is stable against the file rather than against
            // the order the report happens to draw in.
            format!("e{index}")
        } else {
            given
        }
    };

    Some(Event {
        id,
        start,
        end,
        time: first_str(raw, &["t", "time"]),
        title,
        summary: first_str(raw, &["summary", "text"]),
        kind,
        topic: raw.get("topic").and_then(Value::as_i64),
        messages,
        confidence: first_str(raw, &["confidence"]).to_lowercase(),
        weight: first_str(raw, &["w", "weight"]).to_lowercase(),
        who: string_list(raw, "who", MAX_WHO),
        tags: string_list(raw, "tags", MAX_WHO),
    })
}

fn coverage_of(body: &Value) -> Option<Coverage> {
    let raw = body.get("coverage")?;
    if !raw.is_object() {
        return None;
    }
    let cover = Coverage {
        from: as_date(raw.get("from")),
        to: as_date(raw.get("to")),
        level: first_str(raw, &["level"]),
        note: first_str(raw, &["note"]),
    };
    // An empty block is the same as none: it would otherwise render a panel
    // that states nothing, which is worse than not stating it.
    if cover.is_empty() {
        None
    } else {
        Some(cover)
    }
}

/// The notes file for the export at `root`, if there is one.
pub fn find(root: &Path) -> Option<PathBuf> {
    for name in CANDIDATES {
        let path = root.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Notes for the export at `root`.
///
/// Never fails. A missing file is the ordinary case — a Telegram Desktop export
/// has no such sidecar and never will — and a broken one is a problem with the
/// annotation, not with the export.
pub fn load(root: &Path) -> Notes {
    match find(root) {
        Some(path) => load_file(&path),
        None => Notes::default(),
    }
}

/// One named notes file, wherever it sits.
///
/// Split out of [`load`] so a notes file can be read without an export beside
/// it — which is what lets a recorded `stats.json` be re-rendered with
/// annotations and no archive on disk. The KRGM export is 173 MB of other
/// people's conversation; checking that its 42 notes land should not require
/// copying it.
pub fn load_file(path: &Path) -> Notes {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => {
            return Notes {
                source: format!("{name} could not be read (OSError)"),
                ..Default::default()
            }
        }
    };
    let body: Value = match serde_json::from_str(&text) {
        Ok(body) => body,
        Err(_) => {
            return Notes {
                source: format!("{name} could not be read (ValueError)"),
                ..Default::default()
            }
        }
    };

    let coverage = coverage_of(&body);

    // Either `{ "events": [...] }` or a bare array. The report says where the
    // notes came from, so both shapes are worth accepting.
    let raw = match &body {
        Value::Object(map) => map.get("events"),
        other => Some(other),
    };
    let Some(Value::Array(items)) = raw else {
        return Notes {
            coverage,
            source: format!("{name} has no events list"),
            ..Default::default()
        };
    };

    let mut events: Vec<Event> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| one(item, index))
        .collect();
    // Time before title: `_timeline`'s notes give several events per day and a
    // time for each, so ordering on the day alone would shuffle a morning's
    // events into alphabetical order and read as a sequence that never
    // happened. An empty time sorts first, which puts an undated-within-the-day
    // note at the top of it rather than at an arbitrary hour.
    events.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.time.cmp(&b.time))
            .then_with(|| a.title.cmp(&b.title))
    });

    let dropped = items.len() - events.len();
    let mut source = name;
    if let Some(written_by) = body.get("source").and_then(Value::as_str) {
        if !written_by.is_empty() {
            source.push_str(&format!(", written by {written_by}"));
        }
    }
    if dropped > 0 {
        source.push_str(&format!(
            "; {dropped} entr{} skipped as malformed",
            if dropped == 1 { "y" } else { "ies" }
        ));
    }

    Notes {
        events,
        coverage,
        source,
    }
}

/// Events grouped by the day they start on.
pub fn by_day(events: &[Event]) -> std::collections::BTreeMap<NaiveDate, Vec<&Event>> {
    let mut out: std::collections::BTreeMap<NaiveDate, Vec<&Event>> = Default::default();
    for event in events {
        out.entry(event.start).or_default().push(event);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parsed(raw: Value) -> Option<Event> {
        one(&raw, 7)
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_date_and_a_title_are_the_whole_requirement() {
        let event = parsed(json!({ "date": "2025-09-05", "title": "Something" }))
            .expect("date and title is enough");
        assert_eq!(event.start, day(2025, 9, 5));
        assert_eq!(event.kind, "milestone");
        assert_eq!(event.id, "e7", "no id given, so the index names it");
        assert!(!event.spans());
    }

    #[test]
    fn the_timeline_layout_loads_field_for_field() {
        // The shape the 42 existing notes are written in. Every key here is one
        // the Python layout does not have.
        let event = parsed(json!({
            "d": "2025-09-05",
            "t": "20:59",
            "title": "Grupa nastaje",
            "text": "kosta pravi grupu i konvertuje je u supergrupu.",
            "kind": "organizacija",
            "w": "major",
            "who": ["kosta kosta mašinski"],
            "tags": ["osnivanje", "kanali"]
        }))
        .expect("the layout PLAN.md picked must load");
        assert_eq!(event.start, day(2025, 9, 5));
        assert_eq!(event.time, "20:59");
        assert_eq!(
            event.summary,
            "kosta pravi grupu i konvertuje je u supergrupu."
        );
        assert_eq!(event.kind, "organizacija");
        assert_eq!(event.weight, "major");
        assert_eq!(event.who, vec!["kosta kosta mašinski"]);
        assert_eq!(event.tags, vec!["osnivanje", "kanali"]);
        assert!(event.known_kind());
    }

    #[test]
    fn both_layouts_name_the_body_text_and_either_spelling_wins() {
        // `summary` is the Python's word and `text` is `_timeline`'s. A file
        // mixing them is somebody editing by hand, not an error.
        let py = parsed(json!({ "date": "2025-09-05", "title": "x", "summary": "S" })).unwrap();
        let tl = parsed(json!({ "d": "2025-09-05", "title": "x", "text": "T" })).unwrap();
        assert_eq!(py.summary, "S");
        assert_eq!(tl.summary, "T");
    }

    #[test]
    fn start_is_accepted_as_well_as_date_and_d() {
        assert!(parsed(json!({ "start": "2025-09-05", "title": "x" })).is_some());
        assert!(parsed(json!({ "d": "2025-09-05", "title": "x" })).is_some());
    }

    #[test]
    fn a_datetime_is_read_as_its_date() {
        let event = parsed(json!({ "date": "2025-09-05T14:30:00", "title": "x" })).unwrap();
        assert_eq!(event.start, day(2025, 9, 5));
    }

    #[test]
    fn an_entry_with_no_usable_date_is_dropped_not_raised_on() {
        assert!(parsed(json!({ "date": "last tuesday", "title": "x" })).is_none());
        assert!(parsed(json!({ "title": "x" })).is_none());
        assert!(parsed(json!({ "date": null, "title": "x" })).is_none());
    }

    #[test]
    fn an_entry_with_no_title_is_dropped() {
        assert!(parsed(json!({ "date": "2025-09-05" })).is_none());
        assert!(parsed(json!({ "date": "2025-09-05", "title": "   " })).is_none());
    }

    #[test]
    fn a_non_object_entry_is_dropped() {
        assert!(parsed(json!("2025-09-05")).is_none());
        assert!(parsed(json!([1, 2, 3])).is_none());
    }

    #[test]
    fn an_end_before_the_start_is_a_typo_and_leaves_a_moment() {
        let event =
            parsed(json!({ "date": "2025-09-05", "end": "2025-09-01", "title": "x" })).unwrap();
        assert_eq!(event.end, None);
        assert!(!event.spans());
    }

    #[test]
    fn an_end_equal_to_the_start_is_kept_but_does_not_span() {
        // Python keeps it (`end >= start`) and `spans` is strict (`end >
        // start`), so it renders as a marker. Both halves are load-bearing: a
        // zero-width `<rect>` is not a mark.
        let event =
            parsed(json!({ "date": "2025-09-05", "end": "2025-09-05", "title": "x" })).unwrap();
        assert!(event.end.is_some());
        assert!(!event.spans());
    }

    #[test]
    fn cited_messages_are_capped_and_non_integers_dropped() {
        let ids: Vec<Value> = (0..60).map(|n| json!(n)).collect();
        let event = parsed(json!({ "date": "2025-09-05", "title": "x", "messages": ids })).unwrap();
        assert_eq!(event.messages.len(), MAX_CITED);

        let event =
            parsed(json!({ "date": "2025-09-05", "title": "x", "messages": [1, "two", null, 3] }))
                .unwrap();
        assert_eq!(event.messages, vec![1, 3]);
    }

    #[test]
    fn who_and_tags_drop_blanks_and_non_strings() {
        let event = parsed(json!({
            "d": "2025-09-05", "title": "x",
            "who": ["Ana", "  ", null, 7, " Bob "], "tags": []
        }))
        .unwrap();
        assert_eq!(event.who, vec!["Ana", "Bob"]);
        assert!(event.tags.is_empty());
    }

    #[test]
    fn kind_weight_and_confidence_are_lowercased() {
        let event = parsed(json!({
            "date": "2025-09-05", "title": "x",
            "kind": " Decision ", "confidence": "HIGH", "w": "Major"
        }))
        .unwrap();
        assert_eq!(event.kind, "decision");
        assert_eq!(event.confidence, "high");
        assert_eq!(event.weight, "major");
    }

    #[test]
    fn events_sort_on_date_then_time_then_title() {
        // Time before title: several of the existing notes fall on one day, and
        // ordering on the day alone reads as a sequence that never happened.
        let raw = json!([
            { "d": "2025-09-05", "t": "22:00", "title": "z" },
            { "d": "2025-09-09", "t": "08:00", "title": "b" },
            { "d": "2025-09-05", "t": "07:30", "title": "a" },
            { "d": "2025-09-05", "title": "no time" },
        ]);
        let mut events: Vec<Event> = raw
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter_map(|(i, v)| one(v, i))
            .collect();
        events.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then_with(|| a.time.cmp(&b.time))
                .then_with(|| a.title.cmp(&b.title))
        });
        let titles: Vec<&str> = events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["no time", "a", "z", "b"]);
    }

    #[test]
    fn coverage_is_read_and_an_empty_block_is_the_same_as_none() {
        let body = json!({ "coverage": {
            "from": "2025-09-05", "to": "2025-10-05",
            "level": "prekretnice", "note": "Only the turning points."
        }, "events": [] });
        let cover = coverage_of(&body).expect("a stated coverage");
        assert_eq!(cover.from, Some(day(2025, 9, 5)));
        assert_eq!(cover.to, Some(day(2025, 10, 5)));
        assert_eq!(cover.level, "prekretnice");

        assert!(coverage_of(&json!({ "coverage": {} })).is_none());
        assert!(coverage_of(&json!({ "coverage": "soon" })).is_none());
        assert!(coverage_of(&json!({})).is_none());
    }

    #[test]
    fn the_kinds_present_come_from_the_file_and_not_from_the_vocabulary() {
        // A file using three of the four must not get a dead control for the
        // fourth, and a file using a word nobody anticipated must still get a
        // filter.
        let notes = Notes {
            events: vec![
                parsed(json!({ "d": "2025-01-01", "title": "a", "kind": "drama" })).unwrap(),
                parsed(json!({ "d": "2025-01-02", "title": "b", "kind": "pokret" })).unwrap(),
                parsed(json!({ "d": "2025-01-03", "title": "c", "kind": "drama" })).unwrap(),
                parsed(json!({ "d": "2025-01-04", "title": "d", "kind": "svojeglavo" })).unwrap(),
            ],
            ..Default::default()
        };
        assert_eq!(notes.kinds(), vec!["drama", "pokret", "svojeglavo"]);
    }

    #[test]
    fn the_sample_in_the_brief_parses_into_the_event_it_describes() {
        // The brief is what a model is handed. If the example in it does not
        // load, every answer written from it is malformed and the report
        // silently shows nothing.
        let sample = json!({
            "id": "migration",
            "date": "2025-12-14",
            "end": "2025-12-16",
            "kind": "milestone",
            "title": "Moved off the old group",
            "summary": "One or two sentences.",
            "topic": 0,
            "messages": [1, 2, 17],
            "confidence": "high"
        });
        let event = parsed(sample).expect("the documented shape must load");
        assert_eq!(event.id, "migration");
        assert!(event.spans());
        assert_eq!(event.topic, Some(0));
        assert_eq!(event.messages, vec![1, 2, 17]);
        assert!(BRIEF.contains("\"confidence\": \"high\""));
    }

    #[test]
    fn a_missing_file_is_the_ordinary_case_and_says_nothing() {
        let notes = load(Path::new(r"C:\definitely-not-an-export"));
        assert!(notes.is_empty());
        assert_eq!(notes.source, "", "no file is not a complaint");
    }
}
