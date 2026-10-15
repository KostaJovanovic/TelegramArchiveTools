//! Cutting a range into parts that each fit one call.
//!
//! **A day is the unit**, because a day line is how the model knows when
//! something happened and a summary is mostly told in days. Days are packed
//! into a part until the next would not fit. A day too big on its own is
//! split, at the longest quiet gap in the stretch where a cut would leave a
//! reasonably full part; a message is never split.
//!
//! The tokenizer is asked once per day, not per line: a month is thirty
//! requests rather than fifteen thousand. Inside a day that has to be split,
//! each line's share is estimated from its length, which is why a split aims
//! under the budget rather than at it.

use chrono::NaiveDate;
use tga_notes::Row;

use crate::lines::{day_line, day_of, line, minute_of};
use crate::Error;

/// A split day aims this far under the budget, since its pieces are
/// estimated rather than counted.
const SPLIT_SHARE: f64 = 0.9;

/// A cut inside a day goes at the longest gap once the piece is at least this
/// full, so a long lunch break early in the morning does not leave a part of
/// ten messages.
const SPLIT_FLOOR: f64 = 0.6;

/// One day of messages, rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Day {
    pub date: NaiveDate,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub id: i64,
    pub minute: u32,
    /// The message this one answers.
    pub re: Option<i64>,
    pub text: String,
}

impl Day {
    pub fn text(&self) -> String {
        render(self.date, &self.lines)
    }
}

/// What one call reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub first: NaiveDate,
    pub last: NaiveDate,
    pub text: String,
    pub ids: Vec<i64>,
    /// Counted, or for a piece of a split day, estimated.
    pub tokens: usize,
}

/// Rows grouped into days, oldest first. `topic` names a row's topic when the
/// lines mix topics, and returns `None` otherwise.
///
/// Borrowed rows, not a slice of them: a caller holding `&Row`s would
/// otherwise copy every one — document and photo text included — to call this.
pub fn days<'r>(
    rows: impl IntoIterator<Item = &'r Row>,
    topic: &dyn Fn(&Row) -> Option<String>,
) -> Vec<Day> {
    let mut out: Vec<Day> = Vec::new();
    for row in rows {
        let Some(date) = day_of(row) else { continue };
        let rendered = Line {
            id: row.id,
            minute: minute_of(row),
            re: row.re,
            text: line(row, topic(row).as_deref()),
        };
        match out.last_mut() {
            Some(day) if day.date == date => day.lines.push(rendered),
            _ => out.push(Day {
                date,
                lines: vec![rendered],
            }),
        }
    }
    out
}

/// Every day's token count, one tokenizer call each.
pub fn measure(
    days: &[Day],
    count: &mut dyn FnMut(&str) -> Result<usize, Error>,
) -> Result<Vec<usize>, Error> {
    days.iter().map(|d| count(&d.text())).collect()
}

/// Pack days into parts of at most `budget` tokens.
pub fn cut(days: &[Day], tokens: &[usize], budget: usize) -> Vec<Part> {
    let mut units: Vec<Part> = Vec::new();
    for (day, &n) in days.iter().zip(tokens) {
        if n <= budget {
            units.push(part(day.date, &day.lines, n));
        } else {
            units.extend(split(day, n, budget));
        }
    }
    let mut parts: Vec<Part> = Vec::new();
    for unit in units {
        match parts.last_mut() {
            Some(open) if open.tokens + unit.tokens <= budget => {
                open.last = unit.last;
                open.text.push('\n');
                open.text.push_str(&unit.text);
                open.ids.extend(unit.ids);
                open.tokens += unit.tokens;
            }
            _ => parts.push(unit),
        }
    }
    parts
}

/// A day too big for one part, in pieces.
fn split(day: &Day, tokens: usize, budget: usize) -> Vec<Part> {
    let total: usize = day
        .lines
        .iter()
        .map(|l| l.text.len() + 1)
        .sum::<usize>()
        .max(1);
    let share: Vec<f64> = day
        .lines
        .iter()
        .map(|l| tokens as f64 * (l.text.len() + 1) as f64 / total as f64)
        .collect();
    let limit = budget as f64 * SPLIT_SHARE;

    let mut out = Vec::new();
    let mut start = 0;
    while start < day.lines.len() {
        let mut sum = 0.0;
        let mut end = start;
        while end < day.lines.len() && (end == start || sum + share[end] <= limit) {
            sum += share[end];
            end += 1;
        }
        // Everything left fitted: no cut to choose.
        if end < day.lines.len() {
            end = quietest_cut(&day.lines, &share, start, end, limit);
        }
        let estimate: f64 = share[start..end].iter().sum();
        out.push(part(
            day.date,
            &day.lines[start..end],
            estimate.ceil() as usize,
        ));
        start = end;
    }
    out
}

/// Where to end a piece that runs `start..end`: before the line that follows
/// the longest silence, among cuts that leave the piece [`SPLIT_FLOOR`] full.
fn quietest_cut(lines: &[Line], share: &[f64], start: usize, end: usize, limit: f64) -> usize {
    let mut best = end;
    let mut best_gap = 0;
    let mut sum = 0.0;
    for k in start + 1..=end {
        sum += share[k - 1];
        if sum < limit * SPLIT_FLOOR || k >= lines.len() {
            continue;
        }
        let gap = lines[k].minute.saturating_sub(lines[k - 1].minute);
        if gap > best_gap {
            best_gap = gap;
            best = k;
        }
    }
    best
}

fn part(date: NaiveDate, lines: &[Line], tokens: usize) -> Part {
    Part {
        first: date,
        last: date,
        text: render(date, lines),
        ids: lines.iter().map(|l| l.id).collect(),
        tokens,
    }
}

pub fn render(date: NaiveDate, lines: &[Line]) -> String {
    let mut s = day_line(date);
    for l in lines {
        s.push('\n');
        s.push_str(&l.text);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, t: &str, text: &str) -> Row {
        Row {
            id,
            t: t.into(),
            who: "A".into(),
            text: text.into(),
            ..Default::default()
        }
    }

    /// A stand-in tokenizer: a token per four bytes.
    fn quarter(text: &str) -> Result<usize, Error> {
        Ok(text.len().div_ceil(4))
    }

    fn plan(rows: &[Row], budget: usize) -> Vec<Part> {
        let d = days(rows, &|_| None);
        // The same days from borrowed rows, which is how a summary calls it.
        assert_eq!(d, days(rows.iter().collect::<Vec<&Row>>(), &|_| None));
        let n = measure(&d, &mut quarter).unwrap();
        cut(&d, &n, budget)
    }

    #[test]
    fn small_days_share_a_part_and_a_full_part_closes() {
        let rows = vec![
            row(1, "2024-12-01 10:00", "x"),
            row(2, "2024-12-02 10:00", "y"),
            row(3, "2024-12-03 10:00", &"z".repeat(400)),
        ];
        let parts = plan(&rows, 120);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].ids, vec![1, 2]);
        assert_eq!(parts[0].first.to_string(), "2024-12-01");
        assert_eq!(parts[0].last.to_string(), "2024-12-02");
        assert!(parts[0].text.contains("== 2024-12-02 Mon =="));
        assert_eq!(parts[1].ids, vec![3]);
    }

    #[test]
    fn everything_fits_in_one_when_the_budget_allows() {
        let rows: Vec<Row> = (0..30)
            .map(|i| row(i, &format!("2024-12-{:02} 10:00", 1 + i % 7), "hello"))
            .collect();
        let mut sorted = rows.clone();
        sorted.sort_by(|a, b| a.t.cmp(&b.t));
        assert_eq!(plan(&sorted, 100_000).len(), 1);
    }

    #[test]
    fn a_big_day_splits_at_its_longest_quiet_gap_and_keeps_every_message() {
        // Forty messages a minute apart, a three-hour silence, forty more.
        let mut rows = Vec::new();
        for i in 0..40 {
            rows.push(row(i, &format!("2024-12-02 08:{:02}", i), &"m".repeat(60)));
        }
        for i in 0..40 {
            rows.push(row(
                100 + i,
                &format!("2024-12-02 12:{:02}", i),
                &"m".repeat(60),
            ));
        }
        let parts = plan(&rows, 1200);
        let ids: Vec<i64> = parts.iter().flat_map(|p| p.ids.clone()).collect();
        assert_eq!(ids, rows.iter().map(|r| r.id).collect::<Vec<_>>());
        assert_eq!(
            parts.len(),
            2,
            "{:?}",
            parts.iter().map(|p| p.ids.len()).collect::<Vec<_>>()
        );
        assert_eq!(parts[0].ids.last(), Some(&39), "cut at the silence");
        for p in &parts {
            assert!(p.tokens <= 1200);
            assert!(p.text.starts_with("== 2024-12-02 Mon =="));
        }
    }

    #[test]
    fn one_message_bigger_than_the_budget_still_goes_somewhere() {
        let rows = vec![
            row(1, "2024-12-02 08:00", &"x".repeat(2000)),
            row(2, "2024-12-02 08:01", "y"),
        ];
        let parts = plan(&rows, 100);
        let ids: Vec<i64> = parts.iter().flat_map(|p| p.ids.clone()).collect();
        assert_eq!(ids, vec![1, 2]);
    }
}
