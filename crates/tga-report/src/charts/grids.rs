//! The two-dimensional marks: heat grid, calendar, matrix, network.

use chrono::Datelike;

use super::time::{iso, month_abbrev};
use super::*;

/// Weekday against hour. Magnitude, so a single-hue ramp.
pub fn heatgrid(
    row_labels: &[String],
    col_labels: &[String],
    cells: &[Vec<i64>],
    width: f64,
    cell: f64,
    gutter: f64,
    scale: Option<&Quantiles>,
) -> String {
    if cells.is_empty() {
        return frame(width, cell, "", "", "");
    }
    let cols = cells[0].len();
    let size = cell.min((width - gutter) / cols as f64 - BAR_GAP);
    let owned;
    let scale = match scale {
        Some(scale) => scale,
        None => {
            owned = Quantiles::new(cells.iter().flatten().copied());
            &owned
        }
    };
    let height = cells.len() as f64 * (size + BAR_GAP) + 18.0;

    let mut parts = String::new();
    for (r, row) in cells.iter().enumerate() {
        let y = r as f64 * (size + BAR_GAP);
        let _ = write!(
            parts,
            "<text class=\"rowlabel\" x=\"{}\" y=\"{}\" text-anchor=\"end\">{}</text>",
            num(gutter - 10.0),
            num(y + size / 2.0 + 4.0),
            esc(&row_labels[r])
        );
        for (c, value) in row.iter().enumerate() {
            let x = gutter + c as f64 * (size + BAR_GAP);
            let _ = write!(
                parts,
                "<rect class=\"cell\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" \
                 fill=\"{}\" data-tip=\"{} {} &#183; {}\"/>",
                num(x),
                num(y),
                num(size),
                num(size),
                scale.var(*value),
                esc(&row_labels[r]),
                esc(&col_labels[c]),
                thousands(*value)
            );
        }
    }
    for (c, label) in col_labels.iter().enumerate() {
        if c % 3 != 0 {
            continue;
        }
        let x = gutter + c as f64 * (size + BAR_GAP) + size / 2.0;
        let _ = write!(
            parts,
            "<text class=\"axis\" x=\"{}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
            num(x),
            num(height - 4.0),
            esc(label)
        );
    }
    frame(width, height, &parts, "heat", "")
}

/// The same grid, but **each row scaled to its own maximum**.
///
/// Separate from [`heatgrid`] rather than a flag on it, because the two answer
/// different questions and mixing them silently would be the worst of both. A
/// shared scale answers "where in the archive is the noise", which is right for
/// weekday-against-hour where every cell is drawn from the same pile. Rows of
/// wildly different size need the other question — "what shape is *this* row's
/// day" — and a shared scale cannot ask it: in the reference archive the
/// largest topic carries 139,743 messages and the smallest 488, so on one scale
/// nine of the ten rows are solid track and the grid says only that General is
/// big, which the row of numbers beside it already said.
///
/// The cost is stated rather than hidden: a shade means *this row's* rank, so
/// two rows are comparable by profile and **not** by colour. The caller prints
/// that under the chart, and every cell keeps its real count in the tooltip.
pub fn heatrows(
    row_labels: &[String],
    col_labels: &[String],
    cells: &[Vec<i64>],
    width: f64,
    cell: f64,
    gutter: f64,
) -> String {
    if cells.is_empty() {
        return frame(width, cell, "", "", "");
    }
    let cols = cells[0].len();
    let size = cell.min((width - gutter) / cols as f64 - BAR_GAP);
    let height = cells.len() as f64 * (size + BAR_GAP) + 18.0;

    let mut parts = String::new();
    for (r, row) in cells.iter().enumerate() {
        // Built from this row alone. An all-zero row yields no edges and every
        // cell lands on the track, which is the honest drawing of a topic
        // nobody posted in -- not a row of the palest shade, which would read
        // as "quiet but alive".
        let scale = Quantiles::new(row.iter().copied());
        let y = r as f64 * (size + BAR_GAP);
        let _ = write!(
            parts,
            "<text class=\"rowlabel\" x=\"{}\" y=\"{}\" text-anchor=\"end\">{}</text>",
            num(gutter - 10.0),
            num(y + size / 2.0 + 4.0),
            esc(&row_labels[r])
        );
        for (c, value) in row.iter().enumerate() {
            let x = gutter + c as f64 * (size + BAR_GAP);
            // The row's own peak rides along in the tooltip. Without it a
            // reader comparing two bright cells has no way to learn that one is
            // 4,815 messages and the other is 32.
            let _ = write!(
                parts,
                "<rect class=\"cell\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" \
                 fill=\"{}\" data-tip=\"{} {} &#183; {} of {} at this row's peak\"/>",
                num(x),
                num(y),
                num(size),
                num(size),
                scale.var(*value),
                esc(&row_labels[r]),
                esc(&col_labels[c]),
                thousands(*value),
                thousands(scale.top)
            );
        }
    }
    for (c, label) in col_labels.iter().enumerate() {
        if c % 3 != 0 {
            continue;
        }
        let x = gutter + c as f64 * (size + BAR_GAP) + size / 2.0;
        let _ = write!(
            parts,
            "<text class=\"axis\" x=\"{}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
            num(x),
            num(height - 4.0),
            esc(label)
        );
    }
    frame(width, height, &parts, "heat", "")
}

/// One square per day, weeks as columns, running continuously.
///
/// Not split into a block per year. A year block is the shape everyone knows
/// from a contributions graph, and on an archive that starts in December it
/// produces a three-week stub above a full-width year, which reads as a
/// rendering fault rather than as December. Running the weeks straight through
/// keeps this on the same left-to-right time axis as every other ribbon on the
/// page, and the months are labelled along the top.
///
/// The empty square is the track rather than the ramp's palest step, so a day
/// nobody posted is visibly different from a day one person did.
pub fn calendar(
    series: &[Day],
    width: f64,
    cell: Option<f64>,
    scale: Option<&Quantiles>,
) -> String {
    if series.is_empty() {
        return frame(width, cell.unwrap_or(12.0), "", "", "");
    }
    let counts: HashMap<&str, i64> = series
        .iter()
        .map(|day| (day.label.as_str(), day.n))
        .collect();
    let owned;
    let scale = match scale {
        Some(scale) => scale,
        None => {
            owned = Quantiles::new(series.iter().map(|day| day.n));
            &owned
        }
    };
    let first = iso(&series[0].label);
    let last = iso(&series[series.len() - 1].label);
    let origin = first - Days::new(first.weekday().num_days_from_monday() as u64);
    let weeks = ((last - origin).num_days() / 7 + 1).max(1) as f64;

    let gutter = 34.0;
    let head = 16.0;
    let cell = cell.unwrap_or_else(|| ((width - gutter) / weeks - BAR_GAP).clamp(3.0, 18.0));
    let step = cell + if cell >= 6.0 { BAR_GAP } else { 1.0 };
    let height = head + 7.0 * step + 4.0;

    let mut parts = String::new();
    for (row, name) in ["Mon", "", "Wed", "", "Fri", "", "Sun"].iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        let _ = write!(
            parts,
            "<text class=\"axis\" x=\"{}\" y=\"{}\" text-anchor=\"end\">{name}</text>",
            num(gutter - 8.0),
            num(head + row as f64 * step + cell * 0.8)
        );
    }

    let mut day = first;
    while day <= last {
        let week = ((day - origin).num_days() / 7) as f64;
        let x = gutter + week * step;
        let y = head + day.weekday().num_days_from_monday() as f64 * step;
        let iso_day = day.to_string();
        let count = counts.get(iso_day.as_str()).copied().unwrap_or(0);
        let _ = write!(
            parts,
            "<rect class=\"cell\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" \
             fill=\"{}\" data-tip=\"{iso_day} &#183; {}\"/>",
            num(x),
            num(y),
            num(cell),
            num(cell),
            scale.var(count),
            thousands(count)
        );
        day = match day.checked_add_days(Days::new(1)) {
            Some(next) => next,
            None => break,
        };
    }

    // Month labels along the top, thinned until they stop colliding.
    let every = ((46.0 / (step * 4.34).max(1.0)) as usize).max(1);
    let (mut year, mut month) = (first.year(), first.month());
    let mut index = 0usize;
    while let Some(mark) = NaiveDate::from_ymd_opt(year, month, 1) {
        if mark > last {
            break;
        }
        if mark >= origin && index.is_multiple_of(every) {
            let week = ((mark - origin).num_days() / 7) as f64;
            let mut label = month_abbrev(mark.month()).to_string();
            if mark.month() == 1 || index == 0 {
                let _ = write!(label, " {:02}", mark.year().rem_euclid(100));
            }
            let _ = write!(
                parts,
                "<text class=\"axis\" x=\"{}\" y=\"10\">{}</text>",
                num(gutter + week * step),
                esc(&label)
            );
        }
        index += 1;
        month += 1;
        if month > 12 {
            year += 1;
            month = 1;
        }
    }

    frame(width, height, &parts, "calendar", "Messages per day")
}

/// Who replies to whom: rows answer, columns are answered.
pub fn matrix(labels: &[String], cells: &[Vec<i64>], width: f64, gutter: f64) -> String {
    if cells.is_empty() {
        return frame(width, 10.0, "", "", "");
    }
    let size = ((width - gutter) / labels.len().max(1) as f64 - BAR_GAP).clamp(8.0, 22.0);
    // Off-diagonal only: the diagonal is drawn as a neutral and must not be
    // allowed to set the scale for everything else.
    let scale = Quantiles::new(cells.iter().enumerate().flat_map(|(r, row)| {
        row.iter()
            .enumerate()
            .filter_map(move |(c, v)| if r != c { Some(*v) } else { None })
    }));
    let height = labels.len() as f64 * (size + BAR_GAP) + gutter * 0.55;

    let mut parts = String::new();
    for (r, row) in cells.iter().enumerate() {
        let y = r as f64 * (size + BAR_GAP);
        let _ = write!(
            parts,
            "<text class=\"rowlabel\" x=\"{}\" y=\"{}\" text-anchor=\"end\" data-tip=\"{}\">{}</text>",
            num(gutter - 10.0),
            num(y + size / 2.0 + 4.0),
            esc(&labels[r]),
            esc(&clip_default(&labels[r], gutter - 12.0))
        );
        for (c, value) in row.iter().enumerate() {
            let x = gutter + c as f64 * (size + BAR_GAP);
            let fill = if r == c {
                "var(--self)".to_string()
            } else {
                scale.var(*value)
            };
            let _ = write!(
                parts,
                "<rect class=\"cell\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" \
                 fill=\"{fill}\" data-tip=\"{} &#8594; {} &#183; {}\"/>",
                num(x),
                num(y),
                num(size),
                num(size),
                esc(&labels[r]),
                esc(&labels[c]),
                thousands(*value)
            );
        }
    }
    for (c, label) in labels.iter().enumerate() {
        let x = gutter + c as f64 * (size + BAR_GAP) + size / 2.0;
        let y = labels.len() as f64 * (size + BAR_GAP) + 8.0;
        let _ = write!(
            parts,
            "<text class=\"axis colhead\" x=\"{}\" y=\"{}\" transform=\"rotate(45 {} {})\" \
             data-tip=\"{}\">{}</text>",
            num(x),
            num(y),
            num(x),
            num(y),
            esc(label),
            esc(&clip(label, gutter * 0.72, 5.2))
        );
    }
    frame(width, height, &parts, "matrix", "")
}

/// One node of the interaction graph, as `graph::build` emits it.
pub struct Node {
    pub key: String,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub messages: i64,
    pub degree: i64,
}

/// One pooled, undirected edge.
pub struct Link {
    pub a: String,
    pub b: String,
    pub weight: i64,
}

/// The interaction graph. Undirected, pooled, deterministic.
///
/// Edge weight is opacity and width; node area — not radius — carries the
/// message count, because a reader compares blobs by area and doubling the
/// radius quadruples the ink for twice the data.
pub fn network(
    nodes: &[Node],
    edges: &[Link],
    names: &HashMap<String, String>,
    width: f64,
    height: f64,
) -> String {
    if nodes.is_empty() {
        return frame(width, height, "", "", "");
    }
    let pad = 44.0;
    let (inner_w, inner_h) = (width - pad * 2.0, height - pad * 2.0);
    let place: HashMap<&str, (f64, f64)> = nodes
        .iter()
        .map(|n| (n.key.as_str(), (pad + n.x * inner_w, pad + n.y * inner_h)))
        .collect();
    let heaviest = {
        let top = edges.iter().map(|e| e.weight).max().unwrap_or(1);
        if top == 0 {
            1
        } else {
            top
        }
    } as f64;

    let mut parts = String::new();
    for edge in edges {
        let (Some((x1, y1)), Some((x2, y2))) = (
            place.get(edge.a.as_str()).copied(),
            place.get(edge.b.as_str()).copied(),
        ) else {
            continue;
        };
        let share = edge.weight as f64 / heaviest;
        let _ = write!(
            parts,
            "<line class=\"link\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" \
             stroke-width=\"{}\" opacity=\"{}\"/>",
            num(x1),
            num(y1),
            num(x2),
            num(y2),
            num(0.6 + 2.4 * share),
            num(0.18 + 0.5 * share)
        );
    }
    for node in nodes {
        let (x, y) = place[node.key.as_str()];
        let radius = 4.0 + 13.0 * node.size.max(0.0).sqrt();
        // Truncated hard: at this density a full name is a collision, and the
        // tooltip carries the whole string anyway.
        let full = names
            .get(&node.key)
            .cloned()
            .unwrap_or_else(|| node.key.clone());
        let label = clip(&full, 104.0, 5.4);
        let _ = write!(
            parts,
            "<g class=\"node\" data-tip=\"{} &#183; {} messages, {} exchanges\">\
             <circle class=\"dot\" cx=\"{}\" cy=\"{}\" r=\"{}\"/>\
             <text class=\"nodelabel\" x=\"{}\" y=\"{}\" text-anchor=\"middle\">{}</text></g>",
            esc(&label),
            thousands(node.messages),
            thousands(node.degree),
            num(x),
            num(y),
            num(radius),
            num(x),
            num(y - radius - 6.0),
            esc(&label)
        );
    }
    frame(width, height, &parts, "network", "Interaction graph")
}

/// The ramp's key. A single-series chart needs none; a ramp always does.
pub fn legend(steps: usize, low: &str, high: &str) -> String {
    let swatches: String = (0..steps)
        .map(|i| format!("<i style=\"background:var(--r{})\"></i>", i + 1))
        .collect();
    format!(
        "<p class=\"legend\"><span>{}</span><i class=\"swatch-track\"></i>{swatches}<span>{}</span></p>",
        esc(low),
        esc(high)
    )
}
