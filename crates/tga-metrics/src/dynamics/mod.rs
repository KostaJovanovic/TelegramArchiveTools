//! How the group behaves *towards each other*, and how that changes over time.
//!
//! Five figures, all of which the counting elsewhere leaves unanswerable:
//!
//! * **`pairs`** — the graph section already says who talks *at* whom. A
//!   directed edge cannot tell a conversation from a broadcast: somebody who
//!   answers everyone and is answered by nobody has the same fan-out as the
//!   person at the centre of the group. Ranking a pair by `min(there, back)`
//!   asks the other question — who talks *with* whom — and the two lists are
//!   not the same list.
//! * **`answer`** — the median reply latency is one number for an archive that
//!   behaves nothing like itself at 04:00 and at 21:00. Cut by the hour the
//!   *parent* was posted, it answers the question somebody actually has: post
//!   this now, and how long until anyone sees it.
//! * **`tenure`** — `people.rows` carries a `first` and a `last` per person and
//!   the timeline draws their ribbon, but nothing states the fact those two
//!   dates carry: whether they are still here. A row with 4,000 messages whose
//!   last one was eight months ago is not a description of the group today.
//! * **`retention`** — `churn` counts arrivals and departures Telegram
//!   *announced*, which misses everyone who simply stopped typing. Month over
//!   month, of the people who spoke last month, how many spoke again.
//! * **`depth`** — reply chains. `conversation` measures the first hop; this
//!   measures how far a thread actually runs before it dies.
//!
//! Two inherited rules are load-bearing here and are not re-derived:
//!
//! *Latency is measured on epoch seconds*, never the wall clock, and the same
//! [`LATENCY_CAP`] applies — a reply three days later is somebody returning to
//! a thread, not a response time.
//!
//! *A reply to the message that opened a forum topic is not a reply.* The
//! reader drops those once, so the chains below are threads rather than the
//! whole topic hanging off its own first message.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate, Timelike};
use tga_read::{Export, Msg};
use tga_stats::{
    Answer, Count, Deepest, Depth, Dynamics, HourPick, Pair, Pairs, Retention, Tenure, TenureRow,
};

use crate::conversation::{median, LATENCY_CAP};
use crate::identity::People;
use crate::util::{round1, Counter};

/// How many correspondent pairs get a row. Past this the list stops ranking
/// anything — the tail is every pair that exchanged two messages once.
const PAIRS_SHOWN: usize = 20;

/// Replies needed in an hour before its median is quoted as the group's
/// behaviour. Below it, one slow afternoon *is* the median.
const ANSWER_MIN: i64 = 20;

/// Days since somebody's last message, and what to call the silence.
///
/// Stated as constants and reported in the output rather than hidden in a
/// comparison, so a reader who thinks a month is too generous can say so.
const DORMANT_ACTIVE: i64 = 30;
const DORMANT_FADING: i64 = 90;

/// Chain lengths are reported individually up to here and pooled above it.
const DEPTH_CAP: i64 = 8;

/// A chain longer than this is not a thread.
///
/// It exists as a stop rather than as a figure: `reply_to` comes out of a file,
/// and a file that points a message at its own descendant walks forever. The
/// cycle guard below catches the common shape; this catches the rest.
const MAX_CHAIN: usize = 10_000;

pub fn compute(export: &Export, people: &People) -> Dynamics {
    let msgs: Vec<&Msg> = export.said().collect();
    if msgs.is_empty() {
        return Dynamics {
            empty: true,
            ..Default::default()
        };
    }
    let index = export.by_id();

    Dynamics {
        empty: false,
        pairs: pairs(&msgs, export, &index, people),
        answer: answer(&msgs, export, &index),
        tenure: tenure(&msgs, people),
        retention: retention(&msgs, people),
        depth: depth(&msgs, export, &index),
    }
}

mod answer;
mod depth;
mod pairs;
mod retention;
mod tenure;

use answer::answer;
use depth::depth;
use pairs::pairs;
use retention::retention;
use tenure::tenure;

/// The message a reply is answering, if the export contains it.
///
/// An orphan — a reply whose parent is outside the export — is skipped rather
/// than counted as a root. `conversation` reports how many there are; nothing
/// here can say anything true about one.
fn parent_of<'a>(msg: &Msg, export: &'a Export, index: &HashMap<i64, usize>) -> Option<&'a Msg> {
    let at = *index.get(&msg.reply_to?)?;
    Some(&export.msgs[at])
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::NaiveDate;
    use tga_read::{Msg, Topic};

    fn at(day: u32, hour: u32, minute: u32) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(2025, 1, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    fn msg(id: i64, who: &str, when: chrono::NaiveDateTime, reply_to: Option<i64>) -> Msg {
        Msg {
            id,
            topic: 0,
            when,
            unix: when.and_utc().timestamp(),
            service: false,
            action: String::new(),
            sender: who.to_string(),
            name: who.to_string(),
            text: "hello".into(),
            chars: 5,
            words: 1,
            reply_to,
            forward_from: String::new(),
            forward_id: String::new(),
            media: String::new(),
            file: String::new(),
            file_name: String::new(),
            file_size: 0,
            media_saved: false,
            sticker_emoji: String::new(),
            emoji: Vec::new(),
            domains: Vec::new(),
            mentions: Vec::new(),
            hashtags: Vec::new(),
            reactions: Vec::new(),
            edited: false,
            members: Vec::new(),
            grouped: None,
        }
    }

    fn export(msgs: Vec<Msg>) -> Export {
        Export {
            name: "test".into(),
            topics: vec![Topic {
                index: 0,
                name: "General".into(),
                folder: Default::default(),
                chat_id: None,
                chat_type: String::new(),
                created: None,
                messages: msgs.len(),
                root: None,
            }],
            msgs,
            ..Default::default()
        }
    }

    /// Two people answering each other, and one person answering into the void.
    fn conversation() -> Export {
        export(vec![
            msg(1, "user1", at(1, 10, 0), None),
            msg(2, "user2", at(1, 10, 5), Some(1)),
            msg(3, "user1", at(1, 10, 9), Some(2)),
            msg(4, "user2", at(1, 10, 30), Some(3)),
            msg(5, "user3", at(1, 11, 0), Some(1)),
            msg(6, "user3", at(1, 11, 4), Some(1)),
        ])
    }

    #[test]
    fn a_pair_is_ranked_on_the_direction_that_answered_least() {
        let export = conversation();
        let people = People::new(&export);
        let out = compute(&export, &people);
        let rows = &out.pairs.rows;

        // user1<->user2 exchanged three replies, two one way and one the
        // other; user3 sent two and received none. The lopsided pair must not
        // outrank the correspondence just for having a similar total.
        assert_eq!(rows[0].both, 1);
        assert_eq!(rows[0].replies, 3);
        assert_eq!(rows[1].both, 0);
        assert_eq!(out.pairs.mutual, 1);
        assert_eq!(out.pairs.one_way, 1);
    }

    #[test]
    fn a_reply_to_yourself_is_not_a_correspondence() {
        let export = export(vec![
            msg(1, "user1", at(1, 10, 0), None),
            msg(2, "user1", at(1, 10, 1), Some(1)),
        ]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert!(out.pairs.rows.is_empty());
        assert_eq!(out.pairs.directed, 0);
    }

    #[test]
    fn latency_is_bucketed_by_the_hour_the_parent_was_posted() {
        let export = conversation();
        let people = People::new(&export);
        let out = compute(&export, &people);
        let counts = out.answer.counts;
        // Four replies answer messages posted at 10:00, one answers 10:09, and
        // none answers anything posted at 11:00 — the two 11:00 messages are
        // replies themselves, and a reply's own hour is not what this counts.
        assert_eq!(counts[10], 5);
        assert_eq!(counts[11], 0);
        // Well under ANSWER_MIN, so neither superlative is claimed.
        assert!(out.answer.fastest_hour.is_none());
    }

    #[test]
    fn dormancy_is_measured_against_the_archive_rather_than_today() {
        let export = export(vec![
            msg(1, "user1", at(1, 10, 0), None),
            msg(2, "user2", at(1, 10, 5), None),
            msg(3, "user1", at(31, 10, 0), None),
        ]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        let rows = &out.tenure.rows;
        assert_eq!(out.tenure.as_of, "2025-01-31");
        assert_eq!(rows[0].dormant_days, 0);
        assert_eq!(rows[0].span_days, 31);
        assert_eq!(rows[0].active_days, 2);
        assert_eq!(rows[1].dormant_days, 30);
        // 30 days is still inside DORMANT_ACTIVE, and the boundary is the
        // thing worth pinning: an off-by-one here silently reclassifies
        // everyone who posts monthly.
        assert_eq!(rows[1].status, "active");
        assert_eq!(out.tenure.active, 2);
    }

    #[test]
    fn a_silent_month_stays_on_the_axis() {
        let export = export(vec![
            msg(1, "user1", at(1, 10, 0), None),
            msg(2, "user2", at(2, 10, 0), None),
            // March, after a silent February.
            msg(
                3,
                "user1",
                NaiveDate::from_ymd_opt(2025, 3, 4)
                    .unwrap()
                    .and_hms_opt(9, 0, 0)
                    .unwrap(),
                None,
            ),
        ]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        let ret = &out.retention;
        assert_eq!(ret.months, ["2025-01", "2025-02", "2025-03"]);
        assert_eq!(ret.active, [2, 0, 1]);
        assert_eq!(ret.new, [2, 0, 0]);
        // Nobody returned in March, because February is what March is measured
        // against and February was empty. Collapsing the gap would have called
        // this a return.
        assert_eq!(ret.returning, [0, 0, 0]);
        assert_eq!(ret.lost, [0, 2, 0]);
    }

    #[test]
    fn depth_counts_the_chain_not_the_hop() {
        let export = conversation();
        let people = People::new(&export);
        let out = compute(&export, &people);
        let depth = &out.depth;
        // 1 <- 2 <- 3 <- 4 is a chain of four; 5 and 6 both sit at two.
        assert_eq!(depth.max, 4);
        assert_eq!(depth.chained, 5);
        assert_eq!(depth.longest.as_ref().expect("a deepest chain").id, 4);
        assert_eq!(depth.buckets[0], Count::new("2", 3));
        assert_eq!(depth.buckets[1], Count::new("3", 1));
        assert_eq!(depth.buckets[2], Count::new("4", 1));
    }

    #[test]
    fn a_reply_cycle_does_not_hang_the_walk() {
        // Nothing Telegram writes looks like this. A hand-edited events file or
        // a truncated export can, and a metric that never returns takes the
        // whole report with it.
        let export = export(vec![
            msg(1, "user1", at(1, 10, 0), Some(2)),
            msg(2, "user2", at(1, 10, 1), Some(1)),
        ]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert!(out.depth.max >= 1);
    }

    #[test]
    fn an_export_with_nothing_said_says_so() {
        let export = export(vec![]);
        let people = People::new(&export);
        let out = compute(&export, &people);
        assert!(out.empty);
        assert!(out.pairs.rows.is_empty());
        assert!(out.tenure.rows.is_empty());
    }
}
