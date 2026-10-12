//! Reply-chain length: how far a thread runs before it dies.

use super::*;

/// Reply-chain length: how many messages deep a thread got.
///
/// Depth 1 is every message nobody was answering, so it is most of the archive
/// and says nothing; the distribution below starts at 2, where a chain begins.
///
/// The walk is iterative rather than recursive on purpose. A chain is bounded
/// by nothing but the file, and 300,000 stack frames is a crash rather than a
/// wrong number — which is the worse failure of the two.
pub(super) fn depth(msgs: &[&Msg], export: &Export, index: &HashMap<i64, usize>) -> Depth {
    let mut known: HashMap<i64, i64> = HashMap::new();
    for msg in &export.msgs {
        if known.contains_key(&msg.id) {
            continue;
        }
        let mut chain: Vec<i64> = Vec::new();
        let mut on_stack: HashSet<i64> = HashSet::new();
        let mut current = msg;
        let base = loop {
            if let Some(&settled) = known.get(&current.id) {
                break settled;
            }
            // A file can point a message at its own descendant. Neither of
            // these guards produces a figure; they stop a walk that would not
            // return, and the chain built so far is still counted from 0.
            if !on_stack.insert(current.id) || chain.len() >= MAX_CHAIN {
                break 0;
            }
            chain.push(current.id);
            match current.reply_to.and_then(|parent| index.get(&parent)) {
                Some(&at) => current = &export.msgs[at],
                None => break 0,
            }
        };
        for (step, id) in chain.iter().rev().enumerate() {
            known.insert(*id, base + step as i64 + 1);
        }
    }

    let mut counts: Counter<i64> = Counter::new();
    let mut lengths: Vec<i64> = Vec::new();
    let mut deepest: Option<&Msg> = None;
    for msg in msgs {
        let at = known.get(&msg.id).copied().unwrap_or(1);
        if at < 2 {
            continue;
        }
        lengths.push(at);
        counts.bump(at.min(DEPTH_CAP + 1));
        // Ties go to the lower id, which is the rule `extras::superlatives`
        // uses for every other record on the page.
        deepest = match deepest {
            Some(best) if known.get(&best.id).copied().unwrap_or(1) >= at => Some(best),
            _ => Some(msg),
        };
    }

    let mut buckets: Vec<Count> = Vec::new();
    for at in 2..=DEPTH_CAP + 1 {
        let label = if at > DEPTH_CAP {
            format!("{DEPTH_CAP}+")
        } else {
            at.to_string()
        };
        buckets.push(Count::new(label, counts.get(&at)));
    }

    let longest = deepest.map(|msg| Deepest {
        id: msg.id,
        topic: msg.topic,
        date: crate::util::stamp_minutes(&msg.when),
        messages: known.get(&msg.id).copied().unwrap_or(1),
    });

    Depth {
        buckets,
        cap: DEPTH_CAP,
        chained: lengths.len(),
        max: lengths.iter().copied().max().unwrap_or(0),
        median: if lengths.is_empty() {
            0
        } else {
            median(&lengths).trunc() as i64
        },
        mean: if lengths.is_empty() {
            0.0
        } else {
            round1(lengths.iter().sum::<i64>() as f64 / lengths.len() as f64)
        },
        longest,
    }
}
