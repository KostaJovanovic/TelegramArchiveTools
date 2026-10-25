//! Each person's, topic's and month's own words, and the alphabet they write.

use tga_stats::{Stats, WordSet};

use super::*;
use crate::charts::{self, esc, thousands};

/// Below this share of letters, the minority script gets one sentence rather
/// than two charts that are a row of empty columns.
const SCRIPT_FLOOR: f64 = 0.01;

/// A group's words as one line: `word 12 · other 9`.
fn line(set: &WordSet) -> String {
    set.words
        .iter()
        .map(|w| {
            format!(
                "{}<span class=\"num muted\"> {}</span>",
                esc(&w.label),
                thousands(w.n)
            )
        })
        .collect::<Vec<_>>()
        .join(" &#183; ")
}

fn sets(title: &str, column: &str, take: &str, method: &str, sets: &[WordSet]) -> String {
    let rows: Vec<Vec<String>> = sets
        .iter()
        .filter(|s| !s.words.is_empty())
        .map(|s| vec![short(&s.name, 28), line(s)])
        .collect();
    if rows.is_empty() {
        return String::new();
    }
    panel(
        title,
        take,
        method,
        &table(
            &[
                (column, false),
                ("Words used more here than anywhere else", false),
            ],
            &rows,
        ),
    )
}

fn script(stats: &Stats) -> String {
    let Some(words) = &stats.words else {
        return String::new();
    };
    let s = &words.script;
    let letters = s.cyrillic + s.latin;
    if letters == 0 {
        return String::new();
    }
    let cyrillic = s.cyrillic as f64 / letters as f64;
    let messages = s.messages_cyrillic + s.messages_latin + s.messages_mixed;
    let method = format!(
        "Letters, not messages, so a one-word reply and a page of minutes do not \
         count the same. Links, @handles and #tags are left out: a link is Latin \
         whatever its sender writes in. Of the {} messages with any letters, {} are \
         in Cyrillic, {} in Latin and {} mixed &#8212; a message is mixed when \
         neither script has nine letters in ten.",
        thousands(messages),
        thousands(s.messages_cyrillic),
        thousands(s.messages_latin),
        thousands(s.messages_mixed)
    );
    let minority = cyrillic.min(1.0 - cyrillic);
    if minority < SCRIPT_FLOOR {
        let (main, other) = if cyrillic >= 0.5 {
            ("Cyrillic", "Latin")
        } else {
            ("Latin", "Cyrillic")
        };
        return format!(
            "<div class=\"caption\">Written almost entirely in {main}: {} of the letters \
             are {other}. {}</div>",
            pct(minority),
            how(&method)
        );
    }

    let labels: Vec<String> = s
        .months
        .iter()
        .map(|m| m.chars().skip(2).collect())
        .collect();
    let values: Vec<i64> = s
        .month_cyrillic
        .iter()
        .map(|v| (v * 1000.0).round() as i64)
        .collect();
    let shown = |n: i64| format!("{:.1}% Cyrillic", n as f64 / 10.0);
    let months = if s.months.len() >= 2 {
        panel(
            "Cyrillic, month by month",
            "",
            "The Cyrillic share of each month&#8217;s letters.",
            &charts::vcols(&charts::Cols {
                labels: &labels,
                values: &values,
                shown: &shown,
                every: (s.months.len() / 14).max(1),
                id: "",
            }),
        )
    } else {
        String::new()
    };
    let rows: Vec<charts::HBar> = s
        .people
        .iter()
        .map(|p| {
            charts::HBar::new(
                p.name.clone(),
                p.share * 100.0,
                format!("{} Cyrillic", pct(p.share)),
            )
            .who(p.key.clone())
        })
        .collect();
    let people = panel(
        "Who writes in Cyrillic",
        &format!(
            "<b>{}</b> of all the letters are Cyrillic and {} Latin.",
            pct(cyrillic),
            pct(1.0 - cyrillic)
        ),
        &format!(
            "The people who wrote the most letters, each by the Cyrillic share of \
             their own. {method}"
        ),
        &charts::hbars(&rows, None),
    );
    format!("<div class=\"cols2\">{people}{months}</div>")
}

pub fn words(stats: &Stats) -> String {
    let Some(words) = &stats.words else {
        return String::new();
    };
    if words.tokens == 0 {
        return String::new();
    }
    let method = "Each word is scored on how much more often this person, topic or \
                  month uses it than the rest of the archive does &#8212; a log-odds \
                  ratio with the archive itself as the prior, as a z-score &#8212; so \
                  the words everyone uses fall away without a stopword list. A word \
                  counts once per message, so one pasted page cannot make its \
                  vocabulary its poster&#8217;s; forwarded messages are left out, \
                  since somebody else wrote them; and Serbian Cyrillic is read as \
                  Latin, so <i>да</i> and <i>da</i> are one word. The number is how \
                  many of the group&#8217;s messages used the word, which is written \
                  the way the group most often wrote it.";
    let take = format!(
        "{} uses of {} different words, counted once per message.",
        thousands(words.tokens),
        thousands(words.vocabulary as i64)
    );
    // The sentence and the method go on whichever list renders first; three
    // copies of one method is three things competing with the words.
    let cuts = [
        ("Each person\u{2019}s own words", "Person", &words.people),
        ("Each topic\u{2019}s own words", "Topic", &words.topics),
        ("Each month\u{2019}s own words", "Month", &words.months),
    ];
    let mut lists = String::new();
    for (title, column, groups) in cuts {
        let first = lists.is_empty();
        lists.push_str(&sets(
            title,
            column,
            if first { &take } else { "" },
            if first { method } else { "" },
            groups,
        ));
    }
    if lists.is_empty() && script(stats).is_empty() {
        return String::new();
    }
    format!(
        "{}{lists}{}</section>",
        head(
            "Words",
            &format!("{} distinct", thousands(words.vocabulary as i64)),
            "words"
        ),
        script(stats)
    )
}
