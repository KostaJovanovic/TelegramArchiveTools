//! Rank-based bins for a heatmap, and the cut points to print.

use super::*;

/// Rank-based bins for a heatmap, plus the cut points to print.
///
/// Chat activity is long-tailed: in the reference archive one day carries 557
/// messages and the median active day carries 19, so binning linearly on the
/// maximum puts **116 of 133 active days in the first shade** and the other
/// four go unused. The grid comes out one flat colour and says nothing.
///
/// Splitting on quantiles instead spends all five shades. The cost is that a
/// shade then means *rank* rather than *magnitude* — twice the colour is not
/// twice the messages — so the report prints the cut points beside the key and
/// every cell keeps its real number in the tooltip and in the table view.
/// Stating the edges is what makes this honest rather than merely prettier.
///
/// Zeroes are excluded from the split: a day nobody spoke is not the quiet end
/// of the scale, it is off the scale, and it gets the track colour.
#[derive(Debug, Clone)]
pub struct Quantiles {
    pub steps: usize,
    pub edges: Vec<i64>,
    pub top: i64,
}

impl Quantiles {
    pub fn new(values: impl IntoIterator<Item = i64>) -> Self {
        Self::with_steps(values, 5)
    }

    pub fn with_steps(values: impl IntoIterator<Item = i64>, steps: usize) -> Self {
        let mut ordered: Vec<i64> = values.into_iter().filter(|v| *v > 0).collect();
        ordered.sort_unstable();

        let top = ordered.last().copied().unwrap_or(0);
        let mut this = Quantiles {
            steps,
            edges: Vec::new(),
            top,
        };
        if ordered.is_empty() {
            return this;
        }

        let cuts = |source: &[i64]| -> Vec<i64> {
            let mut picked: Vec<i64> = (1..steps)
                .map(|i| source[(source.len() * i / steps).min(source.len() - 1)])
                .collect();
            picked.sort_unstable();
            picked.dedup();
            picked
        };

        let mut distinct = ordered.clone();
        distinct.dedup();

        if distinct.len() <= steps {
            // Few enough values that each can have its own shade. Splitting on
            // quantiles here would put four of the five cut points on the same
            // repeated value and waste the ramp again.
            this.edges = distinct[1..].to_vec();
            return this;
        }
        this.edges = cuts(&ordered);
        if this.edges.len() < steps - 1 {
            // Quantiles of the values collapsed onto a repeated low value — a
            // chat where most active days carry one or two messages does this.
            // Split the distinct values instead, which spends the ramp on the
            // range rather than on the mode.
            this.edges = cuts(&distinct);
        }
        this
    }

    /// Which shade a value lands on: `0` is the track, `1..=steps` the ramp.
    ///
    /// [`Self::var`] formats this; [`strip_compact`] groups by it, which is
    /// what lets a whole row of one shade become a single path.
    pub fn index(&self, value: i64) -> usize {
        if value <= 0 {
            return 0;
        }
        let mut step = 1;
        for edge in &self.edges {
            if value >= *edge {
                step += 1;
            }
        }
        step.min(self.steps)
    }

    pub fn var(&self, value: i64) -> String {
        match self.index(value) {
            0 => "var(--track)".to_string(),
            step => format!("var(--r{step})"),
        }
    }

    /// The cut points, as prose, so the key can be read as numbers.
    pub fn caption(&self, unit: &str) -> String {
        if self.edges.is_empty() {
            return String::new();
        }
        let cuts: Vec<String> = self.edges.iter().map(|e| thousands(*e)).collect();
        let tail = if unit.is_empty() {
            String::new()
        } else {
            format!(" {unit}")
        };
        format!(
            "Five shades, one per fifth of the range actually used; the cut points are {} and {}{tail}.",
            cuts.join(", "),
            thousands(self.top)
        )
    }
}
