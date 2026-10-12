//! SVG marks, drawn from Rust, themed from CSS.
//!
//! Two rules run through all of it.
//!
//! **Colour is a variable, never a hex.** Every fill is `var(--accent)` or a
//! ramp step `var(--r3)`, so the report's light/dark switch is a class on
//! `<html>` and not a re-render. Nothing here needs to know which theme is on.
//!
//! **Every mark grows from one baseline and nothing gets a second axis.** Two
//! measures on one plot with two scales invent a correlation the data does not
//! contain; where the report has two measures it draws two charts on the same
//! x-axis instead, which is the same comparison without the lie.
//!
//! Sizes follow the house specs: bars capped so the band keeps its air, a 2px
//! surface gap between neighbours, hairline solid gridlines one step off the
//! surface, and the value on the extreme rather than on every mark.

use std::collections::HashMap;
use std::fmt::Write as _;

use chrono::{Datelike, Days, NaiveDate};
use tga_notes::Event;

/// A bar narrower than this cannot be hovered or seen, so a series with more
/// points than the width allows is bucketed until each bar clears it.
pub const MIN_BAR: f64 = 3.0;
/// Never let a bar fill its whole band — the gap is what separates it from its
/// neighbour, and it is the surface showing through, not a stroke.
pub const BAR_GAP: f64 = 2.0;
pub const MAX_BAR: f64 = 24.0;

/// One day of the shared time axis.
///
/// The same [`tga_stats::Count`] the metrics emit, so a series arrives here
/// without being reshaped on the way. `.label` is the ISO date and `.n` is how
/// many messages landed on it.
pub type Day = tga_stats::Count;

mod bars;
mod frame;
mod grids;
mod scale;
mod text;
mod time;

pub use bars::*;
pub use frame::*;
pub use grids::*;
pub use scale::*;
pub use text::*;
pub use time::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn series(counts: &[i64]) -> Vec<Day> {
        let start = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        counts
            .iter()
            .enumerate()
            .map(|(i, n)| Day::new((start + Days::new(i as u64)).to_string(), *n))
            .collect()
    }

    #[test]
    fn escaping_does_not_double_escape_its_own_ampersands() {
        assert_eq!(
            esc("<a href='x'>&\"</a>"),
            "&lt;a href=&#39;x&#39;&gt;&amp;&quot;&lt;/a&gt;"
        );
    }

    #[test]
    fn a_coordinate_loses_its_trailing_zeros_but_never_becomes_empty() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.0), "0");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(1.2345), "1.234"); // half-even, as Python's format is
        assert_eq!(num(12.0625), "12.062");
        // Python: f"{-0.0001:.3f}" -> "-0.000" -> "-0". The point of the test is
        // that neither side produces "" or "-".
        assert_eq!(num(-0.0001), "-0");
    }

    #[test]
    fn a_group_separator_is_a_non_breaking_space() {
        // A plain space here puts "6" at the end of one line and "643" at the
        // start of the next, which is how a count becomes two numbers.
        assert_eq!(thousands(6_643), "6\u{a0}643");
        assert_eq!(thousands(333_582), "333\u{a0}582");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(-1_234), "-1\u{a0}234");
    }

    #[test]
    fn clipping_counts_characters_not_bytes() {
        // Every character here is two bytes. Clipping on bytes would cut one in
        // half and emit invalid UTF-8 into the middle of an SVG label.
        // 30px at a 6.05px advance is four characters of room, so three
        // survive and the fourth is the ellipsis.
        let name = "ћћћћћћћћћћ";
        assert_eq!(clip(name, 30.0, 6.05), "ћћћ…");
        assert_eq!(clip("short", 300.0, 6.05), "short");
    }

    #[test]
    fn clipping_never_goes_below_four_characters_of_room() {
        assert_eq!(clip("abcdefgh", 0.0, 6.05), "abc…");
    }

    #[test]
    fn a_series_is_bucketed_until_every_bar_can_be_hovered() {
        let (buckets, per) = bucket_days(&series(&[1; 1800]), 1120.0);
        assert_eq!(per, 5, "1800 days at 3px minimum across 1120px");
        assert_eq!(buckets.len(), 360);
        assert_eq!(buckets[0].2, 5, "each bucket sums its days");
        assert_eq!(buckets[0].0, "2025-01-01");
        assert_eq!(buckets[0].1, "2025-01-05");
    }

    #[test]
    fn a_short_series_is_not_bucketed_at_all() {
        let (buckets, per) = bucket_days(&series(&[1, 2, 3]), 1120.0);
        assert_eq!(per, 1);
        assert_eq!(buckets.len(), 3);
        assert_eq!(buckets[0].0, buckets[0].1, "one day, so first == last");
    }

    #[test]
    fn an_empty_series_buckets_to_nothing_rather_than_dividing_by_zero() {
        let (buckets, per) = bucket_days(&[], 1120.0);
        assert!(buckets.is_empty());
        assert_eq!(per, 1);
    }

    #[test]
    fn quantiles_spend_the_whole_ramp_on_a_long_tailed_series() {
        // The case the type exists for: one day at 557 and a median of 19.
        let mut values: Vec<i64> = (0..132).map(|i| 1 + (i % 40)).collect();
        values.push(557);
        let scale = Quantiles::new(values);
        let used: std::collections::BTreeSet<String> = (1..=557).map(|v| scale.var(v)).collect();
        assert_eq!(used.len(), 5, "all five shades are reachable");
        assert_eq!(scale.var(0), "var(--track)");
        assert_eq!(scale.var(557), "var(--r5)");
    }

    #[test]
    fn few_distinct_values_get_one_shade_each() {
        // Splitting on quantiles here would put four cut points on the same
        // repeated value and waste the ramp.
        let scale = Quantiles::new([1, 1, 1, 2, 2, 3]);
        assert_eq!(scale.edges, vec![2, 3]);
        assert_eq!(scale.var(1), "var(--r1)");
        assert_eq!(scale.var(2), "var(--r2)");
        assert_eq!(scale.var(3), "var(--r3)");
    }

    #[test]
    fn quantiles_collapsing_onto_the_mode_re_cut_on_the_distinct_values() {
        // A chat where most active days carry one or two messages: the
        // quantiles of the raw values are all 1, so the first cut yields a
        // single edge and the fallback splits the distinct values instead.
        let mut values = vec![1i64; 100];
        values.extend([2, 3, 4, 5, 9, 40]);
        let scale = Quantiles::new(values);
        assert!(
            scale.edges.len() >= 4,
            "the fallback should spend the ramp: {:?}",
            scale.edges
        );
    }

    #[test]
    fn an_all_zero_series_has_no_edges_and_no_caption() {
        let scale = Quantiles::new([0, 0, 0]);
        assert!(scale.edges.is_empty());
        assert_eq!(scale.caption("messages"), "");
        assert_eq!(scale.var(0), "var(--track)");
    }

    #[test]
    fn the_caption_states_the_cut_points_as_numbers() {
        let scale = Quantiles::new([1, 2, 3, 4]);
        let caption = scale.caption("messages");
        assert!(caption.starts_with("Five shades"));
        assert!(caption.ends_with("and 4 messages."));
    }

    #[test]
    fn a_zero_column_draws_its_label_but_not_a_rect() {
        // A zero-height rect cannot be seen or hovered, and leaving it in
        // claims the chart drew something it did not.
        let labels: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let svg = columns(&labels, &[0, 4], 200.0, 100.0, 1, "", None);
        assert_eq!(svg.matches("<rect").count(), 1);
        assert_eq!(svg.matches("class=\"axis\"").count(), 2);
    }

    #[test]
    fn an_all_zero_series_draws_its_axis_and_no_bars_rather_than_failing() {
        // An export with no participants.json has exactly this shape: every
        // month's arrivals is 0.
        let labels: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let svg = columns(&labels, &[0, 0, 0], 200.0, 100.0, 1, "", None);
        assert!(!svg.contains("<rect"));
        assert!(svg.contains("class=\"baseline\""));
        assert!(
            !svg.contains("class=\"value\""),
            "no peak, so no peak label"
        );
    }

    #[test]
    fn a_ribbon_shares_the_ceiling_it_is_given() {
        // The property the whole page rests on: rescaled per row, a person who
        // sent four messages and one who sent four hundred both draw full
        // height.
        let quiet = ribbon(&series(&[4]), 100.0, 100.0, Some(400), false, "");
        let loud = ribbon(&series(&[400]), 100.0, 100.0, Some(400), false, "");
        assert!(quiet.contains("height=\"1.5\""), "{quiet}");
        assert!(loud.contains("height=\"100\""), "{loud}");
    }

    #[test]
    fn a_ribbon_carries_no_tooltip_when_asked_not_to() {
        let svg = ribbon(&series(&[1, 2]), 100.0, 20.0, None, false, "mini");
        assert!(!svg.contains("data-tip"));
        assert!(svg.contains("class=\"chart ribbon mini\""));
    }

    #[test]
    fn a_year_axis_replaces_months_past_the_point_they_collide() {
        let long = series(&[1; 1200]);
        let svg = time_axis(&long, 1120.0, 16.0);
        assert!(svg.contains(">2025<"), "{svg}");
        assert!(svg.contains(">2028<"), "{svg}");
        assert!(!svg.contains("Jan 25"));
    }

    #[test]
    fn a_short_axis_labels_months_and_names_the_year_once() {
        let svg = time_axis(&series(&[1; 90]), 1120.0, 16.0);
        assert!(
            svg.contains(">Jan 25<"),
            "first mark carries the year: {svg}"
        );
        assert!(svg.contains(">Feb<"));
        assert!(svg.contains(">Mar<"));
    }

    #[test]
    fn the_calendar_gives_a_silent_day_the_track_and_not_the_palest_step() {
        let svg = calendar(&series(&[0, 5, 0]), 400.0, None, None);
        assert!(svg.contains("fill=\"var(--track)\""));
        assert!(svg.contains("data-tip=\"2025-01-01 &#183; 0\""));
    }

    #[test]
    fn a_matrix_greys_its_own_diagonal_rather_than_letting_it_set_the_scale() {
        let labels: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let svg = matrix(&labels, &[vec![900, 1], vec![2, 900]], 400.0, 176.0);
        assert_eq!(svg.matches("var(--self)").count(), 2);
        // 900 on the diagonal must not have set the top: the off-diagonal
        // values are 1 and 2, so 2 is the maximum shade.
        assert!(svg.contains("&#8594;"));
    }

    #[test]
    fn an_event_rail_with_no_events_draws_nothing_at_all() {
        assert_eq!(event_rail(&[], &series(&[1, 2]), 100.0, 34.0), "");
    }

    #[test]
    fn a_spanning_event_draws_a_bar_and_a_moment_draws_only_a_marker() {
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let base = Event {
            id: "e0".into(),
            start: day(2025, 1, 2),
            end: None,
            title: "x".into(),
            summary: String::new(),
            kind: "milestone".into(),
            topic: None,
            messages: vec![],
            confidence: String::new(),
            time: String::new(),
            weight: String::new(),
            who: vec![],
            tags: vec![],
        };
        let moment = event_rail(std::slice::from_ref(&base), &series(&[1; 10]), 100.0, 34.0);
        assert!(!moment.contains("ev-span"));
        assert!(moment.contains("ev-dot"));

        let span = Event {
            end: Some(day(2025, 1, 6)),
            ..base
        };
        let svg = event_rail(&[span], &series(&[1; 10]), 100.0, 34.0);
        assert!(svg.contains("ev-span"));
    }

    #[test]
    fn low_confidence_is_rendered_differently_rather_than_hidden() {
        let event = Event {
            id: "e0".into(),
            start: NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
            end: None,
            title: "x".into(),
            summary: String::new(),
            kind: "milestone".into(),
            topic: None,
            messages: vec![],
            confidence: "low".into(),
            time: String::new(),
            weight: String::new(),
            who: vec![],
            tags: vec![],
        };
        let svg = event_rail(&[event], &series(&[1; 10]), 100.0, 34.0);
        assert!(svg.contains("class=\"ev conf-low\""), "{svg}");
    }

    #[test]
    fn a_node_carries_its_count_as_area_not_as_radius() {
        // Four times the messages is twice the radius, which is four times the
        // ink for four times the data.
        let names = HashMap::new();
        let node = |key: &str, size: f64| Node {
            key: key.into(),
            x: 0.5,
            y: 0.5,
            size,
            messages: 10,
            degree: 2,
        };
        let svg = network(
            &[node("a", 1.0), node("b", 0.25)],
            &[],
            &names,
            400.0,
            400.0,
        );
        assert!(svg.contains("r=\"17\""), "{svg}");
        assert!(svg.contains("r=\"10.5\""), "{svg}");
    }

    #[test]
    fn a_bar_row_falls_back_to_its_own_number_when_given_no_note() {
        let rows = vec![("Ana".to_string(), 12.0, String::new())];
        let svg = bars_h(&rows, 400.0, 22.0, 150.0, 64.0);
        assert!(svg.contains(">12<"), "{svg}");
    }
}
