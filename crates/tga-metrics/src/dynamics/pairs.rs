//! Reply edges collapsed onto unordered pairs.

use super::*;

/// Reply edges, collapsed onto unordered pairs.
///
/// The ranking key is `min(there, back)` rather than the total, and that choice
/// is the whole figure. A pair where one person sent 300 replies and got 2 back
/// has a large total and is not a correspondence; ranked on the smaller
/// direction it falls where it belongs, and `balance` says how lopsided it was.
pub(super) fn pairs(
    msgs: &[&Msg],
    export: &Export,
    index: &HashMap<i64, usize>,
    people: &People,
) -> Pairs {
    // Insertion-ordered, so the pair set below is built in a deterministic
    // order however the hashes fall. Same reasoning as `util::Counter`'s.
    let mut directed: Counter<(String, String)> = Counter::new();
    for msg in msgs {
        let Some(parent) = parent_of(msg, export, index) else {
            continue;
        };
        let (source, target) = (people.key_of(msg), people.key_of(parent));
        // A reply to yourself is dropped here for the reason `conversation`
        // drops it from the graph: everyone talks to themselves, and counting
        // it makes the loudest person their own closest correspondent.
        if source.is_empty() || target.is_empty() || source == target {
            continue;
        }
        directed.bump((source, target));
    }

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut ranked: Vec<(i64, i64, String, String)> = Vec::new();
    let mut mutual = 0i64;
    let mut one_way = 0i64;
    for (from, to) in directed.keys() {
        let pair = if from <= to {
            (from.clone(), to.clone())
        } else {
            (to.clone(), from.clone())
        };
        if !seen.insert(pair.clone()) {
            continue;
        }
        let there = directed.get(&(pair.0.clone(), pair.1.clone()));
        let back = directed.get(&(pair.1.clone(), pair.0.clone()));
        if there.min(back) > 0 {
            mutual += 1;
        } else {
            one_way += 1;
        }
        ranked.push((there.min(back), there + back, pair.0, pair.1));
    }
    // Total, down to the keys: two pairs with the same two counts must not
    // change places between runs, and the keys are the only thing left that
    // tells them apart.
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });

    let rows: Vec<Pair> = ranked
        .iter()
        .take(PAIRS_SHOWN)
        .map(|(both, total, a, b)| {
            let a_to_b = directed.get(&(a.clone(), b.clone()));
            let b_to_a = directed.get(&(b.clone(), a.clone()));
            let most = a_to_b.max(b_to_a);
            Pair {
                a_name: people.name_of(a),
                b_name: people.name_of(b),
                a: a.clone(),
                b: b.clone(),
                a_to_b,
                b_to_a,
                both: *both,
                replies: *total,
                // 1.0 is an even exchange, 0.0 is one person talking. Emitted
                // raw rather than rounded, like every other share in the dump.
                balance: if most > 0 {
                    *both as f64 / most as f64
                } else {
                    0.0
                },
            }
        })
        .collect();

    Pairs {
        shown: rows.len(),
        rows,
        mutual,
        one_way,
        directed: directed.len(),
    }
}
