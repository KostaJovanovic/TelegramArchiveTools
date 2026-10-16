//! Who answers whom, how fast, and who is in contact with whom.

use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

/// The width the matrix's names are a reading size at.
const MATRIX_MIN: f64 = 760.0;
/// The same for the network's labels.
const NETWORK_MIN: f64 = 860.0;

pub fn conversation(stats: &Stats, names: &Names) -> String {
    let talk = &stats.conversation;
    // The gap the metrics cut bursts on, as they recorded it. Spelled out here
    // as "half an hour" once, and true only until somebody changed the number.
    let gap = esc(&duration(talk.session_gap as f64));
    let stat_line = figures(&[
        (thousands(talk.replies), "replies"),
        (pct(talk.reply_share), "of messages are replies"),
        (
            esc(&duration(talk.latency_median as f64)),
            "median time to reply",
        ),
        (esc(&duration(talk.latency_p90 as f64)), "90th percentile"),
        (thousands(talk.sessions as i64), "bursts of talk"),
        (
            thousands(talk.session_median_messages),
            "messages in a typical burst",
        ),
    ]);
    let orphans = talk.orphan_replies;
    let method = format!(
        "A burst is talk with no gap longer than {gap}, counted per topic &#8212; \
         two topics running at once are two conversations. The time to reply is \
         the gap between a message and a reply to it.{}",
        if orphans == 0 {
            String::new()
        } else {
            format!(
                " {} replies point at a message that is not in this export, so \
                 they have no target in any figure here.",
                thousands(orphans)
            )
        }
    );
    let take = if talk.latency_counted > 0 {
        format!(
            "Half of all replies arrive within <b>{}</b>.",
            esc(&duration(talk.latency_median as f64))
        )
    } else {
        String::new()
    };
    let lede = panel("Replies and bursts", &take, &method, &stat_line);
    let count = format!("{} bursts", thousands(talk.sessions as i64));

    // A private chat's speeds and openers are in the side-by-side panel; the
    // rest of this section is a group's question.
    if is_duo(stats) {
        return format!("{}{lede}</section>", head("Conversation", &count, "talk"));
    }

    let fast = if talk.fastest.is_empty() {
        String::new()
    } else {
        // Slowest first, so the bar length is the wait itself. Inverting it to
        // put the fastest on the longest bar reads better and lies: the mark
        // would no longer be the number printed beside it.
        let slowest: Vec<&tga_stats::Latency> = talk.fastest.iter().take(12).rev().collect();
        let rows: Vec<charts::HBar> = slowest
            .iter()
            .map(|r| {
                let median = r.median as f64;
                charts::HBar::new(r.name.clone(), median, duration(median)).who(r.key.clone())
            })
            .collect();
        let quickest = slowest[slowest.len() - 1];
        panel(
            "How long people wait before answering",
            &format!(
                "<b>{}</b> answers quickest, typically within {}.",
                esc(&quickest.name),
                esc(&duration(quickest.median as f64))
            ),
            "The median gap between a message and this person's reply to it, for \
             anyone with five replies or more. Shorter is faster.",
            &charts::hbars(&rows, Some(rows.len() - 1)),
        )
    };

    let starters = if talk.starters.is_empty() {
        String::new()
    } else {
        let rows: Vec<charts::HBar> = talk
            .starters
            .iter()
            .take(10)
            .map(|st| {
                charts::HBar::new(st.name.clone(), st.count as f64, thousands(st.count))
                    .who(st.key.clone())
            })
            .collect();
        let all: i64 = talk.starters.iter().map(|s| s.count).sum();
        let first = &talk.starters[0];
        panel(
            "Who breaks the silence",
            &format!(
                "<b>{}</b> opens {} of conversations.",
                esc(&first.name),
                share(first.count, all)
            ),
            &format!("The first message after a gap of more than {gap}."),
            &charts::hbars(&rows, Some(0)),
        )
    };

    let edges = &talk.edges;
    let matrix_html = if edges.is_empty() {
        String::new()
    } else {
        // Insertion-ordered, because the sort below is stable and first-seen
        // order is what breaks a tie on volume.
        let mut order: Vec<String> = Vec::new();
        let mut volume: HashMap<String, i64> = HashMap::new();
        let mut bump = |key: &str, count: i64, order: &mut Vec<String>| {
            let entry = volume.entry(key.to_string()).or_insert_with(|| {
                order.push(key.to_string());
                0
            });
            *entry += count;
        };
        for edge in edges {
            bump(&edge.from, edge.count, &mut order);
            bump(&edge.to, edge.count, &mut order);
        }
        let mut keys = order.clone();
        keys.sort_by_key(|k| std::cmp::Reverse(volume[k]));
        keys.truncate(MATRIX_PEOPLE);

        let index: HashMap<&str, usize> = keys
            .iter()
            .enumerate()
            .map(|(n, k)| (k.as_str(), n))
            .collect();
        let mut cells = vec![vec![0i64; keys.len()]; keys.len()];
        for edge in edges {
            if let (Some(&from), Some(&to)) =
                (index.get(edge.from.as_str()), index.get(edge.to.as_str()))
            {
                cells[from][to] += edge.count;
            }
        }
        let labels: Vec<String> = keys.iter().map(|k| label_for(names, k)).collect();
        // The busiest pair off the diagonal, which is what the eye looks for.
        let busiest = edges
            .iter()
            .filter(|e| e.from != e.to)
            .max_by_key(|e| e.count);
        let take = busiest.map_or(String::new(), |e| {
            format!(
                "<b>{}</b> answers <b>{}</b> most: {} replies.",
                esc(&label_for(names, &e.from)),
                esc(&label_for(names, &e.to)),
                thousands(e.count)
            )
        });
        panel(
            "Who answers whom",
            &take,
            "A row replies; a column is replied to. The diagonal is greyed out \
             &#8212; everyone answers themselves, and counting it would make the \
             loudest person their own closest correspondent.",
            &(charts::scroll(
                &charts::matrix(&labels, &keys, &cells, WIDTH * 0.66, 176.0),
                MATRIX_MIN,
            ) + &charts::legend(5, "never", "most often")),
        )
    };

    let net = &stats.graph;
    let network_html = if net.nodes.is_empty() {
        String::new()
    } else {
        let drawn: Vec<charts::Node> = net
            .nodes
            .iter()
            .map(|n| charts::Node {
                key: n.key.clone(),
                x: n.x,
                y: n.y,
                size: n.size,
                messages: n.messages,
                degree: n.degree,
            })
            .collect();
        let links: Vec<charts::Link> = net
            .edges
            .iter()
            .map(|e| charts::Link {
                a: e.a.clone(),
                b: e.b.clone(),
                weight: e.weight,
            })
            .collect();
        let labels: Names = drawn
            .iter()
            .map(|n| (n.key.clone(), label_for(names, &n.key)))
            .collect();
        let hidden = net.hidden;
        // Neighbours counted from the drawn links: a node's `degree` is
        // weighted by how often, which is not a number of people.
        let mut neighbours: HashMap<&str, usize> = HashMap::new();
        for link in &links {
            *neighbours.entry(link.a.as_str()).or_default() += 1;
            *neighbours.entry(link.b.as_str()).or_default() += 1;
        }
        let hub = drawn
            .iter()
            .map(|n| (n, neighbours.get(n.key.as_str()).copied().unwrap_or(0)))
            .max_by_key(|(n, count)| (*count, n.messages));
        let take = hub
            .filter(|(_, count)| *count > 0)
            .map_or(String::new(), |(n, count)| {
                format!(
                    "<b>{}</b> is in contact with the most people drawn here: {}.",
                    esc(&label_for(names, &n.key)),
                    thousands(count as i64)
                )
            });
        panel(
            "Who is in contact with whom",
            &take,
            &format!(
                "Replies and reactions pooled and drawn undirected: an edge means \
                 these two are in contact, and its weight is how often. Circle area \
                 is how much that person posted.{}",
                if hidden != 0 {
                    format!(
                        " The {hidden} least-connected people are left out; the \
                         matrix has the numbers."
                    )
                } else {
                    String::new()
                }
            ),
            &charts::scroll(
                &charts::network(&drawn, &links, &labels, WIDTH, 620.0),
                NETWORK_MIN,
            ),
        )
    };

    format!(
        "{}{lede}<div class=\"cols2\">{fast}{starters}</div>{matrix_html}{network_html}</section>",
        head("Conversation", &count, "talk")
    )
}
