//! Columns, horizontal bars, and the legend they share.

use super::*;

/// A column per slot, one hue, the extreme labelled and nothing else.
pub fn columns(
    labels: &[String],
    values: &[i64],
    width: f64,
    height: f64,
    label_every: usize,
    unit: &str,
    highlight: Option<usize>,
) -> String {
    if values.is_empty() {
        return frame(width, height, "", "", "");
    }
    // `max(values) or 1` used to stand in for an empty series, which turned a
    // genuine all-zero series into a top of 1 that no value equals — and the
    // peak lookup below then failed. An export with no participants.json has
    // exactly that shape: every month's arrivals is 0. A series with nothing in
    // it draws its baseline and its axis and no bars, which is the true
    // picture, rather than failing the whole report.
    let peak_value = *values.iter().max().expect("non-empty");
    let top = if peak_value == 0 { 1 } else { peak_value } as f64;
    // Headroom for the one value that gets a direct label. Without it the
    // tallest bar reaches the top of the box and its label is drawn outside,
    // where it survives only because the SVG does not clip.
    let headroom = 15.0;
    let plot = height - 18.0;
    let slot = width / values.len() as f64;
    let bar = (slot - BAR_GAP).clamp(1.0, MAX_BAR);
    let peak: Option<usize> = if peak_value != 0 {
        values.iter().position(|v| *v == peak_value)
    } else {
        None
    };
    let label_every = label_every.max(1);

    let mut parts = format!(
        "<line class=\"baseline\" x1=\"0\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
        num(plot),
        num(width),
        num(plot)
    );
    let axis_label = |index: usize| -> String {
        format!(
            "<text class=\"axis\" x=\"{}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
            num(index as f64 * slot + slot / 2.0),
            num(height - 5.0),
            esc(&labels[index])
        )
    };
    for (index, value) in values.iter().enumerate() {
        let x = index as f64 * slot + (slot - bar) / 2.0;
        let tall = (plot - headroom) * (*value as f64 / top);
        let cls = if Some(index) == peak || Some(index) == highlight {
            "col peak"
        } else {
            "col"
        };
        if *value <= 0 {
            // A zero-height rect is not a mark. It cannot be seen or hovered,
            // and leaving it in claims the chart drew something it did not.
            if index % label_every == 0 {
                parts.push_str(&axis_label(index));
            }
            continue;
        }
        let _ = write!(
            parts,
            "<rect class=\"{cls}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" \
             data-tip=\"{} &#183; {}{}\"/>",
            num(x),
            num(plot - tall),
            num(bar),
            num(tall.max(0.0)),
            num((bar / 2.0).min(4.0)),
            esc(&labels[index]),
            thousands(*value),
            esc(unit)
        );
        if index % label_every == 0 {
            parts.push_str(&axis_label(index));
        }
    }
    if let Some(peak) = peak {
        let _ = write!(
            parts,
            "<text class=\"value\" x=\"{}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
            num(peak as f64 * slot + slot / 2.0),
            num(plot - (plot - headroom) - 5.0),
            thousands(peak_value)
        );
    }
    frame(width, height, &parts, "columns", "")
}

/// One ranked row: label, value, and the text to print at the tip.
pub type BarRow = (String, f64, String);

/// Ranked horizontal bars: label, bar, value at the tip.
///
/// Horizontal because the labels are people's names and topic titles, which do
/// not fit under a column and must never be rotated to make them.
pub fn bars_h(
    rows: &[BarRow],
    width: f64,
    row_height: f64,
    label_width: f64,
    value_width: f64,
) -> String {
    if rows.is_empty() {
        return frame(width, row_height, "", "", "");
    }
    let top = {
        let biggest = rows.iter().map(|(_, v, _)| *v).fold(f64::MIN, f64::max);
        if biggest == 0.0 {
            1.0
        } else {
            biggest
        }
    };
    let plot = width - label_width - value_width;
    let height = row_height * rows.len() as f64;
    let bar = (row_height - 8.0).min(MAX_BAR);

    let mut parts = String::new();
    for (index, (label, value, note)) in rows.iter().enumerate() {
        let y = index as f64 * row_height;
        let length = plot * (*value / top);
        let shown = if note.is_empty() {
            thousands_f(*value)
        } else {
            note.clone()
        };
        let _ = write!(
            parts,
            "<text class=\"rowlabel\" x=\"{}\" y=\"{}\" text-anchor=\"end\" \
             data-tip=\"{} &#183; {}\">{}</text>",
            num(label_width - 10.0),
            num(y + row_height / 2.0 + 4.0),
            esc(label),
            esc(&shown),
            esc(&clip_default(label, label_width - 12.0))
        );
        let _ = write!(
            parts,
            "<rect class=\"bar\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" \
             data-tip=\"{} &#183; {}\"/>",
            num(label_width),
            num(y + (row_height - bar) / 2.0),
            num(length.max(1.0)),
            num(bar),
            num((bar / 2.0).min(4.0)),
            esc(label),
            esc(&shown)
        );
        let _ = write!(
            parts,
            "<text class=\"value\" x=\"{}\" y=\"{}\">{}</text>",
            num(label_width + length + 8.0),
            num(y + row_height / 2.0 + 4.0),
            esc(&shown)
        );
    }
    frame(width, height, &parts, "bars", "")
}
