//! Everything drawn on the shared time axis: ribbons, strips, rails.

use super::*;

/// One bar of the shared axis: first day, last day, total.
pub type Bucket = (String, String, i64);

/// Group a daily series until each bar is at least [`MIN_BAR`] wide.
///
/// Returns `(buckets, days_per_bucket)`. A five-year archive is 1,800 days and
/// no screen has 1,800 hoverable columns; bucketing is honest about that, and
/// the report says how wide a bar is.
pub fn bucket_days(series: &[Day], width: f64) -> (Vec<Bucket>, usize) {
    if series.is_empty() {
        return (Vec::new(), 1);
    }
    let per = if width > 0.0 {
        ((series.len() as f64 * MIN_BAR / width).ceil() as usize).max(1)
    } else {
        1
    };
    if per == 1 {
        return (
            series
                .iter()
                .map(|day| (day.label.clone(), day.label.clone(), day.n))
                .collect(),
            1,
        );
    }
    let mut out = Vec::with_capacity(series.len() / per + 1);
    for chunk in series.chunks(per) {
        out.push((
            chunk[0].label.clone(),
            chunk[chunk.len() - 1].label.clone(),
            chunk.iter().map(|day| day.n).sum(),
        ));
    }
    (out, per)
}

fn bucket_label(first: &str, last: &str) -> String {
    if first == last {
        first.to_string()
    } else {
        format!("{first} to {last}")
    }
}

/// Every bucket's label for a series drawn at `width`, for the shared axis.
///
/// [`strip_compact`] leaves these out of its cells; they are written into the
/// document once and every row's tooltip is composed from them. Emitting them
/// per cell is what made the presence rows 64% of the file.
pub fn bucket_labels(series: &[Day], width: f64) -> Vec<String> {
    bucket_days(series, width)
        .0
        .iter()
        .map(|(first, last, _)| bucket_label(first, last))
        .collect()
}

/// Messages per day as columns on a shared time axis.
///
/// `top` is the value that reaches full height. Passing the whole archive's
/// maximum into every ribbon is what makes a stack of them comparable —
/// rescaled per row, a person who sent four messages and a person who sent four
/// hundred both draw a full-height bar.
pub fn ribbon(
    series: &[Day],
    width: f64,
    height: f64,
    top: Option<i64>,
    tips: bool,
    cls: &str,
) -> String {
    let (buckets, per) = bucket_days(series, width);
    if buckets.is_empty() {
        return frame(width, height, "", "", "");
    }
    let ceiling = top
        .unwrap_or_else(|| buckets.iter().map(|(_, _, c)| *c).max().unwrap_or(1))
        .max(1) as f64;
    let slot = width / buckets.len() as f64;
    let gap = if slot > MIN_BAR + BAR_GAP {
        BAR_GAP
    } else {
        0.0
    };
    let bar = (slot - gap).clamp(1.0, MAX_BAR);

    let mut parts = format!(
        "<rect class=\"ribbon-bed\" x=\"0\" y=\"{}\" width=\"{}\" height=\"1\"/>",
        num(height - 1.0),
        num(width)
    );
    for (index, (first, last, count)) in buckets.iter().enumerate() {
        if *count <= 0 {
            continue;
        }
        let tall = (height * (*count as f64 / ceiling).min(1.0)).max(1.5);
        let x = index as f64 * slot + (slot - bar) / 2.0;
        let radius = (bar / 2.0).min(2.0);
        let tip = if tips {
            format!(
                " data-tip=\"{} &#183; {}\"",
                esc(&bucket_label(first, last)),
                thousands(*count)
            )
        } else {
            String::new()
        };
        let _ = write!(
            parts,
            "<rect class=\"ribbon-bar\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\"{tip}/>",
            num(x),
            num(height - tall),
            num(bar),
            num(tall),
            num(radius)
        );
    }
    let cls = format!("ribbon {cls}");
    let label = if per == 1 {
        "Messages per day".to_string()
    } else {
        format!("Messages per {per} days")
    };
    frame(width, height, &parts, cls.trim(), &label)
}

/// One person's activity as a heat row on the shared time axis.
///
/// A row of columns scaled to a shared ceiling is the right mark for the
/// archive as a whole and the wrong one for a table of forty people: with one
/// person on 897 messages and the thirtieth on 35, every row past the top few
/// draws as a scatter of 1px dots and reads as a rendering fault.
///
/// Shading the row instead keeps both halves of the question answerable. *Was
/// this person here* is presence, which is visible at any volume, and *how loud
/// were they* is the shade, on a scale shared by every row — so the rows still
/// compare, which was the point of not rescaling them.
///
/// Bucketed with [`bucket_days`] exactly as the hero ribbon is, so a column here
/// sits under the same days as the column above it.
///
/// It is drawn as one path per shade rather than one rect per cell.
///
/// **This is the whole size budget.** Drawn the obvious way — a `<rect>` per
/// cell, each carrying its own `fill` and its own `data-tip` — the KRGM
/// archive's 260 rows (250 people and 10 topics) came to 969 KB of a 1,519 KB
/// report: 64% of the file, against 275 KB for the embedded type and 275 KB for
/// everything else on the page. Nothing else in the document was worth
/// optimising until this was.
///
/// Three things go, and none of them is a fact:
///
/// * **The per-cell `fill`** becomes a class. `.strip .c3{fill:var(--r3)}` is
///   stated once for the document instead of 8,000 times in it.
/// * **The per-cell `<rect>`** becomes a subpath. Cells sharing a shade share
///   one element, so a row is at most five elements however many buckets it
///   has, and a run of equal-shaded days costs 21 characters instead of 128.
/// * **The per-cell `data-tip`** becomes one `data-n` on the row. Every bucket
///   is the same width, so the pointer's x *is* the bucket index; the tooltip
///   is composed from that and the shared axis labels rather than repeated into
///   every cell. It also improves the thing it shrinks — hover now answers
///   anywhere along the row, including over the silent days, where before there
///   was no element to hover at all.
///
/// **The chart is still entirely server-side**, which is the rule that matters:
/// with scripting off the row draws exactly as it does with scripting on, and
/// only the tooltip is missing — and the tooltip was never anything but JS.
///
/// `data-n` is the non-zero buckets as `index:count` pairs. Dense would be one
/// entry per bucket per row, and these rows are mostly silence.
pub fn strip_compact(
    series: &[Day],
    width: f64,
    height: f64,
    scale: &Quantiles,
    cls: &str,
) -> String {
    let (buckets, _) = bucket_days(series, width);
    if buckets.is_empty() {
        return frame(width, height, "", "", "");
    }
    let slot = width / buckets.len() as f64;
    let bar = (slot - 0.5).max(1.0);

    // One path per ramp step. Index 0 is the track and is not drawn: a day
    // nobody spoke is the absence of a mark, not a pale one.
    let mut shades: Vec<String> = vec![String::new(); scale.steps + 1];
    let mut counts: Vec<String> = Vec::new();
    for (index, (_, _, count)) in buckets.iter().enumerate() {
        if *count <= 0 {
            continue;
        }
        let shade = scale.index(*count);
        // `M<x> 0h<w>v<h>h-<w>z` — the rect, as a subpath. Absolute move, then
        // relative, because relative is shorter and the move already fixed the
        // origin.
        let _ = write!(
            shades[shade],
            "M{} 0h{}v{}h-{}z",
            num(index as f64 * slot),
            num(bar),
            num(height),
            num(bar)
        );
        counts.push(format!("{index}:{count}"));
    }

    let mut parts = String::new();
    for (shade, data) in shades.iter().enumerate() {
        if shade == 0 || data.is_empty() {
            continue;
        }
        let _ = write!(parts, "<path class=\"c{shade}\" d=\"{data}\"/>");
    }

    let cls = format!("strip {cls}");
    format!(
        "<svg class=\"chart {}\" viewBox=\"0 0 {} {}\" preserveAspectRatio=\"xMinYMin meet\" \
         role=\"img\" aria-label=\"Activity over time\" data-n=\"{}\" data-b=\"{}\">{parts}</svg>",
        cls.trim(),
        num(width),
        num(height),
        counts.join(","),
        buckets.len()
    )
}

pub(super) fn iso(day: &str) -> NaiveDate {
    NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap_or_default()
}

/// The three-letter month, in English, as `strftime("%b")` gives it.
pub(super) fn month_abbrev(month: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    NAMES[(month as usize - 1).min(11)]
}

/// Month or year ticks under the spine, thinned to what fits.
pub fn time_axis(series: &[Day], width: f64, height: f64) -> String {
    if series.is_empty() {
        return frame(width, height, "", "", "");
    }
    let first = iso(&series[0].label);
    let last = iso(&series[series.len() - 1].label);
    let span = ((last - first).num_days()).max(1);

    let mut marks: Vec<(NaiveDate, String)> = Vec::new();
    if span > 900 {
        for year in first.year()..=last.year() {
            if let Some(day) = NaiveDate::from_ymd_opt(year, 1, 1) {
                marks.push((day, year.to_string()));
            }
        }
    } else {
        let every = if span <= 200 {
            1
        } else if span <= 500 {
            2
        } else {
            3
        };
        let (mut year, mut month) = (first.year(), first.month());
        let mut step = 0usize;
        while let Some(mark) = NaiveDate::from_ymd_opt(year, month, 1) {
            if mark > last {
                break;
            }
            if step.is_multiple_of(every) {
                let mut label = month_abbrev(mark.month()).to_string();
                // The year is worth stating at January and at the very first
                // mark, and nowhere else — repeating it on every tick is the
                // repeated chrome the design exists to remove.
                if mark.month() == 1 || marks.is_empty() {
                    let _ = write!(label, " {:02}", mark.year().rem_euclid(100));
                }
                marks.push((mark, label));
            }
            step += 1;
            month += 1;
            if month > 12 {
                year += 1;
                month = 1;
            }
        }
    }

    let mut parts = String::new();
    for (when, label) in marks {
        if when < first {
            continue;
        }
        let x = (when - first).num_days() as f64 / span as f64 * width;
        let _ = write!(
            parts,
            "<line class=\"tick\" x1=\"{}\" y1=\"0\" x2=\"{}\" y2=\"4\"/>\
             <text class=\"axis\" x=\"{}\" y=\"13\">{}</text>",
            num(x),
            num(x),
            num((x + 4.0).min(width - 1.0)),
            esc(&label)
        );
    }
    frame(width, height, &parts, "axis-strip", "")
}

/// Event markers pinned to the same axis as the ribbon below them.
///
/// A span draws as a bar with a marker at its start; a moment draws as a marker
/// alone. The marker is 9px so it can be hit, and it carries the surface ring
/// that keeps it legible where two events nearly collide.
pub fn event_rail(events: &[Event], series: &[Day], width: f64, height: f64) -> String {
    rail(events, series, width, height, &[])
}

/// The four marker shapes, in the order kinds are first seen.
///
/// **Shape, not hue, and that is not a preference.** `_timeline` colour-codes
/// its four note kinds green/blue/pink/orange, and its own README records why
/// that failed: normal-vision ΔE 19.3 against CVD ΔE 6.9, which forced
/// shape-coding regardless. Once the shapes carry the distinction the colour is
/// redundant — and this design has one hue to spend, which magnitude has a
/// better claim on. Past four kinds the shapes repeat, and the filter row and
/// the card both name the kind in words, so nothing rests on the shape alone.
const SHAPES: usize = 4;

/// The marker for one kind, drawn at the origin so the group can place it.
fn marker(shape: usize, r: f64) -> String {
    match shape % SHAPES {
        // ● a circle
        0 => format!("<circle class=\"ev-dot\" r=\"{}\"/>", num(r)),
        // ■ a square, sized to the circle's area rather than its diameter, or
        // it reads as the larger mark
        1 => {
            let s = r * 1.77;
            format!(
                "<rect class=\"ev-dot\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                num(-s / 2.0),
                num(-s / 2.0),
                num(s),
                num(s)
            )
        }
        // ▲ a triangle, likewise
        2 => {
            let s = r * 1.35;
            format!(
                "<polygon class=\"ev-dot\" points=\"0,{} {},{} {},{}\"/>",
                num(-s * 1.15),
                num(s),
                num(s * 0.66),
                num(-s),
                num(s * 0.66)
            )
        }
        // ◆ a diamond
        _ => {
            let s = r * 1.25;
            format!(
                "<polygon class=\"ev-dot\" points=\"0,{} {},0 0,{} {},0\"/>",
                num(-s),
                num(s),
                num(s),
                num(-s)
            )
        }
    }
}

/// Event markers pinned to the same axis as the ribbon below them.
///
/// `kinds` is the vocabulary the notes file actually used, in first-seen order.
/// Non-empty shape-codes each kind and adds the `k<n>` class the filter row
/// toggles; empty renders plain circles, which is what a notes file whose
/// entries carry no `kind` at all gets.
pub fn rail(events: &[Event], series: &[Day], width: f64, height: f64, kinds: &[String]) -> String {
    if series.is_empty() || events.is_empty() {
        return String::new();
    }
    let first = iso(&series[0].label);
    let last = iso(&series[series.len() - 1].label);
    let span = ((last - first).num_days()).max(1) as f64;

    // Inset, so a marker on the first or last day keeps its whole dot and its
    // ring inside the box instead of being sliced by the edge.
    let inset = 6.0;
    let inner = (width - inset * 2.0).max(1.0);

    let mut parts = String::new();
    for (index, event) in events.iter().enumerate() {
        let at = |day: NaiveDate| -> f64 {
            inset + ((day - first).num_days() as f64 / span).clamp(0.0, 1.0) * inner
        };
        let x = at(event.start);
        let row = index % 3;
        let y = 8.0 + row as f64 * 8.0;
        if event.spans() {
            if let Some(end) = event.end {
                let end_x = at(end);
                let _ = write!(
                    parts,
                    "<rect class=\"ev-span\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"3\" rx=\"1.5\"/>",
                    num(x),
                    num(y - 1.5),
                    num((end_x - x).max(2.0))
                );
            }
        }
        let conf = if event.confidence.is_empty() {
            String::new()
        } else {
            format!(" conf-{}", esc(&event.confidence))
        };

        if kinds.is_empty() {
            // A plain dot: no kind was named, so there is nothing to shape-code
            // and no filter row for a class to talk to.
            let _ = write!(
                parts,
                "<g class=\"ev{conf}\" data-event=\"{}\" tabindex=\"0\" role=\"button\" aria-label=\"{}\">\
                 <line class=\"ev-stem\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>\
                 <circle class=\"ev-hit\" cx=\"{}\" cy=\"{}\" r=\"9\"/>\
                 <circle class=\"ev-dot\" cx=\"{}\" cy=\"{}\" r=\"4\"/></g>",
                esc(&event.id),
                esc(&event.title),
                num(x),
                num(y),
                num(x),
                num(height),
                num(x),
                num(y),
                num(x),
                num(y)
            );
            continue;
        }

        let shape = kinds.iter().position(|k| *k == event.kind).unwrap_or(0);
        // Weight scales the marker. `major` is the default size rather than an
        // enlargement, so a file that states no weights looks like one that
        // states them all as major — which is the honest reading of silence.
        let r = match event.weight.as_str() {
            "minor" => 2.6,
            "medium" => 3.3,
            _ => 4.0,
        };
        let _ = write!(
            parts,
            "<g class=\"ev k{shape}{conf}\" data-event=\"{}\" data-kind=\"{}\" tabindex=\"0\" \
             role=\"button\" aria-label=\"{}\">\
             <line class=\"ev-stem\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>\
             <circle class=\"ev-hit\" cx=\"{}\" cy=\"{}\" r=\"9\"/>\
             <g transform=\"translate({},{})\">{}</g></g>",
            esc(&event.id),
            esc(&event.kind),
            esc(&event.title),
            num(x),
            num(y),
            num(x),
            num(height),
            num(x),
            num(y),
            num(x),
            num(y),
            marker(shape, r)
        );
    }
    frame(width, height, &parts, "rail", "Events")
}

/// How much of the archive the notes claim to have read, on the shared axis.
///
/// **Coverage is the field that changes what the page means.** A timeline with
/// markers over one month and nothing over
/// the next nine looks identical whether the rest was quiet or simply unread,
/// and those are opposite facts about the same picture. The band draws the read
/// span against the whole span, so the unread part is visibly unread rather
/// than apparently uneventful.
///
/// An open end — `from` with no `to`, or neither — is drawn as far as it is
/// claimed and no further. Guessing the missing end would be inventing the
/// claim the block exists to state.
pub fn coverage_band(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    series: &[Day],
    width: f64,
    height: f64,
) -> String {
    if series.is_empty() || (from.is_none() && to.is_none()) {
        return String::new();
    }
    let first = iso(&series[0].label);
    let last = iso(&series[series.len() - 1].label);
    let span = ((last - first).num_days()).max(1) as f64;
    let at = |day: NaiveDate| -> f64 {
        ((day - first).num_days() as f64 / span).clamp(0.0, 1.0) * width
    };

    let start = from.map(at).unwrap_or(0.0);
    let end = to.map(at).unwrap_or(width);
    let body = format!(
        "<rect class=\"cov-bed\" x=\"0\" y=\"0\" width=\"{}\" height=\"{}\"/>\
         <rect class=\"cov-read\" x=\"{}\" y=\"0\" width=\"{}\" height=\"{}\"/>",
        num(width),
        num(height),
        num(start.min(end)),
        num((end - start).abs().max(1.0)),
        num(height)
    );
    frame(
        width,
        height,
        &body,
        "cov",
        "How much of the archive was read",
    )
}
