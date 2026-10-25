//! The words each person, topic and month uses more than everyone else, and
//! the alphabet each writes in.
//!
//! **Scored, not counted.** Ranked on frequency every group's list is the
//! same list — *i*, *da*, *je*, *to* — so each word is scored on how much more
//! often this group uses it than the rest of the archive does: the log-odds
//! ratio with an informative Dirichlet prior (Monroe, Colaresi and Quinn,
//! 2008), as a z-score. The archive itself is the prior, which is what lets it
//! work with no stopword list and in whatever language the chat is in.
//!
//! Three rules decide what a "use" is, each because the obvious reading was
//! wrong:
//!
//! * **A word counts once per message.** One pasted page of minutes would
//!   otherwise make its vocabulary the poster's.
//! * **A forwarded message is nobody's words here.** It was written elsewhere.
//! * **Serbian Cyrillic is read as Latin for counting**, so *да* and *da* are
//!   one word — the group writes both and means one. Each word is still shown
//!   in the script that group most often wrote it in; a Cyrillic word is shown
//!   as it was first spelled anywhere, which the transliteration makes nearly
//!   always the only way it is spelled.
//!   Only the Serbian alphabet is transliterated; a Russian or Ukrainian
//!   letter is left as it is rather than mapped onto something it is not.
//!
//! Links, @mentions and #hashtags are not words and are skipped, for the
//! script count too: a URL is Latin whatever its sender writes in.

use std::collections::HashMap;

use tga_read::{Export, Msg};
use tga_stats::{Count, Script, ScriptRow, WordSet, Words};

use crate::identity::People;
use crate::util::{month_axis, MonthIndex};

/// A word shorter than this is almost always a particle, and the score
/// handles the rest.
const MIN_LETTERS: usize = 3;

/// Messages a group must have used a word in before it can be the group's.
const MIN_USES: u32 = 4;

/// How many words each group is given.
const SHOWN: usize = 12;

/// Word uses a person needs before they get a list, and how many people can.
const PERSON_MIN_TOKENS: u32 = 300;
const PEOPLE_SHOWN: usize = 30;

/// The prior's weight, in pseudo-uses. Large enough that a word used twice by
/// one person and never by anyone else does not top their list; small enough
/// that a group's own habits still show through.
const PRIOR: f64 = 10_000.0;

/// How many people the script table ranks.
const SCRIPT_PEOPLE: usize = 30;

/// A message is in one script when that script has this share of its letters.
const PURE: f64 = 0.9;

/// Serbian Cyrillic to Serbian Latin, one letter at a time.
fn latin_of(ch: char) -> Option<&'static str> {
    Some(match ch {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'ђ' => "đ",
        'е' => "e",
        'ж' => "ž",
        'з' => "z",
        'и' => "i",
        'ј' => "j",
        'к' => "k",
        'л' => "l",
        'љ' => "lj",
        'м' => "m",
        'н' => "n",
        'њ' => "nj",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'ћ' => "ć",
        'у' => "u",
        'ф' => "f",
        'х' => "h",
        'ц' => "c",
        'ч' => "č",
        'џ' => "dž",
        'ш' => "š",
        _ => return None,
    })
}

#[cfg(test)]
fn latinise(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for ch in word.chars() {
        match latin_of(ch) {
            Some(latin) => out.push_str(latin),
            None => out.push(ch),
        }
    }
    out
}

fn is_cyrillic(ch: char) -> bool {
    matches!(ch, '\u{0400}'..='\u{052F}')
}

fn is_latin(ch: char) -> bool {
    ch.is_ascii_alphabetic() || (matches!(ch, '\u{00C0}'..='\u{024F}') && ch.is_alphabetic())
}

/// Whitespace-separated pieces that are not a link, a mention or a hashtag.
fn plain_pieces(text: &str) -> impl Iterator<Item = &str> {
    text.split_whitespace().filter(|piece| {
        let lower =
            piece.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '@' && c != '#');
        !(piece.contains("://")
            || lower.starts_with("www.")
            || lower.starts_with('@')
            || lower.starts_with('#'))
    })
}

/// Every word in a message: `each(counted, cyrillic, written)`.
///
/// `counted` is the word lowercased and read as Latin — the form it is
/// counted under — built in one reused buffer, because a fresh string per word
/// was most of what this cost on half a million messages. `cyrillic` says the
/// word had a Cyrillic letter in it, and `written` is the word as it stood.
fn words(text: &str, buf: &mut String, mut each: impl FnMut(&str, bool, &str)) {
    for piece in plain_pieces(text) {
        for word in piece.split(|c: char| !c.is_alphabetic()) {
            if word.chars().nth(MIN_LETTERS - 1).is_none() {
                continue;
            }
            buf.clear();
            let mut cyrillic = false;
            for ch in word.chars().flat_map(char::to_lowercase) {
                cyrillic |= is_cyrillic(ch);
                match latin_of(ch) {
                    Some(latin) => buf.push_str(latin),
                    None => buf.push(ch),
                }
            }
            each(buf, cyrillic, word);
        }
    }
}

/// Cyrillic and Latin letters in a message, links and handles left out.
fn letters(text: &str) -> (i64, i64) {
    let (mut cyr, mut lat) = (0, 0);
    for piece in plain_pieces(text) {
        for ch in piece.chars() {
            if is_cyrillic(ch) {
                cyr += 1;
            } else if is_latin(ch) {
                lat += 1;
            }
        }
    }
    (cyr, lat)
}

/// How often a group used a word, and how many of those uses were written in
/// Cyrillic — which is all that is needed to show the word the way the group
/// wrote it, since read as Latin the two spellings are the same word.
#[derive(Default, Clone, Copy)]
struct Uses {
    all: u32,
    cyrillic: u32,
}

/// One way of cutting the archive into groups: by person, topic or month.
struct Cut {
    names: Vec<(String, String)>,
    counts: Vec<HashMap<u32, Uses>>,
    tokens: Vec<u32>,
}

impl Cut {
    fn new() -> Self {
        Cut {
            names: Vec::new(),
            counts: Vec::new(),
            tokens: Vec::new(),
        }
    }

    /// A new group, in the order groups are first met.
    fn open(&mut self, key: String, name: String) -> usize {
        self.names.push((key, name));
        self.counts.push(HashMap::new());
        self.tokens.push(0);
        self.names.len() - 1
    }

    fn add(&mut self, at: usize, ids: &[(u32, bool)]) {
        let counts = &mut self.counts[at];
        for (id, cyrillic) in ids {
            let uses = counts.entry(*id).or_default();
            uses.all += 1;
            uses.cyrillic += u32::from(*cyrillic);
        }
        self.tokens[at] += ids.len() as u32;
    }
}

/// The z-score of one word for one group against everything else.
fn z(used: u32, group: u32, everywhere: u32, all: u32) -> f64 {
    let (y, n, yw, total) = (used as f64, group as f64, everywhere as f64, all as f64);
    let alpha = PRIOR * yw / total;
    let (rest_y, rest_n) = (yw - y, total - n);
    let tiny = 1e-9;
    let ours = ((y + alpha) / (n + PRIOR - y - alpha).max(tiny)).ln();
    let theirs = ((rest_y + alpha) / (rest_n + PRIOR - rest_y - alpha).max(tiny)).ln();
    let variance = 1.0 / (y + alpha) + 1.0 / (rest_y + alpha);
    (ours - theirs) / variance.sqrt()
}

/// The `SHOWN` best-scoring words of each group in `wanted`, as word ids.
fn pick(cut: &Cut, wanted: &[usize], global: &[u32], all: u32) -> Vec<Vec<(u32, u32)>> {
    wanted
        .iter()
        .map(|&g| {
            let mut scored: Vec<(f64, u32, u32)> = cut.counts[g]
                .iter()
                .map(|(&id, uses)| (id, uses.all))
                .filter(|(_, used)| *used >= MIN_USES)
                .map(|(id, used)| (z(used, cut.tokens[g], global[id as usize], all), id, used))
                .filter(|(score, _, _)| *score > 0.0)
                .collect();
            // Highest score first, and on a tie the lower id — the word met
            // first in the archive — so the order never depends on a hash.
            scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            scored
                .into_iter()
                .take(SHOWN)
                .map(|(_, id, used)| (id, used))
                .collect()
        })
        .collect()
}

pub fn compute(export: &Export, people: &People) -> Words {
    let said: Vec<&Msg> = export
        .said()
        .filter(|m| m.forward_from.is_empty() && m.forward_id.is_empty())
        .collect();

    // Word id -> the word as counted, how many messages used it anywhere, and
    // the first Cyrillic spelling met, for the groups that wrote it that way.
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut counted: Vec<String> = Vec::new();
    let mut global: Vec<u32> = Vec::new();
    let mut cyrillic_form: Vec<Option<String>> = Vec::new();
    let mut all: u32 = 0;
    let (mut by_person, mut by_topic, mut by_month) = (Cut::new(), Cut::new(), Cut::new());
    let mut person_at: HashMap<String, usize> = HashMap::new();
    let mut topic_at: HashMap<usize, usize> = HashMap::new();

    let mut script = Script::default();
    let months = month_axis(&said);
    let month_index = MonthIndex::new(&months);
    let mut month_at: Vec<Option<usize>> = vec![None; months.len()];
    let mut month_letters: Vec<(i64, i64)> = vec![(0, 0); months.len()];
    let mut person_letters: HashMap<String, (i64, i64)> = HashMap::new();
    let mut person_order: Vec<String> = Vec::new();

    let mut buf = String::new();
    let mut message_ids: Vec<(u32, bool)> = Vec::new();
    for msg in &said {
        let key = people.key_ref(msg);
        let month = month_index.of(&msg.when);

        let (cyr, lat) = letters(&msg.text);
        if cyr + lat > 0 {
            script.cyrillic += cyr;
            script.latin += lat;
            let share = cyr as f64 / (cyr + lat) as f64;
            if share >= PURE {
                script.messages_cyrillic += 1;
            } else if share <= 1.0 - PURE {
                script.messages_latin += 1;
            } else {
                script.messages_mixed += 1;
            }
            if let Some(m) = month {
                month_letters[m].0 += cyr;
                month_letters[m].1 += lat;
            }
            if !key.is_empty() {
                let p = match person_letters.get_mut(key.as_ref()) {
                    Some(p) => p,
                    None => {
                        person_order.push(key.to_string());
                        person_letters.entry(key.to_string()).or_default()
                    }
                };
                p.0 += cyr;
                p.1 += lat;
            }
        }

        message_ids.clear();
        words(&msg.text, &mut buf, |norm, cyrillic, written| {
            let id = match ids.get(norm) {
                Some(&id) => id,
                None => {
                    let id = counted.len() as u32;
                    ids.insert(norm.to_string(), id);
                    counted.push(norm.to_string());
                    global.push(0);
                    cyrillic_form.push(None);
                    id
                }
            };
            if cyrillic && cyrillic_form[id as usize].is_none() {
                cyrillic_form[id as usize] = Some(written.to_lowercase());
            }
            message_ids.push((id, cyrillic));
        });
        if message_ids.is_empty() {
            continue;
        }
        // Once per message: a word written once in each script is one use,
        // and a Cyrillic one.
        message_ids.sort_unstable_by_key(|(id, _)| *id);
        message_ids.dedup_by(|later, kept| {
            if later.0 == kept.0 {
                kept.1 |= later.1;
                true
            } else {
                false
            }
        });
        for (id, _) in &message_ids {
            global[*id as usize] += 1;
        }
        all += message_ids.len() as u32;
        if !key.is_empty() {
            let at = match person_at.get(key.as_ref()) {
                Some(&at) => at,
                None => {
                    let at = by_person.open(key.to_string(), people.name_of(&key));
                    person_at.insert(key.to_string(), at);
                    at
                }
            };
            by_person.add(at, &message_ids);
        }
        let at = *topic_at.entry(msg.topic).or_insert_with(|| {
            let name = export
                .topics
                .iter()
                .find(|t| t.index == msg.topic)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| msg.topic.to_string());
            by_topic.open(msg.topic.to_string(), name)
        });
        by_topic.add(at, &message_ids);
        if let Some(m) = month {
            let at = *month_at[m]
                .get_or_insert_with(|| by_month.open(months[m].clone(), months[m].clone()));
            by_month.add(at, &message_ids);
        }
    }

    // Which groups get a list: the people who wrote enough, loudest first;
    // every topic and every month — but only when there are at least two,
    // since a group compared against nothing has no distinctive words.
    let mut people_wanted: Vec<usize> = (0..by_person.names.len())
        .filter(|&g| by_person.tokens[g] >= PERSON_MIN_TOKENS)
        .collect();
    people_wanted.sort_by(|a, b| {
        by_person.tokens[*b]
            .cmp(&by_person.tokens[*a])
            .then(a.cmp(b))
    });
    people_wanted.truncate(PEOPLE_SHOWN);
    let many = |cut: &Cut| -> Vec<usize> {
        if cut.names.len() < 2 {
            Vec::new()
        } else {
            (0..cut.names.len()).collect()
        }
    };
    let mut topics_wanted = many(&by_topic);
    topics_wanted.sort_by_key(|&g| by_topic.names[g].0.parse::<usize>().unwrap_or(usize::MAX));
    // Months in calendar order, which is also the order they were met in.
    let months_wanted = many(&by_month);
    if people_wanted.len() < 2 {
        people_wanted.clear();
    }

    let people_picks = pick(&by_person, &people_wanted, &global, all);
    let topic_picks = pick(&by_topic, &topics_wanted, &global, all);
    let month_picks = pick(&by_month, &months_wanted, &global, all);

    // A word is shown in Cyrillic where most of the group's uses of it were.
    // Read as Latin the two spellings are one word, and a Latin one is the
    // counted form itself, so nothing more than the tally is needed.
    let form = |cut: &Cut, group: usize, id: u32| -> String {
        let uses = cut.counts[group].get(&id).copied().unwrap_or_default();
        match &cyrillic_form[id as usize] {
            Some(cyrillic) if uses.cyrillic * 2 > uses.all => cyrillic.clone(),
            _ => counted[id as usize].clone(),
        }
    };
    let sets = |cut: &Cut, groups: &[usize], picks: &[Vec<(u32, u32)>]| -> Vec<WordSet> {
        groups
            .iter()
            .zip(picks)
            .map(|(&g, words)| WordSet {
                key: cut.names[g].0.clone(),
                name: cut.names[g].1.clone(),
                tokens: cut.tokens[g] as i64,
                words: words
                    .iter()
                    .map(|(id, used)| Count::new(form(cut, g, *id), *used as i64))
                    .collect(),
            })
            .collect()
    };

    let share = |(cyr, lat): (i64, i64)| {
        if cyr + lat > 0 {
            cyr as f64 / (cyr + lat) as f64
        } else {
            0.0
        }
    };
    script.month_cyrillic = month_letters.iter().map(|m| share(*m)).collect();
    script.months = months;
    let mut rows: Vec<ScriptRow> = person_order
        .iter()
        .map(|key| {
            let (cyr, lat) = person_letters[key];
            ScriptRow {
                key: key.clone(),
                name: people.name_of(key),
                cyrillic: cyr,
                latin: lat,
                share: share((cyr, lat)),
            }
        })
        .collect();
    // Stable: on a tie, the person met first.
    rows.sort_by_key(|r| std::cmp::Reverse(r.cyrillic + r.latin));
    rows.truncate(SCRIPT_PEOPLE);
    script.people = rows;

    Words {
        tokens: all as i64,
        vocabulary: ids.len(),
        people: sets(&by_person, &people_wanted, &people_picks),
        topics: sets(&by_topic, &topics_wanted, &topic_picks),
        months: sets(&by_month, &months_wanted, &month_picks),
        script,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, export, msg, on};

    fn said(id: i64, who: &str, when: chrono::NaiveDateTime, text: &str) -> Msg {
        let mut m = msg(id, who, when, None);
        m.text = text.into();
        m
    }

    #[test]
    fn serbian_cyrillic_is_counted_as_latin_and_nothing_else_is() {
        assert_eq!(latinise("љубав"), "ljubav");
        assert_eq!(latinise("џак"), "džak");
        assert_eq!(latinise("ćevap"), "ćevap");
        // Russian letters are not Serbian ones and are left alone.
        assert_eq!(latinise("ы"), "ы");
    }

    #[test]
    fn links_handles_and_tags_are_not_words() {
        let mut got = Vec::new();
        words(
            "Pogledaj https://example.com/stvar @neko #tag www.sajt.rs (zagrade) ok",
            &mut String::new(),
            |w, _, _| got.push(w.to_string()),
        );
        assert_eq!(got, ["pogledaj", "zagrade"]);
        assert_eq!(letters("abc где https://x.yz"), (3, 3));
    }

    #[test]
    fn a_word_is_counted_as_latin_and_remembers_it_was_cyrillic() {
        let mut got = Vec::new();
        words(
            "Љубав ДА",
            &mut String::new(),
            |w, cyrillic, written| got.push((w.to_string(), cyrillic, written.to_string())),
        );
        assert_eq!(got, [("ljubav".to_string(), true, "Љубав".to_string())]);
    }

    /// Two people with a shared vocabulary and one word each of their own.
    fn two_voices() -> Export {
        let mut msgs = Vec::new();
        let mut id = 0;
        for day in 1..=28 {
            for (who, text) in [
                ("user1", "danas sastanak kafana"),
                ("user2", "danas sastanak plakat"),
            ] {
                id += 1;
                msgs.push(said(id, who, at(day, 10, 0), text));
            }
        }
        // Twelve more each, so both clear the floor for a list of their own.
        for n in 0..120 {
            id += 1;
            msgs.push(said(
                id,
                "user1",
                at(1, 11, 0) + chrono::Duration::minutes(n),
                "danas sastanak",
            ));
            id += 1;
            msgs.push(said(
                id,
                "user2",
                at(1, 11, 0) + chrono::Duration::minutes(n),
                "danas sastanak",
            ));
        }
        export(msgs)
    }

    #[test]
    fn each_person_gets_the_word_only_they_use_and_not_the_shared_ones() {
        let export = two_voices();
        let people = People::new(&export);
        let out = compute(&export, &people);
        let of = |key: &str| -> Vec<String> {
            out.people
                .iter()
                .find(|s| s.key == key)
                .expect("a list")
                .words
                .iter()
                .map(|w| w.label.clone())
                .collect()
        };
        assert_eq!(of("user1"), ["kafana"]);
        assert_eq!(of("user2"), ["plakat"]);
        // One topic is nothing to compare against.
        assert!(out.topics.is_empty());
    }

    #[test]
    fn a_word_counts_once_per_message_and_a_forward_counts_for_nobody() {
        let mut fwd = said(2, "user2", at(1, 10, 0), "tuđe tuđe tuđe");
        fwd.forward_from = "Channel".into();
        let export = export(vec![said(1, "user1", at(1, 9, 0), "isto isto isto"), fwd]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert_eq!(out.tokens, 1);
        assert_eq!(out.vocabulary, 1);
    }

    #[test]
    fn a_word_is_shown_the_way_its_group_wrote_it() {
        let mut msgs = Vec::new();
        for n in 0..10 {
            msgs.push(said(n + 1, "user1", at(1, 9, 0), "блокада"));
            msgs.push(said(n + 100, "user2", on(2025, 2, 1, 9, 0), "blokada"));
            msgs.push(said(n + 200, "user2", on(2025, 2, 1, 9, 0), "ništa drugo"));
        }
        let export = export(msgs);
        let people = People::new(&export);
        let out = compute(&export, &people);
        let january = &out.months[0];
        assert_eq!(january.words[0].label, "блокада");
        assert_eq!(out.script.months, ["2025-01", "2025-02"]);
        assert_eq!(out.script.month_cyrillic[0], 1.0);
        assert_eq!(out.script.month_cyrillic[1], 0.0);
        assert_eq!(out.script.messages_cyrillic, 10);
        assert_eq!(out.script.messages_latin, 20);
        assert_eq!(out.script.people[0].key, "user2");
    }
}
