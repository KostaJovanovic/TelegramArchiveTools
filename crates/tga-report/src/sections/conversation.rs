//! Who answers whom, how fast, and who is in contact with whom.

use tga_stats::Stats;

use super::*;
use crate::charts::{self, esc, thousands};

pub fn conversation(stats: &Stats, names: &Names) -> String {
    let talk = &stats.conversation;
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

    let fast = if talk.fastest.is_empty() {
        String::new()
    } else {
        // Slowest first, so the bar length is the wait itself. Inverting it to
        // put the fastest on the longest bar reads better and lies: the mark
        // would no longer be the number printed beside it.
        let slowest: Vec<&tga_stats::Latency> = talk.fastest.iter().take(12).rev().collect();
        let rows: Vec<charts::BarRow> = slowest
            .iter()
            .map(|r| {
                let median = r.median as f64;
                (r.name.clone(), median, duration(median))
            })
            .collect();
        let bars = charts::bars_h(&rows, WIDTH, 22.0, 250.0, 64.0);
        let quickest = slowest[slowest.len() - 1];
        format!(
            "<h3>How long people wait before answering</h3>{bars}\
             <p class=\"caption\">Median gap between a message and their reply to it, \
             for anyone with five replies or more. Shorter is faster; {} is quickest \
             at {}.</p>",
            esc(&quickest.name),
            esc(&duration(quickest.median as f64))
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
        format!(
            "<h3>Who answers whom</h3><div class=\"split\"><div>{}</div><aside>{}\
             <p class=\"caption\">A row replies; a column is replied to. The \
             diagonal is greyed out &#8212; everyone answers themselves, and \
             counting it would make the loudest person their own closest \
             correspondent.</p></aside></div>",
            charts::matrix(&labels, &cells, WIDTH * 0.66, 176.0),
            charts::legend(5, "never", "most often")
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
        format!(
            "<h3>Who is in contact with whom</h3>{}\
             <p class=\"caption\">Replies and reactions pooled and drawn \
             undirected: an edge means these two are in contact, and its weight \
             is how often. Circle area is how much that person posted. {}</p>",
            charts::network(&drawn, &links, &labels, WIDTH, 620.0),
            if hidden != 0 {
                format!(
                    "The {hidden} least-connected people are left out; the \
                     matrix above has the numbers."
                )
            } else {
                String::new()
            }
        )
    };

    let starters = if talk.starters.is_empty() {
        String::new()
    } else {
        let rows: Vec<charts::BarRow> = talk
            .starters
            .iter()
            .take(10)
            .map(|st| (st.name.clone(), st.count as f64, thousands(st.count)))
            .collect();
        format!(
            "<h3>Who breaks the silence</h3>{}\
             <p class=\"caption\">First message after a gap of half an hour or more.</p>",
            charts::bars_h(&rows, WIDTH, 22.0, 210.0, 64.0)
        )
    };

    let orphans = talk.orphan_replies;
    let orphan = if orphans == 0 {
        String::new()
    } else {
        format!(
            "<p class=\"note\">{orphans} replies point at a message that is not \
             in this export, so they have no target in any figure above.</p>"
        )
    };

    format!(
        "{}<p class=\"lede\">A burst is talk with no gap longer than half an \
         hour, counted per topic &#8212; two topics running at once are two \
         conversations.</p>{stat_line}{fast}{starters}{matrix_html}{network_html}{orphan}</section>",
        head(
            "Conversation",
            &format!("{} bursts", thousands(talk.sessions as i64)),
            "talk"
        )
    )
}
