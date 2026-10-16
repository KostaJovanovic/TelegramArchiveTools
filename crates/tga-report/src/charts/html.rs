//! The marks drawn as HTML rather than SVG: ranked bars, columns, the time
//! axis, and the frame a wide chart scrolls in.
//!
//! **Text in an SVG scales with the drawing.** A chart drawn at 1,120 units and
//! shown on a 390 px phone sets its 11-unit labels at about 4 px, which is
//! nobody's reading size. These marks are the ones whose labels are the point —
//! a name beside its bar, an hour under its column — so they are laid out by
//! the browser, and their text stays the size of the page's text on any
//! screen. The bars are still drawn server-side; scripting only adds to them.

use super::*;

/// One ranked bar: the label, the value it is drawn at, the text printed at
/// its tip, and the peer keys it belongs to (space-separated, for Focus).
pub struct HBar {
    pub label: String,
    pub value: f64,
    pub shown: String,
    pub who: String,
}

impl HBar {
    pub fn new(label: impl Into<String>, value: f64, shown: impl Into<String>) -> Self {
        HBar {
            label: label.into(),
            value,
            shown: shown.into(),
            who: String::new(),
        }
    }

    pub fn who(mut self, keys: impl Into<String>) -> Self {
        self.who = keys.into();
        self
    }
}

/// Ranked horizontal bars: label, bar, value at the tip.
///
/// Horizontal because the labels are people's names and topic titles, which do
/// not fit under a column and must never be rotated to make them. `hot` is the
/// row the chart points at, drawn in the accent; every other bar is `mark`.
pub fn hbars(rows: &[HBar], hot: Option<usize>) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let top = rows.iter().map(|r| r.value).fold(0.0f64, f64::max);
    let top = if top <= 0.0 { 1.0 } else { top };
    let mut out = String::from("<ol class=\"hbars\">");
    for (index, row) in rows.iter().enumerate() {
        let shown = if row.shown.is_empty() {
            thousands_f(row.value)
        } else {
            row.shown.clone()
        };
        let who = if row.who.is_empty() {
            String::new()
        } else {
            format!(" data-who=\"{}\"", esc(&row.who))
        };
        let cls = if Some(index) == hot {
            " class=\"hot\""
        } else {
            ""
        };
        let _ = write!(
            out,
            "<li{cls}{who}><span class=\"hl\" title=\"{}\">{}</span>\
             <span class=\"ht\"><i style=\"width:{}%\"></i></span>\
             <span class=\"hv\">{}</span></li>",
            esc(&row.label),
            esc(&row.label),
            num((row.value / top * 100.0).clamp(0.6, 100.0)),
            esc(&shown)
        );
    }
    out.push_str("</ol>");
    out
}

/// One column per slot, the tallest labelled with its value and drawn in the
/// accent.
///
/// `shown` turns a value into the words its tooltip and peak label use, so a
/// column of seconds can say "6 min" rather than "6 416". `id`, when not
/// empty, is for the script, which draws a focused person over the columns.
pub struct Cols<'a> {
    pub labels: &'a [String],
    pub values: &'a [i64],
    pub shown: &'a dyn Fn(i64) -> String,
    /// Label every this many columns. On a phone every other one of those.
    pub every: usize,
    pub id: &'a str,
}

pub fn vcols(c: &Cols) -> String {
    if c.values.is_empty() {
        return String::new();
    }
    let peak_value = c.values.iter().copied().max().unwrap_or(0);
    let top = peak_value.max(1) as f64;
    let peak = (peak_value > 0)
        .then(|| c.values.iter().position(|v| *v == peak_value))
        .flatten();
    let every = c.every.max(1);
    let id = if c.id.is_empty() {
        String::new()
    } else {
        format!(" id=\"{}\"", esc(c.id))
    };
    let mut bars = String::new();
    let mut labels = String::new();
    for (index, value) in c.values.iter().enumerate() {
        let label = c.labels.get(index).map(String::as_str).unwrap_or_default();
        let tip = format!("{} · {}", label, (c.shown)(*value));
        let height = if *value > 0 {
            (*value as f64 / top * 100.0).max(0.8)
        } else {
            0.0
        };
        let (cls, value_label) = if Some(index) == peak {
            // A label over a column near either end is anchored to that end,
            // or it runs out of its panel and off a phone's screen.
            let at = index as f64 / c.values.len().max(1) as f64;
            let edge = if at >= 0.8 {
                " r"
            } else if at < 0.2 {
                " l"
            } else {
                ""
            };
            (
                " hot",
                format!(
                    "<b class=\"pv{edge}\" style=\"bottom:{}%\">{}</b>",
                    num(height),
                    esc(&(c.shown)(*value))
                ),
            )
        } else {
            ("", String::new())
        };
        let _ = write!(
            bars,
            "<div class=\"vc{cls}\" data-tip=\"{}\" data-v=\"{value}\">\
             <i style=\"height:{}%\"></i>{value_label}</div>",
            esc(&tip),
            num(height)
        );
        let shown = if index % every == 0 {
            let minor = if (index / every) % 2 == 1 {
                " class=\"m\""
            } else {
                ""
            };
            format!("<span{minor}>{}</span>", esc(label))
        } else {
            "<span></span>".to_string()
        };
        labels.push_str(&shown);
    }
    format!(
        "<div class=\"vchart{}\"{id} data-max=\"{peak_value}\"><div class=\"vcols\">{bars}</div>\
         <div class=\"vlabels\">{labels}</div></div>",
        dense(c.values.len(), every)
    )
}

/// More labels than a phone has room for, so every other one is dropped
/// there. Eight is what fits across 360 px at the page's label size.
fn dense(columns: usize, every: usize) -> &'static str {
    if columns.div_ceil(every) > 8 {
        " dense"
    } else {
        ""
    }
}

/// Two people's columns side by side in each slot, on one scale: the two
/// sides of a private chat, hour by hour.
pub fn vcols2(
    labels: &[String],
    a: &[i64],
    b: &[i64],
    names: (&str, &str),
    every: usize,
) -> String {
    if a.is_empty() || a.len() != b.len() {
        return String::new();
    }
    let top = a.iter().chain(b.iter()).copied().max().unwrap_or(0).max(1) as f64;
    let every = every.max(1);
    let height = |v: i64| {
        if v > 0 {
            (v as f64 / top * 100.0).max(0.8)
        } else {
            0.0
        }
    };
    let mut bars = String::new();
    let mut ticks = String::new();
    for (index, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        let label = labels.get(index).map(String::as_str).unwrap_or_default();
        let _ = write!(
            bars,
            "<div class=\"vc two-up\" data-tip=\"{} · {} {} · {} {}\">\
             <i class=\"one\" style=\"height:{}%\"></i><i class=\"two\" style=\"height:{}%\"></i></div>",
            esc(label),
            esc(names.0),
            thousands(*x),
            esc(names.1),
            thousands(*y),
            num(height(*x)),
            num(height(*y))
        );
        if index % every == 0 {
            let minor = if (index / every) % 2 == 1 {
                " class=\"m\""
            } else {
                ""
            };
            let _ = write!(ticks, "<span{minor}>{}</span>", esc(label));
        } else {
            ticks.push_str("<span></span>");
        }
    }
    format!(
        "<div class=\"vchart{}\"><div class=\"vcols\">{bars}</div>\
         <div class=\"vlabels\">{ticks}</div></div>",
        dense(a.len(), every)
    )
}

/// The marks the time axis puts under the shared span, as `(share, label)`:
/// years past two and a half years, otherwise months, thinned to fit.
pub fn axis_marks(series: &[Day]) -> Vec<(f64, String)> {
    let Some((first_day, last_day)) = series.first().zip(series.last()) else {
        return Vec::new();
    };
    let first = super::time::iso(&first_day.label);
    let last = super::time::iso(&last_day.label);
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
                let mut label = super::time::month_abbrev(mark.month()).to_string();
                // The year at January and at the very first mark, and nowhere
                // else: repeated on every tick it is chrome.
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
    marks
        .into_iter()
        .filter(|(when, _)| *when >= first)
        .map(|(when, label)| ((when - first).num_days() as f64 / span as f64, label))
        .collect()
}

/// Month or year ticks under the shared span, as text the size of the page's.
///
/// Every other label is marked minor, and a phone shows only the major ones:
/// a month per tick across 390 px is a row of collisions.
pub fn time_axis(series: &[Day]) -> String {
    let marks = axis_marks(series);
    if marks.is_empty() {
        return String::new();
    }
    let mut out = String::from("<div class=\"taxis\" aria-hidden=\"true\">");
    for (index, (share, label)) in marks.iter().enumerate() {
        let minor = if index % 2 == 1 { " m" } else { "" };
        // The last label is right-aligned on its tick, or a mark near the end
        // runs off the page.
        let end = if *share > 0.94 { " end" } else { "" };
        let _ = write!(
            out,
            "<span class=\"t{minor}{end}\" style=\"left:{}%\">{}</span>",
            num(share * 100.0),
            esc(label)
        );
    }
    out.push_str("</div>");
    out
}

/// A chart too dense to shrink, in a frame that scrolls sideways instead.
///
/// `min_width` is the width the chart's text is the right size at — its
/// viewBox width — so on a narrow screen it keeps that size and pans, and on
/// a wide one it fills the measure as before.
pub fn scroll(chart: &str, min_width: f64) -> String {
    format!(
        "<div class=\"scrollx\"><div class=\"scrollin\" style=\"min-width:{}px\">{chart}</div></div>",
        num(min_width)
    )
}

/// The same frame for a chart with a fixed size, which must not be stretched
/// to fill a wider one: the calendar, whose squares are a fixed size.
pub fn scroll_fixed(chart: &str, width: f64) -> String {
    format!(
        "<div class=\"scrollx end\"><div class=\"scrollin\" style=\"width:{}px\">{chart}</div></div>",
        num(width)
    )
}
