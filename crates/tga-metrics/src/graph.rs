//! Who talks to whom, laid out as a diagram.
//!
//! The layout is Fruchterman-Reingold, written out here rather than pulled in,
//! because the algorithm is thirty lines.
//!
//! **It is deterministic.** Nodes start on a circle in rank order and the
//! temperature schedule is fixed, so the same export always produces the same
//! picture — a report that reshuffles its own diagram between two runs of the
//! same data cannot be compared with itself, and no test can pin it.
//!
//! Edges are **undirected and pooled**: a reply from A to B and a reaction
//! from B to A are both "these two are in contact", and drawing them as two
//! arcs says twice as much as the data supports. Direction is still in the
//! matrix, which is the view that can carry it.
//!
//! *Deterministic per implementation* is not the same as *stable under a
//! rewrite*: 220 steps of a force layout amplify the last bit of `hypot`, so
//! these coordinates are presentation and never a figure. Nothing in the report
//! reads a number off them.

use std::collections::HashMap;

use tga_stats::{Edge, Graph, Link, Node};

use crate::util::Counter;

/// Above this the picture is a hairball and the matrix is the honest view.
pub const MAX_NODES: usize = 28;
pub const ITERATIONS: usize = 220;

pub fn build(
    reply_edges: &[Edge],
    reaction_edges: &[Edge],
    weights: &HashMap<String, i64>,
) -> Graph {
    let mut pooled: Counter<(String, String)> = Counter::new();
    for edge in reply_edges.iter().chain(reaction_edges.iter()) {
        let (a, b) = (edge.from.clone(), edge.to.clone());
        if !a.is_empty() && !b.is_empty() && a != b {
            let pair = if a <= b { (a, b) } else { (b, a) };
            pooled.add(pair, edge.count);
        }
    }
    if pooled.is_empty() {
        return Graph::default();
    }

    let mut degree: Counter<String> = Counter::new();
    for (a, b) in pooled.keys() {
        let n = pooled.get(&(a.clone(), b.clone()));
        degree.add(a.clone(), n);
        degree.add(b.clone(), n);
    }

    let mut ranked: Vec<String> = degree.keys().cloned().collect();
    ranked.sort_by(|x, y| {
        let dx = degree.get(x);
        let dy = degree.get(y);
        let wx = weights.get(x).copied().unwrap_or(0);
        let wy = weights.get(y).copied().unwrap_or(0);
        dy.cmp(&dx).then_with(|| wy.cmp(&wx)).then_with(|| x.cmp(y))
    });

    let keep: Vec<String> = ranked.iter().take(MAX_NODES).cloned().collect();
    let kept: std::collections::HashSet<&String> = keep.iter().collect();

    let mut edges: Vec<(String, String, i64)> = pooled
        .keys()
        .filter(|(a, b)| kept.contains(a) && kept.contains(b))
        .map(|(a, b)| {
            let n = pooled.get(&(a.clone(), b.clone()));
            (a.clone(), b.clone(), n)
        })
        .collect();
    // Stable, so ties keep the order they were pooled in.
    edges.sort_by_key(|edge| std::cmp::Reverse(edge.2));

    let positions = layout(&keep, &edges);
    let top_weight = keep
        .iter()
        .map(|k| weights.get(k).copied().unwrap_or(1))
        .max()
        .unwrap_or(1);

    let nodes: Vec<Node> = keep
        .iter()
        .map(|key| {
            let (x, y) = positions[key];
            let messages = weights.get(key).copied().unwrap_or(0);
            Node {
                key: key.clone(),
                x,
                y,
                degree: degree.get(key),
                messages,
                size: if top_weight != 0 {
                    messages as f64 / top_weight as f64
                } else {
                    0.0
                },
            }
        })
        .collect();

    Graph {
        nodes,
        edges: edges
            .iter()
            .map(|(a, b, n)| Link {
                a: a.clone(),
                b: b.clone(),
                weight: *n,
            })
            .collect(),
        hidden: ranked.len().saturating_sub(keep.len()),
    }
}

/// Fruchterman-Reingold on the unit square, seeded on a circle.
fn layout(keys: &[String], edges: &[(String, String, i64)]) -> HashMap<String, (f64, f64)> {
    layout_with(keys, edges, ITERATIONS)
}

/// The iteration count is a parameter only so a test can run a handful of
/// steps. Over 220 steps on a real graph the two implementations drift apart
/// on libm's last bit; over one or five they cannot, which is what makes the
/// port checkable at all.
fn layout_with(
    keys: &[String],
    edges: &[(String, String, i64)],
    iterations: usize,
) -> HashMap<String, (f64, f64)> {
    let count = keys.len();
    let mut pos: HashMap<String, (f64, f64)> = HashMap::new();
    if count == 1 {
        pos.insert(keys[0].clone(), (0.5, 0.5));
        return pos;
    }

    for (i, key) in keys.iter().enumerate() {
        let angle = 2.0 * std::f64::consts::PI * i as f64 / count as f64;
        pos.insert(
            key.clone(),
            (0.5 + 0.4 * angle.cos(), 0.5 + 0.4 * angle.sin()),
        );
    }

    let area = 1.0f64;
    let k = (area / count as f64).sqrt();
    let heaviest = edges.iter().map(|e| e.2).max().unwrap_or(1) as f64;
    let mut temp = 0.12f64;

    for _ in 0..iterations {
        let mut disp: HashMap<&String, (f64, f64)> = keys.iter().map(|k| (k, (0.0, 0.0))).collect();

        for i in 0..count {
            let (ax, ay) = pos[&keys[i]];
            for b in &keys[i + 1..] {
                let (bx, by) = pos[b];
                let (dx, dy) = (ax - bx, ay - by);
                let dist = nonzero(dx.hypot(dy));
                let force = (k * k) / dist;
                let (ux, uy) = (dx / dist, dy / dist);
                let a = &keys[i];
                let e = disp.get_mut(a).unwrap();
                e.0 += ux * force;
                e.1 += uy * force;
                let e = disp.get_mut(b).unwrap();
                e.0 -= ux * force;
                e.1 -= uy * force;
            }
        }

        for (a, b, weight) in edges {
            let (ax, ay) = pos[a];
            let (bx, by) = pos[b];
            let (dx, dy) = (ax - bx, ay - by);
            let dist = nonzero(dx.hypot(dy));
            let pull = (dist * dist) / k * (0.35 + 0.65 * *weight as f64 / heaviest);
            let (ux, uy) = (dx / dist, dy / dist);
            let e = disp.get_mut(a).unwrap();
            e.0 -= ux * pull;
            e.1 -= uy * pull;
            let e = disp.get_mut(b).unwrap();
            e.0 += ux * pull;
            e.1 += uy * pull;
        }

        for key in keys {
            let (dx, dy) = disp[key];
            let dist = nonzero(dx.hypot(dy));
            let step_len = dist.min(temp);
            let (x, y) = pos[key];
            let x = x + dx / dist * step_len;
            let y = y + dy / dist * step_len;
            pos.insert(key.clone(), (x.clamp(0.02, 0.98), y.clamp(0.02, 0.98)));
        }
        temp *= 0.985;
    }

    let xs: Vec<f64> = keys.iter().map(|k| pos[k].0).collect();
    let ys: Vec<f64> = keys.iter().map(|k| pos[k].1).collect();
    let (min_x, max_x) = span(&xs);
    let (min_y, max_y) = span(&ys);
    let span_x = nonzero_span(max_x - min_x);
    let span_y = nonzero_span(max_y - min_y);

    keys.iter()
        .map(|key| {
            let (x, y) = pos[key];
            (key.clone(), ((x - min_x) / span_x, (y - min_y) / span_y))
        })
        .collect()
}

/// Python's `x or 1e-4` — a zero distance would divide by nothing.
fn nonzero(value: f64) -> f64 {
    if value == 0.0 {
        1e-4
    } else {
        value
    }
}

/// Python's `(max - min) or 1.0`.
fn nonzero_span(value: f64) -> f64 {
    if value == 0.0 {
        1.0
    } else {
        value
    }
}

fn span(values: &[f64]) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for v in values {
        if *v < min {
            min = *v;
        }
        if *v > max {
            max = *v;
        }
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Vec<String>, Vec<(String, String, i64)>) {
        let keys = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        let edges = vec![
            ("a".to_string(), "b".to_string(), 5),
            ("b".to_string(), "c".to_string(), 2),
            ("a".to_string(), "d".to_string(), 1),
        ];
        (keys, edges)
    }

    /// Expected values produced by the Python `_layout` on the same input.
    ///
    /// This is the check that tells a porting bug apart from floating-point
    /// drift. On the 28-node real graph the two implementations disagree in
    /// the third decimal after 220 iterations, and there are only two possible
    /// reasons: the algorithm was mistranslated, or `math.hypot` and Rust's
    /// `f64::hypot` disagree in the last bit and 220 chaotic steps amplify it.
    /// A four-node graph cannot amplify anything, so agreement here settles it.
    fn check(iterations: usize, expected: &[(&str, f64, f64)]) {
        let (keys, edges) = fixture();
        let pos = layout_with(&keys, &edges, iterations);
        for (key, x, y) in expected {
            let (gx, gy) = pos[*key];
            assert!(
                (gx - x).abs() < 1e-12 && (gy - y).abs() < 1e-12,
                "{iterations} iterations, node {key}: got ({gx}, {gy}), python ({x}, {y})"
            );
        }
    }

    #[test]
    fn one_step_matches_python() {
        check(
            1,
            &[
                ("a", 1.0, 0.5825239448522472),
                ("b", 0.5806720922751878, 1.0),
                ("c", 0.0, 0.5481441112511826),
                ("d", 0.5360922473789768, 0.0),
            ],
        );
    }

    #[test]
    fn five_steps_match_python() {
        check(
            5,
            &[
                ("a", 1.0, 0.6141706675624516),
                ("b", 0.5604906276969381, 1.0),
                ("c", 0.0, 0.8300471723015362),
                ("d", 0.804201811181001, 0.0),
            ],
        );
    }

    #[test]
    fn the_full_schedule_matches_python_on_a_graph_too_small_to_diverge() {
        check(
            ITERATIONS,
            &[
                ("a", 1.0, 0.6416606564699688),
                ("b", 0.5927791731799454, 1.0),
                ("c", 0.0, 1.0),
                ("d", 1.0, 0.0),
            ],
        );
    }

    #[test]
    fn the_layout_is_deterministic() {
        // The stated property of this module. If it ever stops holding, the
        // report reshuffles its own diagram between two runs of one export.
        let (keys, edges) = fixture();
        assert_eq!(
            format!("{:?}", {
                let mut v: Vec<_> = layout_with(&keys, &edges, 40).into_iter().collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v
            }),
            format!("{:?}", {
                let mut v: Vec<_> = layout_with(&keys, &edges, 40).into_iter().collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v
            })
        );
    }
}
