//! Every figure the analyser computes, as named types.
//!
//! This is the whole interface between the two halves of the program:
//! `tga-metrics` fills a [`Stats`] in, `tga-report` reads one out, and neither
//! knows the other exists. That is the same property the layering rule always
//! had — it is what lets `--stats` dump a run and `--from-stats` replay it
//! through the writer with no export on disk — except that the shape now has
//! names the compiler checks, where before it was a `serde_json::Value` and a
//! misspelt key read as zero.
//!
//! # Two things about the JSON
//!
//! **Key order is not declaration order.** [`write`] sorts every object, so
//! moving a field here moves nothing in the file. `save.bat baseline` compares
//! a dump against an earlier copy of itself byte for byte, and a diff that
//! reorders itself when somebody rearranges a struct is a diff nobody reads.
//!
//! **A `[label, count]` pair is a [`Count`], and it still serialises as a
//! two-element array.** There are nine of these branches — days, months, emoji,
//! domains, hashtags, mentions, stickers, forward sources and message lengths.
//! Named fields on this side, the same bytes on the other.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// the shared shapes
// ---------------------------------------------------------------------------

/// One labelled count, written as `["2025-09-05", 41]`.
///
/// The `from`/`into` pair is what keeps the two-element array in the file while
/// the code says `.label` and `.n`. Worth the twelve lines: this shape appears
/// nine times and `row.0` / `row.1` at every use site is exactly the sort of
/// thing that reads fine until somebody swaps them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "(String, i64)", into = "(String, i64)")]
pub struct Count {
    pub label: String,
    pub n: i64,
}

impl Count {
    pub fn new(label: impl Into<String>, n: i64) -> Self {
        Count {
            label: label.into(),
            n,
        }
    }
}

impl From<(String, i64)> for Count {
    fn from((label, n): (String, i64)) -> Self {
        Count { label, n }
    }
}

impl From<Count> for (String, i64) {
    fn from(count: Count) -> Self {
        (count.label, count.n)
    }
}

/// A figure that is a number in most rows and a piece of text in one.
///
/// "Largest file" reports `1.8 MB`; every other superlative reports a count.
/// The alternative was to format the number away in the metrics and hand the
/// report a string it cannot do arithmetic on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Figure {
    Count(i64),
    Text(String),
}

impl Figure {
    /// How the report prints it. A float keeps the trailing `.0` it was
    /// rounded to, which is why this is not `to_string` on the number.
    pub fn display(&self) -> String {
        match self {
            Figure::Count(n) => n.to_string(),
            Figure::Text(text) => text.clone(),
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Figure::Count(n) => Some(*n),
            Figure::Text(_) => None,
        }
    }
}

impl Default for Figure {
    fn default() -> Self {
        Figure::Count(0)
    }
}

/// A sparse `{day: count}` series, one per person or per topic.
///
/// Sparse rather than dense because the report draws each of these as a row on
/// the same axis as the whole-archive ribbon and densifies against that axis at
/// draw time. Dense here would be one entry per person per day of the archive,
/// which on a large group is millions of zeroes nobody reads.
pub type Sparse = BTreeMap<String, i64>;

// ---------------------------------------------------------------------------
// the whole dump
// ---------------------------------------------------------------------------

/// **Every field defaults.** The writer is handed a file that something else
/// produced — `--from-stats` takes a dump off disk — and half a report is worth
/// more than a stack trace, so a branch that is missing renders as an empty
/// section rather than failing to load. The accessors this replaced had the
/// same property one field at a time; here it is stated once.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub export: ExportInfo,
    pub activity: Activity,
    pub people: People,
    pub content: Content,
    pub conversation: Conversation,
    pub graph: Graph,
    pub topics: Vec<Topic>,
    pub superlatives: Vec<Superlative>,
    pub awards: Vec<Award>,
    /// Absent when nobody posted on two consecutive days.
    pub streak: Option<Streak>,
    pub churn: Churn,
    pub renamed: Vec<Renamed>,
    /// `None` in a dump recorded before this branch existed, which is a
    /// different thing from an archive with nothing in it — the first renders
    /// no section at all and the second renders one that says so. Always
    /// written by a live run.
    pub dynamics: Option<Dynamics>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportInfo {
    pub name: String,
    pub root: String,
    pub topics: usize,
    pub messages: usize,
    pub service_messages: usize,
    /// `None` when the export carries no member list at all, which is a
    /// different fact from a list of nobody.
    pub members_count: Option<i64>,
    pub roster_complete: Option<bool>,
}

// ---------------------------------------------------------------------------
// activity
// ---------------------------------------------------------------------------

/// When the group talked.
///
/// Every clock-face and calendar figure is built from the export's local wall
/// clock: "who posts at 3am" is a question about the clock in the room.
///
/// `empty` is the flag every reader checks first. When it is set the rest is
/// left at its default rather than absent — an export with nothing in it is
/// rare enough that a second shape for it was not worth carrying.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Activity {
    pub empty: bool,
    pub first: String,
    pub last: String,
    pub span_days: usize,
    pub active_days: usize,
    pub mean_per_active_day: f64,
    /// **Dense**: every date between the first message and the last, silent
    /// days included, at zero. A sparse series drawn as a line closes a
    /// two-month gap into a straight segment and invents activity.
    pub per_day: Vec<Count>,
    pub per_month: Vec<Count>,
    /// Fixed by the clock, not by the data.
    pub per_hour: [i64; 24],
    pub per_weekday: [i64; 7],
    /// `[weekday][hour]`, Monday first.
    pub hour_weekday: [[i64; 24]; 7],
    pub busiest_day: Busiest,
    /// The longest run of silent days. `None` when there was none — reported
    /// because a month of nothing is as much a fact about a group as its
    /// busiest week, and a ribbon renders it as blank space nobody can measure.
    pub quietest: Option<Gap>,
    pub per_day_by_topic: BTreeMap<String, Sparse>,
    pub per_day_by_person: BTreeMap<String, Sparse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Busiest {
    pub date: String,
    pub messages: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gap {
    pub days: usize,
    pub from: String,
    pub to: String,
}

// ---------------------------------------------------------------------------
// people
// ---------------------------------------------------------------------------

/// Who talked, how much, and where.
///
/// One row per identity, never per name. Two counts are kept apart on purpose
/// and must not be added together: `messages` is what somebody sent, and
/// `votes_*` is reactions *given*, which is a floor rather than a total —
/// Telegram names at most three reactors per message and never names an
/// anonymous one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct People {
    pub rows: Vec<Person>,
    pub speakers: usize,
    pub known_members: usize,
    pub roster_complete: Option<bool>,
    /// The exporter stopped collecting members before it ran out of them.
    /// Distinct from `roster_complete`; see `tga_read::Export`.
    pub roster_capped: Option<bool>,
    /// People with a roster entry who also posted — the overlap, which is the
    /// number that says whether the member list and the history describe the
    /// same group. A roster of 43 against 45 speakers sounds like a match until
    /// you notice only 41 appear in both.
    pub members_who_spoke: usize,
    pub silent_members: usize,
    pub total_messages: usize,
    /// What share of the conversation the loudest three carry. One number for
    /// "is this a group or a broadcast".
    pub top3_share: f64,
    pub votes_named: i64,
    pub votes_total: i64,
    pub votes_anonymous: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Person {
    pub key: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub role: String,
    pub joined: Option<String>,
    /// The `@handle`, from the member list, without the `@`.
    ///
    /// The only stable identifier a reader can act on: a display name changes
    /// whenever its owner feels like it — this archive has 11 people who
    /// renamed themselves — and the peer key is a number that means nothing to
    /// anybody. Skipped when empty, which is most people in an export with no
    /// member list.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub username: String,
    /// Telegram says this account is a bot.
    ///
    /// Worth separating because a bot on the member list is not a person who
    /// chose not to speak, and lumping the two together makes the silence look
    /// more sociable than it was.
    #[serde(skip_serializing_if = "is_false")]
    pub bot: bool,
    /// This person has a row in `participants.json`.
    ///
    /// Recorded rather than inferred. The obvious proxies are both wrong: a
    /// member may have no `@handle` (two of UA KOLAB's 43 have never set one)
    /// and `silent` is only true for a member who *also* never appears in the
    /// history. Deriving membership from either quietly drops real members from
    /// the one table whose job is to be complete.
    #[serde(skip_serializing_if = "is_false")]
    pub listed: bool,
    pub messages: i64,
    pub words: i64,
    pub chars: i64,
    pub media: i64,
    pub forwards: i64,
    pub replies: i64,
    pub edited: i64,
    pub reactions_received: i64,
    pub reacted_messages: i64,
    pub votes_given: i64,
    pub first: Option<String>,
    pub last: Option<String>,
    pub by_topic: BTreeMap<String, i64>,
    pub hours: [i64; 24],
    pub avg_words: f64,
    pub share: f64,
    /// Somebody the member list knows about who never appears in the history.
    ///
    /// Written only when true, and absent otherwise, because it is a fact about
    /// a roster row rather than a field every person has. In a group of 43 with
    /// 18 lurkers, "43 members" and "25 people talked" are both true and only
    /// one of them describes the conversation.
    #[serde(skip_serializing_if = "is_false")]
    pub silent: bool,
}

fn is_false(flag: &bool) -> bool {
    !*flag
}

// ---------------------------------------------------------------------------
// content
// ---------------------------------------------------------------------------

/// What the messages were made of.
///
/// Two media figures are easy to conflate and are kept apart. `sent` counts
/// every message that carried an attachment, which is a fact about the
/// conversation; `saved` counts the ones whose bytes are on disk, which is a
/// fact about the *export*. Bytes are the sizes Telegram reported, so the total
/// covers skipped files too.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Content {
    pub messages: usize,
    pub with_text: i64,
    pub edited: i64,
    pub total_words: i64,
    pub total_chars: i64,
    pub mean_words: f64,
    pub lengths: Vec<Count>,
    pub media_kinds: Vec<MediaKind>,
    pub media_messages: i64,
    pub media_saved: i64,
    pub media_bytes: i64,
    pub emoji: Vec<Count>,
    pub emoji_total: i64,
    /// Peer key -> their three most-used emoji.
    pub emoji_by_person: BTreeMap<String, Vec<Count>>,
    pub stickers: Vec<Count>,
    pub domains: Vec<Count>,
    pub links_total: i64,
    pub hashtags: Vec<Count>,
    pub mentions: Vec<Count>,
    pub forwards: i64,
    pub forward_sources: Vec<Count>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaKind {
    pub kind: String,
    pub label: String,
    pub sent: i64,
    pub saved: i64,
    pub bytes: i64,
}

// ---------------------------------------------------------------------------
// conversation
// ---------------------------------------------------------------------------

/// The shape of the conversation: who answers whom, and how fast.
///
/// Latency is measured on epoch seconds, never the wall clock — a reply twenty
/// minutes after its parent across a DST boundary is twenty minutes, and
/// subtracting naive local timestamps would call it eighty.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conversation {
    pub replies: i64,
    pub reply_share: f64,
    /// Replies whose parent is outside the export.
    pub orphan_replies: i64,
    pub self_replies: i64,
    pub latency_counted: usize,
    pub latency_median: i64,
    pub latency_p90: i64,
    /// A reply longer than this after its parent is somebody returning to a
    /// thread days later, not a response time. It still counts as a reply and
    /// is left out of the latency figures, which the report says.
    pub latency_cap: i64,
    pub fastest: Vec<Latency>,
    pub slowest: Vec<Latency>,
    /// Directed: author of the reply -> author of the parent. A reply to your
    /// own message is dropped, or the loudest person becomes their own closest
    /// correspondent.
    pub edges: Vec<Edge>,
    pub reaction_edges: Vec<Edge>,
    pub sessions: usize,
    /// A pause longer than this ends the session. Stated rather than hidden in
    /// a comparison, so the reader can disagree with it.
    pub session_gap: i64,
    pub session_median_messages: i64,
    pub session_median_voices: i64,
    pub session_longest: Option<Session>,
    /// Who opens a burst of talk, which is not the same list as who says most.
    pub starters: Vec<Starter>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Starter {
    pub key: String,
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Latency {
    pub key: String,
    pub name: String,
    pub replies: usize,
    pub median: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub count: i64,
    pub from_name: String,
    pub to_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    pub topic: usize,
    pub start: String,
    pub messages: usize,
    pub voices: usize,
    pub minutes: f64,
    pub opened_by: String,
}

// ---------------------------------------------------------------------------
// graph
// ---------------------------------------------------------------------------

/// Who talks to whom, laid out as a diagram.
///
/// Edges are **undirected and pooled**: a reply from A to B and a reaction from
/// B to A are both "these two are in contact", and drawing them as two arcs
/// says twice as much as the data supports. Direction is still in the matrix,
/// which is the view that can carry it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Link>,
    /// People left out because the picture would be a hairball. The report says
    /// how many.
    pub hidden: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Node {
    pub key: String,
    pub x: f64,
    pub y: f64,
    pub degree: i64,
    pub messages: i64,
    pub size: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Link {
    pub a: String,
    pub b: String,
    pub weight: i64,
}

// ---------------------------------------------------------------------------
// topics, records, arrivals
// ---------------------------------------------------------------------------

/// `#[serde(default)]` because this struct grows: a dump recorded before a
/// column existed must still render, with the new cell empty rather than the
/// whole load failing. The same reasoning as on `Stats` itself, one level down.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Topic {
    pub index: usize,
    pub name: String,
    pub messages: usize,
    pub voices: usize,
    pub words: i64,
    /// Absent on a topic nobody posted in — there is no average of nothing,
    /// and zero would read as "they wrote empty messages".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_words: Option<f64>,
    pub media: usize,
    pub replies: usize,
    pub reactions: i64,
    pub first: String,
    pub last: String,
    pub top: String,
    /// Days this topic carried at least one message.
    ///
    /// Not derivable from `first` and `last`, and the difference is the whole
    /// point: a topic open for 278 days and posted in on 12 of them is a
    /// different animal from one posted in daily, and the two look identical
    /// in a date range.
    pub active_days: usize,
    /// This topic's own loudest day.
    ///
    /// Deliberately not the archive's busiest day cut to this topic — the
    /// archive's peak is usually one topic's spike, and reporting it on every
    /// row would say the same thing ten times and tell you nothing about nine
    /// of them.
    pub busiest: Busiest,
    /// Hour of day, this topic alone, on `Msg::when` like every other clock
    /// face.
    ///
    /// The report scales each row to its own maximum rather than to a shared
    /// one, so the rows compare by *shape*. That is the only way a
    /// 1,527-message topic and a 139,743-message one can be read on one grid:
    /// on a shared scale the small one is a row of black and says nothing.
    pub per_hour: [i64; 24],
}

/// A record, and the message that backs it.
///
/// Nothing here reports a record without saying which message it was: an
/// unsourced "busiest day" is a number the reader cannot check. The two that
/// are about the calendar rather than a message carry no id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Superlative {
    pub title: String,
    pub id: Option<i64>,
    pub topic: Option<usize>,
    pub date: String,
    pub who: String,
    pub text: String,
    pub value: Figure,
    pub unit: String,
}

/// A per-person record, restricted to people with enough messages to mean it.
///
/// Without the floor every one of these is won by somebody who sent four
/// messages, all of them at 4am, and the report says something true about
/// nobody.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Award {
    pub title: String,
    pub name: String,
    pub key: String,
    /// Already formatted — a percentage for the two hour-share awards and one
    /// decimal for the rest.
    pub value: String,
    pub note: String,
    pub messages: i64,
}

/// The longest run of consecutive days somebody posted on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Streak {
    pub name: String,
    pub days: usize,
    pub ended: String,
}

/// Arrivals, departures, and how many people were actually talking.
///
/// Arrivals come from two places that disagree, and both are reported. The
/// member list dates every *current* member's join, which is the better source
/// but says nothing about anyone who has since left; the service messages in
/// the history record joins and removals as they happened, but only the ones
/// Telegram announced.
///
/// The four count vectors are all indexed by `months`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Churn {
    pub months: Vec<String>,
    pub roster_joins: Vec<i64>,
    pub announced_joins: Vec<i64>,
    pub announced_leaves: Vec<i64>,
    pub active: Vec<usize>,
    pub events: Vec<ChurnEvent>,
    pub roster_dated: i64,
    pub roster_size: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChurnEvent {
    pub date: String,
    /// `join` or `leave`.
    pub kind: String,
    pub count: usize,
    pub who: String,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Renamed {
    pub name: String,
    pub aliases: Vec<String>,
}

// ---------------------------------------------------------------------------
// dynamics
// ---------------------------------------------------------------------------

/// How the group behaves *towards each other*, and how that changes over time.
///
/// Five figures the counting elsewhere leaves unanswerable. As with
/// [`Activity`], `empty` is the flag and the rest is left at its default when
/// it is set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dynamics {
    pub empty: bool,
    pub pairs: Pairs,
    pub answer: Answer,
    pub tenure: Tenure,
    pub retention: Retention,
    pub depth: Depth,
}

/// Reply edges collapsed onto unordered pairs.
///
/// **Ranked on `min(there, back)` rather than the total, and that choice is the
/// whole figure.** A directed edge cannot tell a conversation from a broadcast:
/// a pair where one person sent 300 replies and got 2 back has a large total
/// and is not a correspondence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pairs {
    pub rows: Vec<Pair>,
    pub shown: usize,
    pub mutual: i64,
    pub one_way: i64,
    pub directed: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pair {
    pub a: String,
    pub b: String,
    pub a_name: String,
    pub b_name: String,
    pub a_to_b: i64,
    pub b_to_a: i64,
    /// The smaller direction, which is what the list is ranked on.
    pub both: i64,
    pub replies: i64,
    /// 1.0 is an even exchange, 0.0 is one person talking.
    pub balance: f64,
}

/// Reply latency, cut by the hour the message being answered was posted.
///
/// Keyed on the **parent's** hour, not the reply's. "How long until somebody
/// answers me" is a question about when you posted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Answer {
    pub counts: [i64; 24],
    pub medians: [i64; 24],
    pub counted: i64,
    /// Replies an hour needs before its median is quoted as the group's
    /// behaviour. Below it, one slow afternoon *is* the median.
    pub minimum: i64,
    pub cap: i64,
    pub fastest_hour: Option<HourPick>,
    pub slowest_hour: Option<HourPick>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HourPick {
    pub hour: usize,
    pub median: i64,
    pub replies: i64,
}

/// Where each person sits between their first message and the end of the
/// archive.
///
/// Dormancy is measured against the **archive's** last day rather than today,
/// because an export is a fixed document: reading the same file a year later
/// must not silently reclassify everyone in it as gone.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tenure {
    pub rows: Vec<TenureRow>,
    pub as_of: String,
    pub active: i64,
    pub fading: i64,
    pub gone: i64,
    pub active_within: i64,
    pub fading_within: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TenureRow {
    pub key: String,
    pub name: String,
    pub messages: i64,
    pub first: String,
    pub last: String,
    pub span_days: i64,
    pub active_days: usize,
    /// What share of the days they were around on they actually spoke.
    pub density: f64,
    pub dormant_days: i64,
    /// `active`, `fading` or `gone`.
    pub status: String,
}

/// Of the people who spoke last month, how many spoke again.
///
/// Nobody is announced for going quiet, and going quiet is how a group actually
/// ends. The axis is every month between the first and the last, silent ones
/// included — taking only the months that appear closes a six-month gap into a
/// single step and reports the return after it as ordinary retention.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Retention {
    pub months: Vec<String>,
    pub active: Vec<i64>,
    pub new: Vec<i64>,
    pub returning: Vec<i64>,
    pub lost: Vec<i64>,
    pub people: usize,
    /// The mean of the monthly rates, not the ratio of the totals: every month
    /// gets one vote, so a single enormous month cannot speak for the years
    /// around it.
    pub kept_mean: f64,
    pub months_counted: usize,
}

/// Reply-chain length: how many messages deep a thread got.
///
/// Depth 1 is every message nobody was answering, so it is most of the archive
/// and says nothing; the distribution starts at 2, where a chain begins.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Depth {
    pub buckets: Vec<Count>,
    /// Lengths are reported individually up to here and pooled above it.
    pub cap: i64,
    pub chained: usize,
    pub max: i64,
    pub median: i64,
    pub mean: f64,
    pub longest: Option<Deepest>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Deepest {
    pub id: i64,
    pub topic: usize,
    pub date: String,
    pub messages: i64,
}

// ---------------------------------------------------------------------------
// the dump
// ---------------------------------------------------------------------------

/// The dump `--stats` writes: sorted keys, one-space indent, UTF-8 as it is.
///
/// **The sort is done here rather than left to the map type, and that is not
/// belt and braces.** `serde_json`'s object is a `BTreeMap` by default, which
/// sorts on its own — but its `preserve_order` feature swaps in an `IndexMap`,
/// and cargo unifies features across everything built in one invocation. The
/// window's toolkit turned that feature on somewhere in its tree, so
/// building one crate alone and building the whole workspace
/// produced *differently ordered dumps from the same numbers*, and only one of
/// them matched the recorded baseline. Sorting explicitly makes the file the
/// same file however the binary that wrote it was built, and keeps it that way
/// the next time a dependency reaches for the same feature.
///
/// The indent is one space rather than `to_string_pretty`'s two only because a
/// 777 KB dump is 777 KB either way and the narrower one wraps less.
pub fn write(stats: &Stats) -> serde_json::Result<String> {
    let value = sorted(serde_json::to_value(stats)?);
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    Serialize::serialize(&value, &mut ser)?;
    Ok(String::from_utf8(buf).expect("serde_json emits UTF-8"))
}

/// Every object in the tree, rebuilt with its keys in order.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut pairs: Vec<(String, Value)> = map.into_iter().collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                pairs
                    .into_iter()
                    .map(|(key, inner)| (key, sorted(inner)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_is_still_a_two_element_array_in_the_file() {
        // The whole point of the `from`/`into` pair. If this ever changes,
        // every recorded dump stops loading and `save.bat baseline` fails on
        // nine branches at once.
        let count = Count::new("2025-09-05", 41);
        let text = serde_json::to_string(&count).unwrap();
        assert_eq!(text, r#"["2025-09-05",41]"#);
        assert_eq!(serde_json::from_str::<Count>(&text).unwrap(), count);
    }

    #[test]
    fn a_superlative_carries_either_a_number_or_a_text() {
        assert_eq!(
            serde_json::to_string(&Figure::Count(41)).unwrap(),
            "41",
            "a count must not gain quotes"
        );
        assert_eq!(
            serde_json::to_string(&Figure::Text("1.8 MB".into())).unwrap(),
            r#""1.8 MB""#
        );
        // And back: untagged tries the number first, so "1.8 MB" cannot be
        // silently read as one.
        assert_eq!(
            serde_json::from_str::<Figure>("41").unwrap(),
            Figure::Count(41)
        );
        assert_eq!(
            serde_json::from_str::<Figure>(r#""1.8 MB""#).unwrap(),
            Figure::Text("1.8 MB".into())
        );
    }

    #[test]
    fn a_person_who_spoke_carries_no_silent_key_at_all() {
        // It is a fact about a member-list row, not a field every person has,
        // and `"silent": false` on 250 speakers is 250 lines of noise.
        let speaker = Person::default();
        assert!(!serde_json::to_string(&speaker).unwrap().contains("silent"));
        let lurker = Person {
            silent: true,
            ..Default::default()
        };
        assert!(serde_json::to_string(&lurker).unwrap().contains("silent"));
    }

    #[test]
    fn an_empty_topic_has_no_average_rather_than_an_average_of_zero() {
        let empty = Topic::default();
        assert!(!serde_json::to_string(&empty).unwrap().contains("avg_words"));
    }

    #[test]
    fn the_dump_comes_out_with_its_keys_sorted_at_every_level() {
        // **This failed under `cargo test --all` and passed under
        // `cargo test -p tga-stats`**, which is how the `preserve_order` note
        // on `write` was found: the map type changes with the feature set, so
        // the sort cannot be left to it. Written against the text rather than
        // against the parsed value, because the parsed value is the thing whose
        // ordering is in question.
        let text = write(&Stats::default()).unwrap();
        // One space of indent per level, so a top-level key is the only kind
        // that starts with exactly one space and then a quote.
        let top: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with(" \""))
            .filter_map(|line| line.split('"').nth(1))
            .collect();
        let mut in_order = top.clone();
        in_order.sort();
        assert_eq!(top, in_order, "the top level came out unsorted");
        assert_eq!(top.first(), Some(&"activity"));
        assert_eq!(top.last(), Some(&"topics"));
    }

    #[test]
    fn the_sort_reaches_every_level_and_inside_arrays() {
        // The top-level check above cannot see this: a branch is one key at the
        // top and the object it holds is where nearly every key in the dump
        // actually lives.
        let messy = serde_json::json!({
            "z": 1,
            "a": { "zz": 1, "aa": [ { "zzz": 1, "aaa": 2 } ] },
        });
        let text = serde_json::to_string(&sorted(messy)).unwrap();
        assert_eq!(text, r#"{"a":{"aa":[{"aaa":2,"zzz":1}],"zz":1},"z":1}"#);
    }

    #[test]
    fn a_dump_reads_back_into_the_same_numbers() {
        // The `--from-stats` round trip, which is the one thing the dump has to
        // do beyond being readable.
        let stats = Stats {
            activity: Activity {
                per_day: vec![Count::new("2025-01-01", 3)],
                per_hour: [1; 24],
                ..Default::default()
            },
            ..Default::default()
        };
        let back: Stats = serde_json::from_str(&write(&stats).unwrap()).unwrap();
        assert_eq!(back.activity.per_day, stats.activity.per_day);
        assert_eq!(back.activity.per_hour, stats.activity.per_hour);
    }
}
