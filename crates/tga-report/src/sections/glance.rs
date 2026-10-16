//! At a glance: four to six findings in plain words, before any chart.
//!
//! **Computed, not written.** Every line here is read off the same figures the
//! charts below draw, and says where it came from by being the first thing
//! those charts show. Nothing is chosen by a model and nothing is a guess: a
//! finding the figures cannot support is left out rather than softened.

use tga_stats::Stats;

use super::*;
use crate::charts::{esc, thousands};

/// The findings, as `(headline, detail)` markup pairs. At most six.
pub fn findings(stats: &Stats) -> Vec<(String, String)> {
    let mut out = match duo(stats) {
        Some((a, b)) => pair(stats, a, b),
        None => group(stats),
    };
    out.extend(shared(stats));
    out.truncate(6);
    out
}

/// The block itself, or nothing when there is nothing to say.
pub fn glance(stats: &Stats) -> String {
    let items = findings(stats);
    if items.is_empty() {
        return String::new();
    }
    let cells: String = items
        .iter()
        .map(|(head, detail)| format!("<li><b>{head}</b><span>{detail}</span></li>"))
        .collect();
    format!("<ul class=\"glance\" aria-label=\"At a glance\">{cells}</ul>")
}

/// When the chat is loudest: the single busiest hour of the week.
///
/// Said as an hour, because the busiest *weekday* can be another day — a
/// Friday that is busy all day beats a Saturday that is busy only late — and
/// the Rhythm section names that one.
fn busiest_time(stats: &Stats) -> Option<(String, String)> {
    let grid = &stats.activity.hour_weekday;
    let (day, hour, n) = grid
        .iter()
        .enumerate()
        .flat_map(|(d, row)| row.iter().enumerate().map(move |(h, n)| (d, h, *n)))
        .max_by_key(|(d, h, n)| (*n, std::cmp::Reverse((*d, *h))))?;
    if n == 0 {
        return None;
    }
    Some((
        format!("Busiest hour: {}s at {hour:02}:00", WEEKDAYS[day.min(6)]),
        format!("{} messages in that hour of the week", thousands(n)),
    ))
}

fn group(stats: &Stats) -> Vec<(String, String)> {
    let mut out = Vec::new();
    out.extend(busiest_time(stats));

    // How many people carry half of it: the one number for "is this a group
    // or a few people with an audience".
    let mut counts: Vec<i64> = stats
        .people
        .rows
        .iter()
        .map(|r| r.messages)
        .filter(|n| *n > 0)
        .collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));
    let total: i64 = counts.iter().sum();
    if total > 0 && counts.len() > 2 {
        let mut running = 0;
        let half = counts
            .iter()
            .position(|n| {
                running += n;
                running * 2 >= total
            })
            .map_or(counts.len(), |at| at + 1);
        out.push((
            format!(
                "{} {} wrote half of everything",
                thousands(half as i64),
                if half == 1 { "person" } else { "people" }
            ),
            format!("of {} who wrote anything", thousands(counts.len() as i64)),
        ));
    }

    let talk = &stats.conversation;
    if talk.latency_counted > 0 {
        out.push((
            format!(
                "Typical reply within {}",
                esc(&duration(talk.latency_median as f64))
            ),
            format!(
                "nine in ten within {}",
                esc(&duration(talk.latency_p90 as f64))
            ),
        ));
    }

    let busiest_topic = stats
        .topics
        .iter()
        .filter(|t| t.messages > 0)
        .max_by_key(|t| t.messages);
    if let (Some(topic), true) = (busiest_topic, stats.topics.len() > 1) {
        let all: usize = stats.topics.iter().map(|t| t.messages).sum();
        out.push((
            format!("Busiest topic: {}", esc(&topic.name)),
            format!(
                "{} messages, {} of all",
                thousands(topic.messages as i64),
                share(topic.messages as i64, all as i64)
            ),
        ));
    }

    let folk = &stats.people;
    let quiet = folk.known_members.saturating_sub(folk.members_who_spoke);
    if folk.known_members > 0 && quiet > 0 {
        out.push((
            format!(
                "{} of {} members never posted",
                thousands(quiet as i64),
                thousands(folk.known_members as i64)
            ),
            "on the member list, and silent".to_string(),
        ));
    }
    out
}

fn pair(stats: &Stats, a: &tga_stats::Person, b: &tga_stats::Person) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let total = a.messages + b.messages;
    out.push((
        format!("{} writes {}", esc(&a.name), share(a.messages, total)),
        format!(
            "{} {} &#183; {} {}",
            esc(&a.name),
            thousands(a.messages),
            esc(&b.name),
            thousands(b.messages)
        ),
    ));

    // Reply speed each way, from the medians the conversation figures keep
    // for anybody with enough replies to have one.
    let median = |key: &str| {
        stats
            .conversation
            .fastest
            .iter()
            .chain(stats.conversation.slowest.iter())
            .find(|l| l.key == key)
            .map(|l| l.median)
    };
    if let (Some(ma), Some(mb)) = (median(&a.key), median(&b.key)) {
        let (fast, slow, f, s) = if ma <= mb {
            (a, b, ma, mb)
        } else {
            (b, a, mb, ma)
        };
        out.push((
            format!("{} answers faster", esc(&fast.name)),
            format!(
                "{} against {} for {}",
                esc(&duration(f as f64)),
                esc(&duration(s as f64)),
                esc(&slow.name)
            ),
        ));
    }

    let starts = |key: &str| {
        stats
            .conversation
            .starters
            .iter()
            .find(|s| s.key == key)
            .map_or(0, |s| s.count)
    };
    let (sa, sb) = (starts(&a.key), starts(&b.key));
    if sa + sb > 0 {
        let (who, n) = if sa >= sb { (a, sa) } else { (b, sb) };
        out.push((
            format!(
                "{} starts {} of conversations",
                esc(&who.name),
                share(n, sa + sb)
            ),
            format!("{} of {} after a silence", thousands(n), thousands(sa + sb)),
        ));
    }
    out.extend(busiest_time(stats));
    out
}

/// What every chat gets: its busiest day and its longest silence.
fn shared(stats: &Stats) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let act = &stats.activity;
    if act.busiest_day.messages > 0 {
        out.push((
            format!("Peak day: {}", esc(&pretty_date(&act.busiest_day.date))),
            format!("{} messages", thousands(act.busiest_day.messages)),
        ));
    }
    if let Some(gap) = act.quietest.as_ref().filter(|g| g.days >= 2) {
        out.push((
            format!("Longest silence: {} days", thousands(gap.days as i64)),
            format!(
                "{} &ndash; {}",
                esc(&pretty_date(&gap.from)),
                esc(&pretty_date(&gap.to))
            ),
        ));
    }
    out
}
