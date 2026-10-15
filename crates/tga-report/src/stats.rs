//! The few things the writer needs that reading a field cannot give it.
//!
//! This module used to be fifteen accessors that dug values out of a
//! `serde_json::Value` with a fallback for every one. The shape has names now —
//! see `tga-stats` — so the fallbacks are gone with the guessing that needed
//! them, and what is left is two conversions and one formatter. The byte
//! formatter the metrics share lives in `tga-stats`, where both halves reach it.

use std::collections::BTreeMap;

use tga_stats::Sparse;

use crate::charts::Day;

/// A float the way the dump prints it: `12.0` keeps its trailing zero.
///
/// `avg_words` and `mean_words` are rounded to one decimal, so they are floats
/// and a reader expects to see one. Rust's `{}` writes `12` for `12.0`, which
/// reads as an integer and puts a whole column out of step with itself.
/// serde_json writes the shortest representation that round-trips, which keeps
/// the decimal, so the number is handed through its serialiser rather than
/// formatted by hand.
pub fn number(value: f64) -> String {
    serde_json::to_string(&value).unwrap_or_else(|_| "0".to_string())
}

/// A sparse `{day: count}` series, densified against the shared axis.
///
/// `per_day_by_person` and `per_day_by_topic` are stored sparse — dense they
/// would be one entry per person per day of the archive, which for a large
/// group is millions of zeroes nobody reads. The report draws each as a row on
/// the same axis as the whole-archive ribbon, so the zeroes are put back here,
/// at draw time, against that axis.
pub fn densify(sparse: Option<&Sparse>, axis: &[Day]) -> Vec<Day> {
    axis.iter()
        .map(|day| {
            let n = sparse
                .and_then(|counts| counts.get(&day.label))
                .copied()
                .unwrap_or(0);
            Day::new(day.label.clone(), n)
        })
        .collect()
}

/// Every value in a sparse-by-key branch, for building one shared scale.
pub fn every_count(branch: &BTreeMap<String, Sparse>) -> Vec<i64> {
    branch
        .values()
        .flat_map(|inner| inner.values().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rounded_float_keeps_its_trailing_zero() {
        assert_eq!(number(12.0), "12.0");
        assert_eq!(number(12.5), "12.5");
        assert_eq!(number(0.0), "0.0");
    }

    #[test]
    fn a_sparse_person_is_densified_against_the_shared_axis() {
        let axis: Vec<Day> = vec![
            Day::new("2025-01-01", 5),
            Day::new("2025-01-02", 0),
            Day::new("2025-01-03", 2),
        ];
        let sparse: Sparse = [("2025-01-03".to_string(), 2)].into_iter().collect();
        assert_eq!(
            densify(Some(&sparse), &axis),
            vec![
                Day::new("2025-01-01", 0),
                Day::new("2025-01-02", 0),
                Day::new("2025-01-03", 2),
            ]
        );
    }

    #[test]
    fn a_person_absent_from_the_branch_densifies_to_silence_not_to_nothing() {
        // A row of no cells and a row of zero cells are different pictures: the
        // second says "here is this person's row, and it is empty".
        let axis: Vec<Day> = vec![Day::new("2025-01-01", 5)];
        assert_eq!(densify(None, &axis), vec![Day::new("2025-01-01", 0)]);
    }
}
