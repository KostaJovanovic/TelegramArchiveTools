//! The SVG box, and which ramp step a value lands on.

use super::*;

/// A responsive SVG box. Width is a viewBox unit, not a pixel.
///
/// Scaling is uniform (`meet`), never stretched: `none` would fit the box
/// exactly and squash every circle into an ellipse and every label into
/// condensed type on the way. The page's measure is close to the viewBox width,
/// so the type still lands near its intended size.
pub fn frame(width: f64, height: f64, body: &str, cls: &str, label: &str) -> String {
    format!(
        "<svg class=\"chart {cls}\" viewBox=\"0 0 {} {}\" \
         preserveAspectRatio=\"xMinYMin meet\" role=\"img\" aria-label=\"{}\">{body}</svg>",
        num(width),
        num(height),
        esc(label)
    )
}

/// Which ramp step a value lands on, linearly. Zero is the track.
///
/// Correct for a bar's colour, wrong for a heatmap — see [`Quantiles`].
pub fn ramp_var(value: f64, top: f64, steps: usize) -> String {
    if value <= 0.0 || top <= 0.0 {
        return "var(--track)".to_string();
    }
    let index = ((value / top * steps as f64) as usize + 1).min(steps);
    format!("var(--r{index})")
}
