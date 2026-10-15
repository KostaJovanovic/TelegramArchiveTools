//! What the model is told.
//!
//! Three calls, three prompts: **notes** on one part (structured, so ids can be
//! checked and parts merged), the **final** text from either the messages or
//! the notes (streamed prose), and an **overview** above a by-topic summary.
//! A fourth, **merge**, condenses notes when there are too many for the final
//! call — the notes prompt pointed at notes rather than messages. A question
//! has **chat**, and over notes a **pick** first, which chooses the notes whose
//! messages the answer should read.
//!
//! [`VERSION`] is part of every cache key: change a prompt and bump it, or a
//! cached part answers with what the old prompt asked for.

use serde_json::{json, Value};

use crate::{Lang, Length};

pub const VERSION: u32 = 4;

/// What the lines and the notes look like, said once for every prompt.
const READING: &str = "Each line is one message: `#id HH:MM name (re #id): text`. \
A line `== YYYY-MM-DD Day ==` starts a day. `[photo]`, `[video]` and the like mark \
attachments; `[document \"name\", dated …: text]` is a file with the text inside it; \
`[photo with text: …]` is a photo — a poster, a screenshot, a page — and the text a \
machine read off it, which may have misread a letter or two; \
`(N reactions)` says how much the room reacted. A name in square brackets after the \
time is the forum topic the message was posted in.";

/// Where the summary is from, for every prompt's first sentence.
pub struct About<'a> {
    pub chat: &'a str,
    /// A forum topic's name when the summary is of one topic.
    pub topic: Option<&'a str>,
    pub from: &'a str,
    pub to: &'a str,
}

impl About<'_> {
    fn of(&self) -> String {
        match self.topic {
            Some(t) => format!("the topic \"{t}\" of the Telegram group \"{}\"", self.chat),
            None => format!("the Telegram group \"{}\"", self.chat),
        }
    }

    fn span(&self) -> String {
        if self.from == self.to {
            format!("on {}", self.from)
        } else {
            format!("from {} to {}", self.from, self.to)
        }
    }
}

pub fn language(lang: Lang) -> &'static str {
    match lang {
        Lang::Serbian => {
            "Write in Serbian, Latin script, ekavian (\"dete\", \"mleko\", \"uvek\" — never \
             \"dijete\", \"mlijeko\", \"uvijek\"), whatever script or dialect the messages use."
        }
        Lang::English => "Write in English, whatever language the messages are in.",
        Lang::AsChat => {
            "Write in the language most of the messages are written in. If that is Serbian, \
             use Latin script, ekavian."
        }
    }
}

/// Notes on one part.
pub fn notes(about: &About, lang: Lang) -> String {
    format!(
        "You read one stretch of {of}. {READING}\n\n\
         List what happened, as notes for a summary written later. Be thorough and \
         faithful: report every real event, and invent nothing. What counts: decisions, \
         plans and deadlines, events and actions, meetings, announcements, documents \
         shared, discussions and disagreements, problems, questions left open, who took on \
         what.\n\
         - Merge many messages about one thing into ONE note.\n\
         - A discussion, dispute or argument is ONE note from its start to its end: what \
         it was about, who held which position, and how it ended — or that it did not.\n\
         - Name who held each position, person by person, one position per sentence: \
         \"X wanted A; Y and Z were against it.\" Never \"X and others\" across people who \
         held different positions, and never put a person on a side their own messages \
         do not show.\n\
         - Keep apart things with similar names — two groups, two meetings, two \
         documents, two proposals. Say which one each note is about: by who made it, its \
         date or its name.\n\
         - Every meeting that took place is its own note, on the day it was held, with \
         what was decided.\n\
         - One fact per note, in one or two sentences, concrete: names, places, numbers \
         and dates as written. Small talk, jokes and greetings are not notes unless they \
         reveal one of the above.\n\
         - `day`: the day it happened, YYYY-MM-DD, from the day lines.\n\
         - `weight`: `major` for a turning point, or a discussion or dispute many took \
         part in or that ran long — even with no decision at its end; `medium` for a \
         concrete decision, a plan with a time or place, a notable disagreement, \
         something published; `minor` for routine logistics.\n\
         - `ids`: the most informative messages the note rests on, at most 10 — for a \
         discussion, each named person's own message stating their position.\n\
         A quiet stretch may have few notes; a busy one many. {lang}\n\
         Output only the JSON object.",
        of = about.of(),
        lang = language(lang),
    )
}

/// Fewer notes from many: the same shape in and out.
pub fn merge(about: &About, lang: Lang) -> String {
    format!(
        "You get notes taken from {of}, {span}, each `- day [weight] what [#ids]`. There are \
         too many to summarise at once. Merge them into fewer notes: join notes about the \
         same thing into one, keep every major note, keep medium notes that add something, \
         drop minor ones that repeat. A discussion stays one note with its positions and \
         its outcome. Never move a person from one position to another, never fold two \
         positions into one, and never join two things that only have similar names. \
         Keep each note's day, weight and ids (at most 10). \
         {lang}\nOutput only the JSON object.",
        of = about.of(),
        span = about.span(),
        lang = language(lang),
    )
}

/// The headings of a [`Length::Sections`] summary.
pub fn headings(lang: Lang) -> &'static str {
    match lang {
        Lang::Serbian => {
            "`## Ključni događaji`, `## Odluke`, `## Planovi i rokovi`, `## Nesuglasice`, \
             `## Otvorena pitanja`"
        }
        Lang::English => {
            "`## Key events`, `## Decisions`, `## Plans and deadlines`, `## Disagreements`, \
             `## Open questions`"
        }
        Lang::AsChat => {
            "Key events, Decisions, Plans and deadlines, Disagreements, Open questions — each \
             translated into that language and written as a `## ` heading"
        }
    }
}

/// The summary itself. `from_notes` says whether it reads notes or messages.
pub fn final_text(about: &About, length: Length, lang: Lang, from_notes: bool) -> String {
    let input = if from_notes {
        "notes taken from the messages, one per line: `- YYYY-MM-DD [weight] what happened \
         [#ids]`, where the ids are the messages each note rests on"
            .to_string()
    } else {
        format!("the messages themselves. {READING}")
    };
    let cite = "After each claim, cite the messages it rests on as [#id] or [#id, #id], at \
                most three ids, taken from the input exactly.";
    // ReVision's day and week entries: what mattered most first, and a
    // discussion counts as mattering even when nothing was decided.
    let weigh = "What matters most: decisions, conflicts and long discussions — even ones \
                 that ended without a decision — actions, and anything published.";
    let shape = match length {
        Length::Short => format!(
            "Write ONE paragraph of 3 to 6 sentences: the most important things that \
             happened, told as a story. {weigh} No heading, no list, no message ids."
        ),
        Length::Detailed => format!(
            "Write 2 to 6 short paragraphs separated by a blank line, one theme per paragraph, \
             the most important first. {weigh} A discussion gets its positions and how it \
             ended. Flowing prose: no headings, no lists. {cite}"
        ),
        Length::Sections => format!(
            "Write these sections, in this order: {}. Under each heading, short bullet points \
             starting with `- `, most important first. Leave out a section that would be \
             empty. {cite}",
            headings(lang)
        ),
    };
    format!(
        "You summarise what happened in {of} {span}. You get {input}\n\n{shape}\n\
         Name people as written, and give each person's position only as the input shows \
         it. Keep apart things with similar names — say which group, meeting or document \
         you mean. Mention a date only where the order matters. Only what the \
         input says: no guesses, no advice, no opening like \"in this period\". {lang}\n\
         Output only the summary.",
        of = about.of(),
        span = about.span(),
        lang = language(lang),
    )
}

/// What a question is answered from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// Every message of the range.
    Messages,
    /// The notes the question is about, and the messages around what they
    /// rest on.
    Sources,
    /// Every note, when no message could be looked up.
    Notes,
}

/// A conversation about the range: the system prompt, which the input
/// follows.
///
/// The rules come from a conversation in which the model, reading notes,
/// put one person on both sides of a dispute between two similarly named
/// groups, and when told so invented a story in which both were true.
pub fn chat(about: &About, basis: Basis) -> String {
    let input = match basis {
        Basis::Messages => format!("the messages themselves. {READING}"),
        Basis::Sources => format!(
            "notes taken from the messages, for orientation, then the messages the \
             question turns on and those around them. {READING} A line `…` marks where \
             messages of that day were left out. Where a note and a message disagree, the \
             message is right: answer from the messages, and cite them."
        ),
        Basis::Notes => "notes taken from the messages — the days were too many to read \
             whole — one per line: `- YYYY-MM-DD [weight] what happened [#ids]`, where the \
             ids are the messages each note rests on. A note can be wrong about who held \
             which position; when an answer turns on that, say it rests on the notes."
            .to_string(),
    };
    format!(
        "You answer questions about what was said in {of} {span}. You get {input}\n\n\
         - Answer from the input only. If it does not say, say so plainly; never guess, \
         and never fill a gap from what usually happens.\n\
         - Keep apart things with similar names — two groups, two meetings, two documents, \
         two proposals. Say which one each statement is about, by who made it, its date or \
         its name. If you cannot tell whether two mentions are the same thing, say so.\n\
         - A person's position is only what their own messages say, about each thing \
         separately, each with its citation. Never infer it from whom they agreed or \
         argued with. Someone can be for one thing and against another: say both. If their \
         position changed, or their messages contradict each other, say so, with the \
         dates.\n\
         - When told an earlier answer was wrong, check the input again and say plainly \
         which version it supports — even if that is the earlier answer. Never invent an \
         explanation that makes both true, and do not apologise for what the input \
         says.\n\
         - Be brief and concrete: who, what, when, as written.\n\
         - After each claim, cite the one to three messages that show it most directly, \
         as [#id] or [#id, #id], ids taken from the input exactly.\n\
         - Answer in the language of the question; if it is Serbian, use Latin script, \
         ekavian.",
        of = about.of(),
        span = about.span(),
    )
}

/// Choosing the notes a question is about, so their messages can be read.
pub fn pick(about: &About) -> String {
    format!(
        "You get numbered notes taken from {of} {span}, then a conversation about those \
         days and its newest question. Choose the notes whose messages are needed to \
         answer that question: the notes it is about, the notes about any person, group, \
         meeting or document it names, and — when it follows on from earlier turns — the \
         notes those turns were about. Most relevant first, at most {PICKED}. Choose none \
         if no note bears on it. Output only the JSON object.",
        of = about.of(),
        span = about.span(),
    )
}

/// The most notes [`pick`] may choose.
pub const PICKED: usize = 12;

/// [`pick`]'s answer: note numbers, as listed.
pub fn pick_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["notes"],
        "properties": {
            "notes": {
                "type": "array",
                "items": { "type": "integer" },
                "maxItems": PICKED
            }
        }
    })
}

/// The paragraph above a by-topic summary.
pub fn overview(about: &About, lang: Lang) -> String {
    format!(
        "You get summaries of each forum topic of {of} {span}, each under its topic's name. \
         Write ONE paragraph of 2 to 4 sentences: the overall picture across the topics — \
         what the group was mostly busy with and what mattered most. No heading, no list, no \
         message ids. Only what the summaries say. {lang}\nOutput only the paragraph.",
        of = about.of(),
        span = about.span(),
        lang = language(lang),
    )
}

/// The notes' shape. `maxItems` everywhere: without a cap Gemma has been
/// seen to loop on an id list ("487, 488, 489, …") until it hit the answer
/// limit.
pub fn notes_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["notes"],
        "properties": {
            "notes": {
                "type": "array",
                "maxItems": 40,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["day", "weight", "what", "ids"],
                    "properties": {
                        "day": { "type": "string", "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$" },
                        "weight": { "type": "string", "enum": ["major", "medium", "minor"] },
                        "what": { "type": "string" },
                        "ids": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "maxItems": 10
                        }
                    }
                }
            }
        }
    })
}
