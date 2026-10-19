//! A summary of what happened in a chat between two dates, written by a local
//! model.
//!
//! **In:** the range's messages as `tga_notes::Row`s, already chosen by the
//! caller (`tga-run`), which reads the export the way the digest does. **Out:**
//! text, streamed as the model writes it, with every citation checked.
//!
//! **The shape of a run.** If the whole range fits one call, the model reads
//! the messages and writes the summary. If not, it is cut into parts at day
//! boundaries ([`cut`]); each part becomes notes — short facts, each with the
//! ids it rests on — and the summary is written from the notes. Notes are
//! kept ([`Cache`]), so a re-run, an overlapping range or a sync that changed
//! one day redoes only what changed. By topic, the same runs once per forum
//! topic, with an overview above the detailed lengths. Below it all, every
//! document read in the range is explained in a call of its own — in the
//! summary it is one line among hundreds — except in a catch-up's brief.
//!
//! **The model is on this machine.** [`Client`] takes a port, not a host. And
//! the narration — [`Event::Say`] — carries counts, never a message's text,
//! because the caller logs it.

pub mod cache;
pub mod cite;
pub mod client;
pub mod cut;
pub mod lines;
pub mod ocr;
pub mod prompt;
pub mod server;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use serde_json::{json, Value};
use tga_notes::Row;

pub use cache::Cache;
pub use client::{Client, Props, Streamed, Timings};
pub use server::{Keeper, Launch, Want};

/// Messages are read in parts no bigger than this: ReVision's `CHUNK_TOKENS`
/// (about 150 messages), the size at which its extraction found 79% of
/// events rather than 47%. A model reading a long stretch skims, and a fact
/// said once in one message — or a discussion with no decision at its end —
/// is what it skips.
///
/// It was 8,000 here, with that finding quoted beside it, until a summary
/// left out a discussion the account holder knew mattered (2026-10-15).
pub const PART_TOKENS: usize = 4_500;

/// A range up to this size is summarised from the messages in one call;
/// anything bigger goes through notes on parts first. The same as a part, so
/// no stretch of messages is ever read at a size the finding above warns off.
pub const SINGLE_TOKENS: usize = PART_TOKENS;

/// The most one call reads that is not raw messages to summarise — the notes
/// a summary is written from, or the days a question is asked about:
/// ReVision's `INPUT_CAP`, 42,000 characters, about 14,000 tokens. The rest
/// of the 32K context is the answer's.
pub const INPUT_TOKENS: usize = 14_000;

/// The longest notes answer for one part.
const NOTES_ANSWER: usize = 3_000;

/// The overview paragraph above a by-topic summary.
const OVERVIEW_ANSWER: usize = 600;

/// One document's explanation: a paragraph, then all of it, every item of an
/// agenda or a ballot included. Beside a document cut at
/// `tga_docs::MAX_CHARS` it still fits the 32K context.
const DOCUMENT_ANSWER: usize = 4_000;

/// Left free in every call: the chat template's own tokens.
const MARGIN: usize = 256;

/// Below this the context is too small to be worth cutting for.
const LEAST_BUDGET: usize = 1_000;

/// A topic with fewer messages than this in the range is named on one line
/// at the end rather than summarised: there is nothing to summarise.
pub const LEAST_TOPIC: usize = 5;

/// Notes are merged at most this many times before the least important are
/// dropped instead.
const MERGE_ROUNDS: usize = 3;

/// Tries at a notes answer that is not valid JSON, as ReVision's runner.
const ATTEMPTS: usize = 3;

/// The pause before trying again after the server failed (a 5xx), doubled
/// each time: a slot being reset is over in a second or two, and three tries
/// back to back are spent before it is.
const BACKOFF: Duration = Duration::from_secs(1);

/// A part of more messages than this that the model finds nothing in is not
/// kept. A stretch that long with nothing worth a note is rare, and an empty
/// answer kept would leave it out of every later summary of those days.
const NOTHING_TO_NOTE: usize = 10;

/// How much of a server's refusal is repeated in the narration.
const REFUSAL_CHARS: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no model server is answering on 127.0.0.1:{0}")]
    NoServer(u16),
    #[error("the model server refused: {0}")]
    Server(String),
    #[error("{0}")]
    Bad(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Length {
    /// One paragraph.
    #[default]
    Short,
    /// Prose, one theme per paragraph, cited.
    Detailed,
    /// Headed sections of bullets, cited.
    Sections,
    /// A catch-up: one bold line, then a few cited bullets. What a catch-up
    /// across many chats writes for each, to be skimmed one after another.
    Brief,
}

impl Length {
    pub const ALL: [Length; 4] = [
        Length::Brief,
        Length::Short,
        Length::Detailed,
        Length::Sections,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Length::Short => "short",
            Length::Detailed => "detailed",
            Length::Sections => "sections",
            Length::Brief => "brief",
        }
    }

    /// What the window calls it.
    pub fn label(self) -> &'static str {
        match self {
            Length::Short => "Short",
            Length::Detailed => "Detailed",
            Length::Sections => "Sections",
            Length::Brief => "Brief",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.code() == s)
    }

    /// The longest answer this length may write.
    fn answer(self) -> usize {
        match self {
            Length::Short => 700,
            Length::Brief => 1_000,
            Length::Detailed | Length::Sections => 2_500,
        }
    }

    fn cites(self) -> bool {
        self != Length::Short
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// Latin script, ekavian.
    #[default]
    Serbian,
    English,
    /// Whatever most of the messages are in.
    AsChat,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::Serbian, Lang::English, Lang::AsChat];

    pub fn code(self) -> &'static str {
        match self {
            Lang::Serbian => "sr",
            Lang::English => "en",
            Lang::AsChat => "chat",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.code() == s)
    }
}

/// One fact from one part, and the messages it rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub day: String,
    pub weight: String,
    pub what: String,
    pub ids: Vec<i64>,
}

impl Note {
    fn from_json(v: &Value) -> Option<Note> {
        let what = v.get("what")?.as_str()?.trim().to_string();
        if what.is_empty() {
            return None;
        }
        Some(Note {
            day: v
                .get("day")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            weight: v
                .get("weight")
                .and_then(Value::as_str)
                .unwrap_or("medium")
                .to_string(),
            what,
            ids: v
                .get("ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default(),
        })
    }

    fn all_from(v: &Value) -> Vec<Note> {
        v.get("notes")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Note::from_json).collect())
            .unwrap_or_default()
    }

    fn to_json(&self) -> Value {
        json!({ "day": self.day, "weight": self.weight, "what": self.what, "ids": self.ids })
    }

    /// `- 2024-12-02 [major] A strike was called. [#1, #2]`
    fn line(&self) -> String {
        let ids = if self.ids.is_empty() {
            String::new()
        } else {
            let list: Vec<String> = self.ids.iter().map(|i| format!("#{i}")).collect();
            format!(" [{}]", list.join(", "))
        };
        format!("- {} [{}] {}{ids}", self.day, self.weight, self.what)
    }

    fn rank(&self) -> u8 {
        match self.weight.as_str() {
            "major" => 0,
            "medium" => 1,
            _ => 2,
        }
    }
}

/// What to summarise.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub chat: &'a str,
    /// The range's messages, oldest first.
    pub rows: &'a [Row],
    /// Topic names by `Row::topic`.
    pub topics: &'a [String],
    /// Whether the chat is a forum, which is when topics mean anything.
    pub forum: bool,
    pub by_topic: bool,
    pub length: Length,
    pub lang: Lang,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

/// What a run reports while it works.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Narration: counts, sizes, timings. **Never a message's text.**
    Say(String),
    /// One part's notes are in.
    Part {
        done: usize,
        of: usize,
        cached: bool,
        secs: f64,
        timings: Option<Timings>,
    },
    /// A section of the summary begins: a topic's name, or `None` for the
    /// overview above them.
    Section(Option<String>),
    /// A piece of the summary, as written, citations already checked.
    Text(String),
    /// The call that wrote the text above has finished.
    Written(Timings),
}

/// A finished summary.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub sections: Vec<Section>,
    /// Topics too quiet to summarise, and how many messages each had.
    pub quiet: Vec<(String, usize)>,
    pub messages: usize,
    /// Parts read for notes; zero when everything went in one call.
    pub parts: usize,
    /// Parts the model never gave a readable answer for. Their notes are
    /// missing from the summary, which is why the caller must say so.
    pub failed: usize,
    /// A section reached its answer's length limit and stops where the limit
    /// fell, which the caller must say for the same reason.
    pub cut: bool,
    pub model: String,
    pub lang: Lang,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// A topic's name; `None` for a whole-group summary or the overview.
    pub heading: Option<String>,
    pub text: String,
}

impl Summary {
    /// The summary as Markdown, with no header: the caller knows the chat and
    /// the date it was written, and this does not.
    pub fn body(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        for s in &self.sections {
            match &s.heading {
                Some(h) => out.push(format!("## {h}\n\n{}", demote(&s.text))),
                None => out.push(s.text.clone()),
            }
        }
        if !self.quiet.is_empty() {
            let label = match self.lang {
                Lang::Serbian => "Ostale teme",
                _ => "Other topics",
            };
            let list: Vec<String> = self
                .quiet
                .iter()
                .map(|(t, n)| format!("{t} ({n})"))
                .collect();
            out.push(format!("*{label}:* {}.", list.join(", ")));
        }
        out.join("\n\n")
    }

    /// Every message id the text cites.
    pub fn cited(&self) -> Vec<i64> {
        self.sections
            .iter()
            .flat_map(|s| cite::cited(&s.text))
            .collect()
    }
}

/// A sections summary under a topic heading: its own `## ` headings go one
/// level down.
fn demote(text: &str) -> String {
    text.lines()
        .map(|l| match l.strip_prefix("## ") {
            Some(rest) => format!("### {rest}"),
            None => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Write the summary.
pub fn summarise(
    client: &Client,
    request: &Request,
    cache: &Cache,
    cancel: &AtomicBool,
    on: &mut dyn FnMut(Event),
) -> Result<Summary, Error> {
    let props = client.props()?;
    if props.n_ctx == 0 {
        return Err(Error::Bad(
            "the model server did not say how big its context is".into(),
        ));
    }
    let run = Run {
        client,
        props,
        request,
        cache,
        cancel,
    };
    let mut summary = Summary {
        messages: request.rows.len(),
        model: run.props.model.clone(),
        lang: request.lang,
        ..Default::default()
    };

    let groups = groups(request);
    // Days of stickers, "ok" and "haha" only: the model would be handed an
    // empty input, and asked for a summary it writes one — of nothing.
    if groups.busy.is_empty() && groups.quiet.is_empty() {
        return Err(Error::Bad("nothing was said in these days".into()));
    }
    let headed = request.by_topic && request.forum;
    for (name, rows) in &groups.busy {
        if headed {
            on(Event::Section(Some(name.clone().unwrap_or_default())));
        }
        let text = run.one(name.as_deref(), rows, &mut summary, on)?;
        summary.sections.push(Section {
            heading: if headed { name.clone() } else { None },
            text,
        });
    }
    summary.quiet = groups.quiet;

    // A short or brief summary is the overview already: each topic's opens
    // on its gist.
    if headed
        && summary.sections.len() > 1
        && matches!(request.length, Length::Detailed | Length::Sections)
    {
        on(Event::Section(None));
        let (text, cut) = run.overview(&summary.sections, on)?;
        summary.cut |= cut;
        summary.sections.insert(
            0,
            Section {
                heading: None,
                text,
            },
        );
    }

    // A catch-up is skimmed beside other chats' and stays a few lines.
    if request.length != Length::Brief {
        if let Some(section) = run.documents(&mut summary, on)? {
            summary.sections.push(section);
        }
    }
    Ok(summary)
}

/// What the documents section is called.
fn documents_heading(lang: Lang) -> &'static str {
    match lang {
        Lang::Serbian => "Dokumenti",
        _ => "Documents",
    }
}

/// The rows to summarise, as one group or one per topic.
struct Groups<'r> {
    /// A topic's name (or `None` for the whole group) and its rows.
    busy: Vec<(Option<String>, Vec<&'r Row>)>,
    quiet: Vec<(String, usize)>,
}

fn groups<'r>(request: &Request<'r>) -> Groups<'r> {
    let name = |i: usize| {
        request
            .topics
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("Topic {i}"))
    };
    // ReVision's rule: "haha", "ok", a sticker alone are left out — unless
    // somebody answered them, when they are what the answer is to.
    let replied: HashSet<i64> = request.rows.iter().filter_map(|r| r.re).collect();
    let kept = |r: &&Row| !lines::trivial(r) || replied.contains(&r.id);
    let all: Vec<&Row> = request.rows.iter().filter(kept).collect();
    if !(request.by_topic && request.forum) {
        // One topic of a forum: its name goes in the prompt.
        let mut seen = request.rows.iter().map(|r| r.topic);
        let only = seen.next().filter(|first| seen.all(|t| t == *first));
        let topic = only.filter(|_| request.forum).map(name);
        // Nothing left once the trivial is dropped is no group at all, not
        // a group of nothing: the callers' "nothing was said" rests on it.
        let busy = if all.is_empty() {
            Vec::new()
        } else {
            vec![(topic, all)]
        };
        return Groups {
            busy,
            quiet: Vec::new(),
        };
    }
    let mut by: BTreeMap<usize, Vec<&Row>> = BTreeMap::new();
    for r in all {
        by.entry(r.topic).or_default().push(r);
    }
    let said = |rows: &[&Row]| rows.iter().filter(|r| r.service.is_none()).count();
    let mut busy: Vec<(usize, Vec<&Row>)> = Vec::new();
    let mut quiet = Vec::new();
    for (topic, rows) in by {
        if said(&rows) >= LEAST_TOPIC {
            busy.push((topic, rows));
        } else if said(&rows) > 0 {
            quiet.push((name(topic), said(&rows)));
        }
    }
    // The busiest first: a reader stops reading somewhere.
    busy.sort_by_key(|(_, rows)| std::cmp::Reverse(rows.len()));
    quiet.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    Groups {
        busy: busy
            .into_iter()
            .map(|(t, rows)| (Some(name(t)), rows))
            .collect(),
        quiet,
    }
}

/// One summary's worth of state shared by its calls.
struct Run<'a> {
    client: &'a Client,
    props: Props,
    request: &'a Request<'a>,
    cache: &'a Cache,
    cancel: &'a AtomicBool,
}

impl Run<'_> {
    fn tokens(&self, text: &str) -> Result<usize, Error> {
        self.client.tokens(text, self.cancel)
    }

    /// What a call may read, after its prompt and its answer.
    fn budget(&self, system: &str, answer: usize) -> Result<usize, Error> {
        let used = self.tokens(system)? + answer + MARGIN;
        let left = self.props.n_ctx.saturating_sub(used);
        if left < LEAST_BUDGET {
            return Err(Error::Bad(format!(
                "the model server's context ({} tokens) leaves only {left} for the messages",
                self.props.n_ctx
            )));
        }
        Ok(left)
    }

    fn about<'s>(
        &'s self,
        topic: Option<&'s str>,
        from: &'s str,
        to: &'s str,
    ) -> prompt::About<'s> {
        prompt::About {
            chat: self.request.chat,
            topic,
            from,
            to,
        }
    }

    /// One group's summary: the whole range, or one topic of it.
    fn one(
        &self,
        topic: Option<&str>,
        rows: &[&Row],
        summary: &mut Summary,
        on: &mut dyn FnMut(Event),
    ) -> Result<String, Error> {
        let req = self.request;
        let from = req.from.to_string();
        let to = req.to.to_string();
        let about = self.about(topic, &from, &to);
        let known: HashSet<i64> = rows.iter().map(|r| r.id).collect();
        let direct = prompt::final_text(&about, req.length, req.lang, false);
        let reserve = req.length.answer();
        let (text, cut) = match self.gather(
            &about,
            topic,
            rows,
            &direct,
            reserve,
            SINGLE_TOKENS,
            summary,
            on,
        )? {
            Gathered::Messages(text) => self.write(
                &direct,
                &text,
                req.length.answer(),
                &known,
                req.length.cites(),
                on,
            )?,
            Gathered::Notes(notes, _) => {
                let system = prompt::final_text(&about, req.length, req.lang, true);
                let room = self.budget(&system, reserve)?.min(INPUT_TOKENS);
                let notes = self.fit(&about, notes, room, on)?;
                self.write(
                    &system,
                    &format!("Notes:\n\n{}", lines_of(&notes)),
                    req.length.answer(),
                    &known,
                    req.length.cites(),
                    on,
                )?
            }
        };
        summary.cut |= cut;
        Ok(text)
    }

    /// `rows` as the model reads them, by day — with a topic tag on each line
    /// only where the lines mix topics.
    fn days(&self, topic: Option<&str>, rows: &[&Row]) -> Vec<cut::Day> {
        let req = self.request;
        let mixed =
            req.forum && topic.is_none() && rows.windows(2).any(|w| w[0].topic != w[1].topic);
        let tag = |r: &Row| mixed.then(|| req.topics.get(r.topic).cloned().unwrap_or_default());
        cut::days(rows.iter().copied(), &tag)
    }

    /// What a call about `rows` reads: the messages themselves when they come
    /// to `single` tokens or fewer and fit beside `direct` (the prompt that
    /// would read them) and `reserve` tokens of answer, else notes on each
    /// part — cached, so a summary's notes are a conversation's too — and the
    /// days they were taken from.
    #[allow(clippy::too_many_arguments)]
    fn gather(
        &self,
        about: &prompt::About,
        topic: Option<&str>,
        rows: &[&Row],
        direct: &str,
        reserve: usize,
        single: usize,
        summary: &mut Summary,
        on: &mut dyn FnMut(Event),
    ) -> Result<Gathered, Error> {
        let req = self.request;
        let days = self.days(topic, rows);
        let counts = cut::measure(&days, &mut |t| self.tokens(t))?;
        let total: usize = counts.iter().sum();

        let single = self.budget(direct, reserve)?.min(single);
        if total <= single {
            on(Event::Say(format!(
                "{} messages, {total} tokens: one call",
                rows.len()
            )));
            let text: Vec<String> = days.iter().map(cut::Day::text).collect();
            return Ok(Gathered::Messages(text.join("\n")));
        }

        let system = prompt::notes(about, req.lang);
        let budget = self.budget(&system, NOTES_ANSWER)?.min(PART_TOKENS);
        let parts = cut::cut(&days, &counts, budget);
        on(Event::Say(format!(
            "{} messages, {total} tokens: {} parts of up to {budget}",
            rows.len(),
            parts.len()
        )));
        summary.parts += parts.len();

        let mut notes: Vec<Note> = Vec::new();
        let mut refused: Option<String> = None;
        for (i, part) in parts.iter().enumerate() {
            let key = cache::key(&[
                &prompt::VERSION.to_string(),
                &self.props.model,
                req.lang.code(),
                req.chat,
                topic.unwrap_or(""),
                "notes",
                &system,
                &part.text,
            ]);
            let started = Instant::now();
            if let Some(kept) = self.cache.get(&key) {
                notes.extend(kept);
                on(Event::Part {
                    done: i + 1,
                    of: parts.len(),
                    cached: true,
                    secs: 0.0,
                    timings: None,
                });
                continue;
            }
            let part_ids: HashSet<i64> = part.ids.iter().copied().collect();
            let user = format!("Messages:\n\n{}", part.text);
            match self.notes_call(&system, &user, &part_ids)? {
                Answer::Got(got, timings) => {
                    // The merge's rule, for the same reason: an empty answer
                    // kept is a part left out of every summary after this one.
                    if got.is_empty() && part.ids.len() > NOTHING_TO_NOTE {
                        on(Event::Say(format!(
                            "part {} of {}: no notes on {} messages; not kept, so a re-run asks again",
                            i + 1,
                            parts.len(),
                            part.ids.len()
                        )));
                    } else {
                        self.cache.put(&key, &got);
                    }
                    notes.extend(got);
                    on(Event::Part {
                        done: i + 1,
                        of: parts.len(),
                        cached: false,
                        secs: started.elapsed().as_secs_f64(),
                        timings: Some(timings),
                    });
                }
                Answer::Unreadable(why) => {
                    summary.failed += 1;
                    on(Event::Say(match &why {
                        Some(said) => format!(
                            "part {} of {}: the model server refused ({said}), left out",
                            i + 1,
                            parts.len()
                        ),
                        None => format!(
                            "part {} of {}: no readable answer after {ATTEMPTS} tries, left out",
                            i + 1,
                            parts.len()
                        ),
                    }));
                    refused = why.or(refused);
                }
            }
        }
        if notes.is_empty() {
            return Err(Error::Bad(match refused {
                Some(said) => {
                    format!("the model wrote no notes for any part; the server last said: {said}")
                }
                None => "the model wrote no notes for any part".into(),
            }));
        }
        Ok(Gathered::Notes(notes, days))
    }

    /// Notes cut down until they fit `room`: merged by the model first, then,
    /// if that does not get there, the least important dropped.
    fn fit(
        &self,
        about: &prompt::About,
        mut notes: Vec<Note>,
        room: usize,
        on: &mut dyn FnMut(Event),
    ) -> Result<Vec<Note>, Error> {
        notes.sort_by(|a, b| a.day.cmp(&b.day));
        let system = prompt::merge(about, self.request.lang);
        let budget = self.budget(&system, NOTES_ANSWER)?.min(PART_TOKENS);
        for round in 1..=MERGE_ROUNDS {
            let size = self.tokens(&lines_of(&notes))?;
            if size <= room {
                return Ok(notes);
            }
            on(Event::Say(format!(
                "{} notes, {size} tokens: merging to fit {room}",
                notes.len()
            )));
            // Groups by estimated share of the counted size, in day order.
            let per = size as f64 / notes.len() as f64;
            let take = ((budget as f64 / per) as usize).max(2);
            let known: HashSet<i64> = notes.iter().flat_map(|n| n.ids.iter().copied()).collect();
            let groups: Vec<&[Note]> = notes.chunks(take).collect();
            let mut merged = Vec::new();
            // Each group is a call of a minute or so, and a round can be
            // thirty of them: say where it is and what is left, every call.
            let started = Instant::now();
            let mut asked = 0;
            for (i, group) in groups.iter().enumerate() {
                on(Event::Say(merging(
                    notes.len(),
                    round,
                    i,
                    groups.len(),
                    asked,
                    started.elapsed(),
                )));
                let user = format!("Notes:\n\n{}", lines_of(group));
                // Kept like a part's notes, so asking about the same days
                // again, or a re-run, does not merge them all over again.
                let key = cache::key(&[
                    &prompt::VERSION.to_string(),
                    &self.props.model,
                    self.request.lang.code(),
                    self.request.chat,
                    "merge",
                    &system,
                    &user,
                ]);
                if let Some(kept) = self.cache.get(&key) {
                    merged.extend(kept);
                    continue;
                }
                asked += 1;
                match self.notes_call(&system, &user, &known)? {
                    Answer::Got(got, _) if !got.is_empty() => {
                        self.cache.put(&key, &got);
                        merged.extend(got);
                    }
                    _ => merged.extend(group.iter().cloned()),
                }
            }
            if merged.len() >= notes.len() {
                notes = merged;
                break;
            }
            notes = merged;
            notes.sort_by(|a, b| a.day.cmp(&b.day));
        }
        // Still too big: keep the most important, in day order.
        let mut ranked: Vec<(usize, Note)> = notes.into_iter().enumerate().collect();
        ranked.sort_by_key(|(i, n)| (n.rank(), *i));
        let mut kept: Vec<(usize, Note)> = Vec::new();
        let mut size = 0;
        for (i, n) in ranked {
            let cost = self.tokens(&n.line())? + 1;
            if size + cost > room {
                break;
            }
            size += cost;
            kept.push((i, n));
        }
        kept.sort_by_key(|(i, _)| *i);
        on(Event::Say(format!(
            "kept the {} most important notes",
            kept.len()
        )));
        Ok(kept.into_iter().map(|(_, n)| n).collect())
    }

    /// Notes for one input, their ids checked against `known`.
    fn notes_call(
        &self,
        system: &str,
        user: &str,
        known: &HashSet<i64>,
    ) -> Result<Answer<Vec<Note>>, Error> {
        let mut body = self.body(system, user, NOTES_ANSWER);
        body["response_format"] = json!({
            "type": "json_schema",
            "json_schema": { "name": "notes", "strict": true, "schema": prompt::notes_schema() }
        });
        Ok(match json_answer(self.client, &body, self.cancel)? {
            Answer::Got(v, timings) => {
                let mut notes = Note::all_from(&v);
                for n in &mut notes {
                    n.ids.retain(|id| known.contains(id));
                }
                Answer::Got(notes, timings)
            }
            Answer::Unreadable(why) => Answer::Unreadable(why),
        })
    }

    /// The overview above the topics' summaries, and whether it was cut.
    fn overview(
        &self,
        sections: &[Section],
        on: &mut dyn FnMut(Event),
    ) -> Result<(String, bool), Error> {
        let from = self.request.from.to_string();
        let to = self.request.to.to_string();
        let about = self.about(None, &from, &to);
        let system = prompt::overview(&about, self.request.lang);
        let input: Vec<String> = sections
            .iter()
            .map(|s| {
                format!(
                    "## {}\n\n{}",
                    s.heading.as_deref().unwrap_or_default(),
                    s.text
                )
            })
            .collect();
        self.write(
            &system,
            &input.join("\n\n"),
            OVERVIEW_ANSWER,
            &HashSet::new(),
            false,
            on,
        )
    }

    /// Every document read in the range, explained on its own — a gist,
    /// then all of it — under one heading, and those whose text could not be
    /// read, named. `None` when the range shared no document.
    fn documents(
        &self,
        summary: &mut Summary,
        on: &mut dyn FnMut(Event),
    ) -> Result<Option<Section>, Error> {
        let lang = self.request.lang;
        let mut read: Vec<(&Row, &tga_notes::Attachment)> = Vec::new();
        let mut unread: Vec<&str> = Vec::new();
        // The same file posted twice is one document, explained once.
        let mut seen: HashSet<&str> = HashSet::new();
        for row in self.request.rows {
            let Some(doc) = &row.doc else { continue };
            if doc.text.trim().is_empty() {
                if !unread.contains(&doc.name.as_str()) {
                    unread.push(&doc.name);
                }
            } else if seen.insert(doc.text.trim()) {
                read.push((row, doc));
            }
        }
        if read.is_empty() && unread.is_empty() {
            return Ok(None);
        }
        let heading = documents_heading(lang);
        on(Event::Section(Some(heading.to_string())));
        let mut text = String::new();
        for (i, (row, doc)) in read.iter().enumerate() {
            on(Event::Say(format!(
                "Explaining document {} of {}",
                i + 1,
                read.len()
            )));
            if i > 0 {
                text.push_str("\n\n");
                on(Event::Text("\n\n".into()));
            }
            let (one, cut) = self.document(row, doc, on)?;
            summary.cut |= cut;
            text.push_str(&one);
        }
        if !unread.is_empty() {
            // Not downloaded, or a scanned PDF with no text in it.
            let label = match lang {
                Lang::Serbian => "Tekst nije pročitan",
                _ => "Text not read",
            };
            let mut line = format!("*{label}:* {}.", unread.join(", "));
            if !text.is_empty() {
                line.insert_str(0, "\n\n");
            }
            on(Event::Text(line.clone()));
            text.push_str(&line);
        }
        Ok(Some(Section {
            heading: Some(heading.to_string()),
            text,
        }))
    }

    /// One document's heading and explanation, streamed, and whether the
    /// answer's limit cut it. A refusal leaves this document unexplained,
    /// not the summary above it unwritten.
    fn document(
        &self,
        row: &Row,
        doc: &tga_notes::Attachment,
        on: &mut dyn FnMut(Event),
    ) -> Result<(String, bool), Error> {
        let req = self.request;
        let from = req.from.to_string();
        let to = req.to.to_string();
        let topic = req
            .forum
            .then(|| req.topics.get(row.topic))
            .flatten()
            .map(String::as_str);
        let about = self.about(topic, &from, &to);
        let system = prompt::document(&about, req.lang);
        let room = self.budget(&system, DOCUMENT_ANSWER)?;

        // Too long to read whole beside the answer: its beginning.
        let whole = doc.text.trim().chars().count();
        let mut keep = whole;
        let (mut user, mut early) = document_input(row, doc, None);
        for _ in 0..3 {
            let size = self.tokens(&user)?;
            if size <= room {
                break;
            }
            keep = keep * room / size * 9 / 10;
            (user, early) = document_input(row, doc, Some(keep));
        }
        if keep < whole {
            on(Event::Say(format!(
                "a document does not fit: read its first {keep} of {whole} characters"
            )));
        }

        let who = if row.who.is_empty() { "?" } else { &row.who };
        let mut by = format!("{who}, {}", row.t.get(..10).unwrap_or(&row.t));
        if let Some(t) = topic {
            by.push_str(&format!(", {t}"));
        }
        if early {
            by.push_str(match req.lang {
                Lang::Serbian => " — pročitan je samo početak",
                _ => " — only its beginning was read",
            });
        }
        let head = format!("### {}\n\n*{by}* [#{}]\n\n", doc.name, row.id);
        on(Event::Text(head.clone()));

        let key = cache::key(&[
            &prompt::VERSION.to_string(),
            &self.props.model,
            req.lang.code(),
            req.chat,
            "document",
            &system,
            &user,
        ]);
        if let Some(kept) = self.cache.text(&key) {
            on(Event::Text(kept.clone()));
            return Ok((format!("{head}{kept}"), false));
        }
        match self.write(&system, &user, DOCUMENT_ANSWER, &HashSet::new(), false, on) {
            Ok((text, cut)) => {
                // A cut answer is not kept: a re-run would lose that it was.
                if !cut && !text.is_empty() {
                    self.cache.put_text(&key, &text);
                }
                Ok((format!("{head}{text}"), cut))
            }
            Err(Error::Server(said)) => {
                on(Event::Say(format!(
                    "the model server refused a document ({}), left unexplained",
                    shorten(&said, REFUSAL_CHARS)
                )));
                let line = match req.lang {
                    Lang::Serbian => "*Model nije objasnio ovaj dokument.*",
                    _ => "*The model did not explain this document.*",
                };
                on(Event::Text(line.into()));
                Ok((format!("{head}{line}"), false))
            }
            Err(e) => Err(e),
        }
    }

    /// A streamed call whose text is the summary, and whether the length
    /// limit cut it.
    fn write(
        &self,
        system: &str,
        user: &str,
        answer: usize,
        known: &HashSet<i64>,
        cites: bool,
        on: &mut dyn FnMut(Event),
    ) -> Result<(String, bool), Error> {
        let body = self.body(system, user, answer);
        let none = HashSet::new();
        stream_cited(
            self.client,
            &body,
            if cites { known } else { &none },
            self.cancel,
            on,
        )
    }

    /// A request body, with Gemma's own sampling and its thinking off.
    fn body(&self, system: &str, user: &str, max_tokens: usize) -> Value {
        body(
            json!([
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ]),
            max_tokens,
        )
    }
}

/// A request body for `messages`, with Gemma's own sampling and its thinking
/// off.
fn body(messages: Value, max_tokens: usize) -> Value {
    json!({
        "messages": messages,
        "temperature": 1.0,
        "top_k": 64,
        "top_p": 0.95,
        "max_tokens": max_tokens,
        "chat_template_kwargs": { "enable_thinking": false }
    })
}

/// Stream `body`'s answer through the citation check: `on` gets each piece
/// as [`Event::Text`], then [`Event::Written`]. The whole text, trimmed, and
/// whether it stopped at its length limit — said as it happens, since the
/// text then ends wherever the limit fell.
fn stream_cited(
    client: &Client,
    body: &Value,
    known: &HashSet<i64>,
    cancel: &AtomicBool,
    on: &mut dyn FnMut(Event),
) -> Result<(String, bool), Error> {
    let mut filter = cite::Filter::new(known);
    let mut text = String::new();
    let mut started = false;
    let streamed = client.stream(body, cancel, &mut |piece| {
        let mut shown = filter.push(piece);
        if !started {
            // The model's habit of opening on a blank line.
            shown = shown.trim_start().to_string();
            started = !shown.is_empty();
        }
        if !shown.is_empty() {
            text.push_str(&shown);
            on(Event::Text(shown));
        }
    })?;
    let rest = filter.finish();
    if !rest.is_empty() {
        text.push_str(&rest);
        on(Event::Text(rest));
    }
    on(Event::Written(streamed.timings));
    if streamed.cut {
        on(Event::Say(format!(
            "the answer reached its limit of {} tokens and stops there",
            body["max_tokens"].as_u64().unwrap_or_default()
        )));
    }
    Ok((text.trim().to_string(), streamed.cut))
}

/// What a call asked for JSON came back with.
enum Answer<T> {
    Got(T, Timings),
    /// No try gave readable JSON. The server's last refusal, if that is why:
    /// its own words about the request — "exceeds the context", a schema it
    /// cannot take — never anything the model wrote.
    Unreadable(Option<String>),
}

/// `body`'s answer as JSON, asked up to [`ATTEMPTS`] times: again at once
/// when the answer is not JSON; again after a pause when the server failed
/// (a 5xx), [`BACKOFF`] and doubling; and not again when it refused the
/// request (a 4xx), which the same request would get the same way.
fn json_answer(client: &Client, body: &Value, cancel: &AtomicBool) -> Result<Answer<Value>, Error> {
    let mut refused: Option<String> = None;
    let mut pause = BACKOFF;
    for attempt in 0..ATTEMPTS {
        let (text, timings) = match client.complete(body, cancel) {
            Ok(answer) => {
                // Answered: if this try fails, it is the model's doing.
                refused = None;
                answer
            }
            Err(Error::Server(said)) => {
                // `client` puts the status first: "400: …".
                let for_good = said.starts_with('4');
                // Its reason, not a page of it: the line goes to the log.
                refused = Some(shorten(&said, REFUSAL_CHARS));
                if for_good || attempt + 1 == ATTEMPTS {
                    break;
                }
                wait(pause, cancel)?;
                pause *= 2;
                continue;
            }
            Err(e) => return Err(e),
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        return Ok(Answer::Got(v, timings));
    }
    Ok(Answer::Unreadable(refused))
}

/// Sleep `pause`, or less if the run is stopped.
fn wait(pause: Duration, cancel: &AtomicBool) -> Result<(), Error> {
    let until = Instant::now() + pause;
    while Instant::now() < until {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        std::thread::sleep(
            Duration::from_millis(100).min(until.saturating_duration_since(Instant::now())),
        );
    }
    Ok(())
}

/// `Merging 1,332 notes, round 1 of up to 3: group 8 of 30, about 9 min
/// left` — the time from the calls asked so far; none until one has been.
fn merging(
    notes: usize,
    round: usize,
    at: usize,
    groups: usize,
    asked: usize,
    spent: std::time::Duration,
) -> String {
    let left = if asked == 0 {
        String::new()
    } else {
        let secs = spent.as_secs_f64() / asked as f64 * (groups - at) as f64;
        if secs < 90.0 {
            ", about a minute left".to_string()
        } else {
            format!(", about {} min left", (secs / 60.0).round() as u64)
        }
    };
    format!(
        "Merging {} notes, round {round} of up to {MERGE_ROUNDS}: group {} of {groups}{left}",
        thousands(notes),
        at + 1
    )
}

/// `1332` as `1,332`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What a document's call reads: where the document came from, then its
/// text — only the first `keep` characters, when given — and whether that
/// text stops before the document does.
fn document_input(row: &Row, doc: &tga_notes::Attachment, keep: Option<usize>) -> (String, bool) {
    let mut s = format!("Document \"{}\"", doc.name);
    if !doc.date.is_empty() {
        s.push_str(&format!(", dated {} ({})", doc.date, doc.date_src));
    }
    let who = if row.who.is_empty() { "?" } else { &row.who };
    s.push_str(&format!("\nPosted by {who} on {}", row.t));
    let said = lines::flat(&row.text, lines::TEXT_CHARS);
    if !said.is_empty() {
        s.push_str(&format!(", with the message: {said}"));
    }
    let text = doc.text.trim();
    let (text, early) = match keep {
        Some(n) => (text.chars().take(n).collect::<String>(), true),
        None => (text.to_string(), doc.cut),
    };
    if early {
        s.push_str("\nThe text stops early: only the document's beginning was read.");
    }
    s.push_str(&format!("\n\nText:\n\n{text}"));
    (s, early)
}

fn lines_of(notes: &[Note]) -> String {
    notes.iter().map(Note::line).collect::<Vec<_>>().join("\n")
}

/// What one group of rows came to: the messages, or notes on them and the
/// days they were taken from — which a conversation keeps, to read the
/// messages behind a note, and which are not cheap to render twice.
enum Gathered {
    Messages(String),
    Notes(Vec<Note>, Vec<cut::Day>),
}

// -- a conversation about a range ---------------------------------------------

/// The longest answer to one question.
const CHAT_ANSWER: usize = 1_500;

/// Earlier questions and answers kept in the conversation, by characters
/// (about 4,000 tokens); the oldest go first.
const HISTORY_CHARS: usize = 12_000;

/// Over notes, a question's answer reads the messages around those it turns
/// on: this many either side of each, then fewer until they fit.
const AROUND: [usize; 3] = [3, 1, 0];

/// The newest turns the pick reads, and how much of each answer: enough to
/// resolve "and what did she say about it?", no more.
const PICK_TURNS: usize = 3;
const PICK_ANSWER_CHARS: usize = 1_200;

/// The pick's answer: a dozen numbers.
const PICK_ANSWER: usize = 200;

/// What a conversation about a range rests on: the range's messages, or —
/// when they do not fit — the notes a summary takes of them, and the lines
/// themselves, so a question can read the messages its notes rest on. Built
/// once and kept for every question about the same days.
#[derive(Debug, Clone, PartialEq)]
pub struct Grounding {
    /// The lines, or the notes, as the model reads them.
    pub text: String,
    pub notes: bool,
    /// Every message id the text may cite.
    pub known: HashSet<i64>,
    /// The forum topic, when the range is one topic's.
    pub topic: Option<String>,
    pub messages: usize,
    /// When `notes`: the notes `text` lists, in its order.
    pub listed: Vec<Note>,
    /// When `notes`: every day of the range, rendered.
    pub days: Vec<cut::Day>,
}

/// One question and its answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Turn {
    pub question: String,
    pub answer: String,
}

/// Read a range for a conversation: the messages when they fit beside the
/// questions, else notes on them — the summary's own, from the cache, when
/// a summary of these days was written. Topics are not split: a question may
/// be about any of them.
pub fn ground(
    client: &Client,
    request: &Request,
    cache: &Cache,
    cancel: &AtomicBool,
    on: &mut dyn FnMut(Event),
) -> Result<Grounding, Error> {
    let props = client.props()?;
    if props.n_ctx == 0 {
        return Err(Error::Bad(
            "the model server did not say how big its context is".into(),
        ));
    }
    let whole = Request {
        by_topic: false,
        ..request.clone()
    };
    let run = Run {
        client,
        props,
        request: &whole,
        cache,
        cancel,
    };
    let from = whole.from.to_string();
    let to = whole.to.to_string();
    let groups = groups(&whole);
    let Some((topic, rows)) = groups.busy.into_iter().next() else {
        return Err(Error::Bad("nothing was said in these days".into()));
    };
    let about = run.about(topic.as_deref(), &from, &to);
    let known: HashSet<i64> = rows.iter().map(|r| r.id).collect();
    let reserve = CHAT_ANSWER + HISTORY_CHARS / 3;
    let mut summary = Summary::default();
    let direct = prompt::chat(&about, prompt::Basis::Messages);
    // A question reads up to a whole call's worth of messages: it looks for
    // one thing, where a summary has to weigh everything.
    let gathered = run.gather(
        &about,
        topic.as_deref(),
        &rows,
        &direct,
        reserve,
        INPUT_TOKENS,
        &mut summary,
        on,
    )?;
    let mut grounding = Grounding {
        text: String::new(),
        notes: false,
        known,
        topic: None,
        messages: rows.len(),
        listed: Vec::new(),
        days: Vec::new(),
    };
    match gathered {
        Gathered::Messages(text) => grounding.text = text,
        Gathered::Notes(notes, days) => {
            let room = run
                .budget(&prompt::chat(&about, prompt::Basis::Notes), reserve)?
                .min(INPUT_TOKENS);
            let listed = run.fit(&about, notes, room, on)?;
            grounding.text = lines_of(&listed);
            grounding.notes = true;
            grounding.listed = listed;
            grounding.days = days;
        }
    }
    grounding.topic = topic;
    Ok(grounding)
}

/// Answer `question` about the range `grounding` was read from, with the
/// conversation so far, streamed as [`Event::Text`] with its citations
/// checked. The answer's whole text.
///
/// Over notes, the model first picks the notes the question is about, and
/// the answer reads the messages they rest on: a note can be wrong about
/// who held which position, and a question is usually about exactly that.
/// Only when nothing is picked does the answer read the notes alone.
///
/// Of `request`, only the chat's name and the days are read: what the days
/// say is in `grounding`, so a caller need not build the rows again.
pub fn answer(
    client: &Client,
    request: &Request,
    grounding: &Grounding,
    history: &[Turn],
    question: &str,
    cancel: &AtomicBool,
    on: &mut dyn FnMut(Event),
) -> Result<String, Error> {
    let from = request.from.to_string();
    let to = request.to.to_string();
    let about = prompt::About {
        chat: request.chat,
        topic: grounding.topic.as_deref(),
        from: &from,
        to: &to,
    };
    // The newest turns that fit, oldest first.
    let mut kept: Vec<&Turn> = Vec::new();
    let mut size = 0;
    for turn in history.iter().rev() {
        size += turn.question.len() + turn.answer.len();
        if size > HISTORY_CHARS {
            break;
        }
        kept.push(turn);
    }
    kept.reverse();

    let found = if grounding.notes {
        sources(client, &about, grounding, &kept, question, cancel, on)?
    } else {
        None
    };
    let (basis, input, known) = match (&found, grounding.notes) {
        (Some(s), _) => (prompt::Basis::Sources, s.text.clone(), &s.known),
        (None, true) => (
            prompt::Basis::Notes,
            format!("Notes:\n\n{}", grounding.text),
            &grounding.known,
        ),
        (None, false) => (
            prompt::Basis::Messages,
            format!("Messages:\n\n{}", grounding.text),
            &grounding.known,
        ),
    };
    let system = format!("{}\n\n{input}", prompt::chat(&about, basis));
    let mut messages = vec![json!({ "role": "system", "content": system })];
    for turn in &kept {
        messages.push(json!({ "role": "user", "content": turn.question }));
        messages.push(json!({ "role": "assistant", "content": turn.answer }));
    }
    messages.push(json!({ "role": "user", "content": question }));
    // An answer cut at its limit is said by `stream_cited` as it happens; the
    // conversation goes on, and the next question can ask for the rest.
    let (text, _) = stream_cited(
        client,
        &body(Value::Array(messages), CHAT_ANSWER),
        known,
        cancel,
        on,
    )?;
    Ok(text)
}

/// What a question over notes reads: the notes it turns on, and the
/// messages around what they rest on.
struct Sources {
    text: String,
    /// The ids of the messages shown, the only ones the answer may cite.
    known: HashSet<i64>,
}

/// The notes `question` turns on, picked by the model, and the messages
/// around what each rests on, fitted beside the conversation: fewer
/// neighbours first, then the least relevant notes. `None` when the model
/// picked nothing or gave no readable answer, and the notes must do.
fn sources(
    client: &Client,
    about: &prompt::About,
    grounding: &Grounding,
    history: &[&Turn],
    question: &str,
    cancel: &AtomicBool,
    on: &mut dyn FnMut(Event),
) -> Result<Option<Sources>, Error> {
    if grounding.listed.is_empty() || grounding.days.is_empty() {
        return Ok(None);
    }
    on(Event::Say(format!(
        "Choosing among {} notes",
        grounding.listed.len()
    )));
    let (places, refused) = match pick(client, about, &grounding.listed, history, question, cancel)?
    {
        Answer::Got(places, _) => (places, None),
        Answer::Unreadable(why) => (Vec::new(), why),
    };
    let picked: Vec<&Note> = places
        .into_iter()
        .map(|i| &grounding.listed[i])
        .filter(|n| !n.ids.is_empty())
        .collect();
    if picked.is_empty() {
        on(Event::Say(match refused {
            Some(said) => format!(
                "The model server refused the choice ({said}): answering from all the notes"
            ),
            None => "No note picked: answering from all the notes".into(),
        }));
        return Ok(None);
    }
    on(Event::Say(format!(
        "Looking up the messages behind {} notes",
        picked.len()
    )));

    // The room left after the prompt, the conversation and the answer.
    let n_ctx = client.props()?.n_ctx;
    let mut spoken = String::new();
    for turn in history {
        spoken.push_str(&turn.question);
        spoken.push('\n');
        spoken.push_str(&turn.answer);
        spoken.push('\n');
    }
    spoken.push_str(question);
    let used = client.tokens(&prompt::chat(about, prompt::Basis::Sources), cancel)?
        + client.tokens(&spoken, cancel)?
        + CHAT_ANSWER
        + MARGIN;
    let room = n_ctx.saturating_sub(used).min(INPUT_TOKENS);

    let index = Index::of(&grounding.days);
    for count in (1..=picked.len()).rev() {
        for around in AROUND {
            let found = index.read(&picked[..count], around);
            let size = client.tokens(&found.text, cancel)?;
            if size <= room {
                on(Event::Say(format!(
                    "{} messages around {count} notes, {size} tokens",
                    found.known.len()
                )));
                return Ok(Some(found));
            }
        }
    }
    on(Event::Say(
        "The messages behind the notes do not fit: answering from all the notes".into(),
    ));
    Ok(None)
}

/// The places in `notes` of those `question` turns on, most relevant first;
/// empty when the model chose none.
fn pick(
    client: &Client,
    about: &prompt::About,
    notes: &[Note],
    history: &[&Turn],
    question: &str,
    cancel: &AtomicBool,
) -> Result<Answer<Vec<usize>>, Error> {
    let listed: Vec<String> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| format!("{}. {} [{}] {}", i + 1, n.day, n.weight, n.what))
        .collect();
    let mut user = format!("Notes:\n\n{}\n\n", listed.join("\n"));
    for turn in &history[history.len().saturating_sub(PICK_TURNS)..] {
        user.push_str(&format!(
            "Question: {}\nAnswer: {}\n\n",
            turn.question,
            shorten(&turn.answer, PICK_ANSWER_CHARS)
        ));
    }
    user.push_str(&format!("Newest question: {question}"));
    let mut body = body(
        json!([
            { "role": "system", "content": prompt::pick(about) },
            { "role": "user", "content": user }
        ]),
        PICK_ANSWER,
    );
    body["response_format"] = json!({
        "type": "json_schema",
        "json_schema": { "name": "picked", "strict": true, "schema": prompt::pick_schema() }
    });
    Ok(match json_answer(client, &body, cancel)? {
        Answer::Got(v, timings) => {
            let mut out: Vec<usize> = Vec::new();
            for n in v["notes"].as_array().into_iter().flatten() {
                let Some(i) = n.as_u64().map(|i| i as usize) else {
                    continue;
                };
                if (1..=notes.len()).contains(&i) && !out.contains(&(i - 1)) {
                    out.push(i - 1);
                }
            }
            out.truncate(prompt::PICKED);
            Answer::Got(out, timings)
        }
        Answer::Unreadable(why) => Answer::Unreadable(why),
    })
}

/// The first `chars` characters of `text`, and `…` if there were more.
fn shorten(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// Every line of a range in order, and where each message is in it.
struct Index<'g> {
    lines: Vec<(NaiveDate, &'g cut::Line)>,
    at: HashMap<i64, usize>,
}

impl<'g> Index<'g> {
    fn of(days: &'g [cut::Day]) -> Self {
        let lines: Vec<(NaiveDate, &cut::Line)> = days
            .iter()
            .flat_map(|d| d.lines.iter().map(move |l| (d.date, l)))
            .collect();
        let at = lines
            .iter()
            .enumerate()
            .map(|(i, (_, l))| (l.id, i))
            .collect();
        Index { lines, at }
    }

    /// `notes`, then the messages they rest on with `around` more either
    /// side of each, and the message each of those answers, by day; a `…`
    /// where a day's messages are left out before one that is shown.
    fn read(&self, notes: &[&Note], around: usize) -> Sources {
        let last = self.lines.len().saturating_sub(1);
        let mut chosen: BTreeSet<usize> = BTreeSet::new();
        for id in notes.iter().flat_map(|n| &n.ids) {
            if let Some(&i) = self.at.get(id) {
                chosen.extend(i.saturating_sub(around)..=(i + around).min(last));
            }
        }
        let answered: Vec<usize> = chosen
            .iter()
            .filter_map(|&j| self.lines[j].1.re)
            .filter_map(|re| self.at.get(&re).copied())
            .collect();
        chosen.extend(answered);

        let listed: Vec<String> = notes.iter().map(|n| n.line()).collect();
        let mut text = format!(
            "Notes, for orientation:\n\n{}\n\nMessages:\n",
            listed.join("\n")
        );
        let mut known = HashSet::new();
        let mut day = None;
        for &j in &chosen {
            let (date, line) = self.lines[j];
            if day != Some(date) {
                text.push('\n');
                text.push_str(&lines::day_line(date));
                day = Some(date);
            }
            if j > 0 && !chosen.contains(&(j - 1)) && self.lines[j - 1].0 == date {
                text.push_str("\n…");
            }
            text.push('\n');
            text.push_str(&line.text);
            known.insert(line.id);
        }
        Sources { text, known }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_says_where_it_is_and_what_is_left() {
        use std::time::Duration;
        assert_eq!(
            merging(1332, 1, 0, 30, 0, Duration::ZERO),
            "Merging 1,332 notes, round 1 of up to 3: group 1 of 30"
        );
        // Seven calls at a minute each, 23 groups still to go.
        assert_eq!(
            merging(1332, 1, 7, 30, 7, Duration::from_secs(7 * 60)),
            "Merging 1,332 notes, round 1 of up to 3: group 8 of 30, about 23 min left"
        );
        assert_eq!(
            merging(400, 2, 8, 9, 8, Duration::from_secs(8 * 40)),
            "Merging 400 notes, round 2 of up to 3: group 9 of 9, about a minute left"
        );
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    fn row(id: i64, topic: usize, service: bool) -> Row {
        Row {
            id,
            t: "2024-12-02 10:00".into(),
            topic,
            service: service.then(|| "pin_message".into()),
            ..Default::default()
        }
    }

    fn request<'a>(rows: &'a [Row], topics: &'a [String], by_topic: bool) -> Request<'a> {
        Request {
            chat: "Group",
            rows,
            topics,
            forum: true,
            by_topic,
            length: Length::Detailed,
            lang: Lang::English,
            from: NaiveDate::from_ymd_opt(2024, 12, 2).unwrap(),
            to: NaiveDate::from_ymd_opt(2024, 12, 2).unwrap(),
        }
    }

    #[test]
    fn by_topic_puts_the_busiest_first_and_names_the_quiet_ones() {
        let topics = vec![
            "General".to_string(),
            "Media".to_string(),
            "Legal".to_string(),
        ];
        let mut rows = Vec::new();
        rows.extend((0..5).map(|i| row(i, 0, false)));
        rows.extend((10..18).map(|i| row(i, 1, false)));
        rows.extend((20..22).map(|i| row(i, 2, false)));
        rows.extend((30..40).map(|i| row(i, 2, true))); // service only: not talk
        let g = groups(&request(&rows, &topics, true));
        let names: Vec<_> = g.busy.iter().map(|(n, _)| n.clone().unwrap()).collect();
        assert_eq!(names, vec!["Media", "General"]);
        assert_eq!(g.quiet, vec![("Legal".to_string(), 2)]);
    }

    #[test]
    fn one_topic_of_a_forum_names_it_and_a_mix_names_none() {
        let topics = vec!["General".to_string(), "Media".to_string()];
        let one: Vec<Row> = (0..3).map(|i| row(i, 1, false)).collect();
        assert_eq!(
            groups(&request(&one, &topics, false)).busy[0].0.as_deref(),
            Some("Media")
        );
        let mix = vec![row(1, 0, false), row(2, 1, false)];
        assert_eq!(groups(&request(&mix, &topics, false)).busy[0].0, None);
    }

    #[test]
    fn a_body_puts_the_overview_first_and_demotes_section_headings() {
        let s = Summary {
            sections: vec![
                Section {
                    heading: None,
                    text: "Overall.".into(),
                },
                Section {
                    heading: Some("Media".into()),
                    text: "## Decisions\n- x [#1]".into(),
                },
            ],
            quiet: vec![("Legal".into(), 2)],
            lang: Lang::English,
            ..Default::default()
        };
        assert_eq!(
            s.body(),
            "Overall.\n\n## Media\n\n### Decisions\n- x [#1]\n\n*Other topics:* Legal (2)."
        );
        assert_eq!(s.cited(), vec![1]);
    }

    #[test]
    fn a_note_reads_back_as_one_line() {
        let n = Note::from_json(
            &json!({"day": "2024-12-02", "weight": "major", "what": " Strike. ", "ids": [1, 2]}),
        )
        .unwrap();
        assert_eq!(n.line(), "- 2024-12-02 [major] Strike. [#1, #2]");
        assert!(Note::from_json(&json!({"what": ""})).is_none());
    }
}
