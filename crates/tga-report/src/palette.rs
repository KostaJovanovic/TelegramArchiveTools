//! The report's colour.
//!
//! `tga-app`'s `theme` holds the same tokens for the window, so the report and
//! the app that produced it look like one product. The values are mirrored
//! rather than imported, because this module's only job is to write a text file
//! and it should not drag a graphics toolkit in to do it;
//! `tga_app::theme::tests` asserts the two never drift.
//!
//! **One appearance.** There was a light set here too, kept alive only because
//! a frozen reproduction of an older report emitted it and a byte diff compared
//! that stylesheet character for character. Both are gone, and so is it: a
//! second appearance is a second design to keep in step.
//!
//! **One hue does all the work.** The design has two colours and one red, which
//! rules out a categorical palette — and that turns out to cost nothing,
//! because almost every figure in this report is *magnitude*, whose correct
//! encoding is a single-hue sequential ramp anyway. Where something genuinely
//! needs telling apart by identity, the report uses small multiples on a shared
//! axis rather than inventing hues the design does not have.
//!
//! The ramp is not eyeballed. It is generated in OKLab, holding the accent's
//! hue, with chroma clamped to the sRGB gamut at each step so the lightness
//! steps come out even, and it passes four ordinal checks:
//!
//! ```text
//! monotone L       yes
//! adjacent dL      0.082 (floor 0.06)
//! end vs surface   2.33:1 (floor 2:1)
//! hue spread       1 degree
//! ```
//!
//! The end step is what sets the span. On near-black, anything darker than 2:1
//! stops being a mark and becomes surface. That is why the ramp runs past the
//! accent into deeper red rather than starting at it — the accent is a step of
//! the hue's ramp, not its end.
//!
//! `tests` below re-derives all four from the hex values, so an edit that
//! breaks one fails the suite instead of quietly shipping a ramp nobody can
//! read.

/// The tokens, **in emission order**.
///
/// A slice of pairs rather than a map: the stylesheet is written by walking
/// this, and a `BTreeMap` would sort the custom properties alphabetically, which
/// puts `accent` before `bg` and reads as nothing in particular.
pub type Tokens = &'static [(&'static str, &'static str)];

pub const TOKENS: Tokens = &[
    ("bg", "#0a0a0a"),
    ("fg", "#e8e8e8"),
    ("muted", "#888888"),
    ("hairline", "#333333"),
    ("rule", "#262626"),
    ("surface", "#141414"),
    ("accent", "#ff3347"),
    ("accent_fg", "#ffffff"),
    // Chart-only. `track` is an empty cell — a heatmap needs somewhere for zero
    // to live that is not the first step of the ramp, or a silent day and a
    // quiet day look the same.
    ("track", "#1c1c1c"),
    // Not called `grid`: the exporter's palette uses that name for the window's
    // backdrop texture, which is a different thing at a different value, and
    // the drift check compares shared names.
    ("gridline", "#222222"),
];

/// Deep-to-pale, magnitude increasing. Validated; see the module docs.
pub const RAMP: &[&str] = &["#813435", "#a83f41", "#d14a4e", "#fc555b", "#fd8b88"];

/// The class the document carries, and the only one it has.
pub const DEFAULT: &str = "dark";

pub fn tokens() -> Tokens {
    TOKENS
}

pub fn ramp() -> &'static [&'static str] {
    RAMP
}

pub fn token(name: &str) -> &'static str {
    TOKENS
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
        .unwrap_or("#000000")
}

/// The ramp colour for `value` out of `top`.
///
/// Zero is the track, never the ramp's first step: a day nobody spoke and a day
/// one person spoke are different facts and must not share a colour.
pub fn step(value: f64, top: f64) -> &'static str {
    if value <= 0.0 || top <= 0.0 {
        return token("track");
    }
    let ramp = ramp();
    // Rank by share of the maximum, floored into the first step so a single
    // message is always visible. Linear: a heatmap that quietly applies a
    // square root reports a busy cell as busier than it was.
    let index = ((value / top * ramp.len() as f64) as usize).min(ramp.len() - 1);
    ramp[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(hex: &str) -> (f64, f64, f64) {
        let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).expect("hex");
        (
            ((n >> 16) & 0xff) as f64 / 255.0,
            ((n >> 8) & 0xff) as f64 / 255.0,
            (n & 0xff) as f64 / 255.0,
        )
    }

    fn linear(c: f64) -> f64 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// sRGB -> OKLab, Björn Ottosson's matrices.
    fn oklab(hex: &str) -> (f64, f64, f64) {
        let (r, g, b) = rgb(hex);
        let (r, g, b) = (linear(r), linear(g), linear(b));
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        (
            0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
            1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
            0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
        )
    }

    fn hue_degrees(hex: &str) -> f64 {
        let (_, a, b) = oklab(hex);
        b.atan2(a).to_degrees().rem_euclid(360.0)
    }

    fn relative_luminance(hex: &str) -> f64 {
        let (r, g, b) = rgb(hex);
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (x, y) = (relative_luminance(a), relative_luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// The four checks the module docs claim, re-derived from the hex values.
    ///
    /// Stated as a test rather than as prose because the numbers are the whole
    /// argument for the ramp being this and not something prettier, and prose
    /// does not fail.
    fn check_ramp() {
        let ramp = ramp();
        let surface = token("bg");

        let lightness: Vec<f64> = ramp.iter().map(|c| oklab(c).0).collect();

        // 1. Monotone L. This ramp runs deep-to-pale; the direction is read
        //    from the ends rather than assumed, but either way it may never
        //    turn around, or two adjacent steps stop being orderable.
        let descending = lightness[0] > lightness[lightness.len() - 1];
        for pair in lightness.windows(2) {
            if descending {
                assert!(pair[0] > pair[1], "L is not monotone: {lightness:?}");
            } else {
                assert!(pair[0] < pair[1], "L is not monotone: {lightness:?}");
            }
        }

        // 2. Adjacent dL, floor 0.06. Below this two steps read as one.
        for pair in lightness.windows(2) {
            let delta = (pair[0] - pair[1]).abs();
            assert!(delta >= 0.06, "adjacent dL {delta:.3} is under 0.06");
        }

        // 3. The end step against the surface, floor 2:1. Below it the mark
        //    stops being a mark and becomes background.
        let end = contrast(ramp[ramp.len() - 1], surface);
        assert!(end >= 2.0, "end vs surface is {end:.2}:1, under 2:1");
        // And the other end, which is the one that actually binds: on white the
        // palest step is the one at risk.
        let start = contrast(ramp[0], surface);
        assert!(
            start >= 2.0,
            "first step vs surface is {start:.2}:1, under 2:1"
        );

        // 4. Hue spread. This is a *single-hue* ramp; a step that drifts in hue
        //    encodes a category the data does not have.
        let hues: Vec<f64> = ramp.iter().map(|c| hue_degrees(c)).collect();
        let spread = hues.iter().cloned().fold(f64::MIN, f64::max)
            - hues.iter().cloned().fold(f64::MAX, f64::min);
        assert!(spread <= 12.0, "hue spreads {spread:.1} degrees");
    }

    #[test]
    fn the_ramp_passes_all_four_ordinal_checks() {
        check_ramp();
    }

    #[test]
    fn zero_is_the_track_and_never_the_ramps_first_step() {
        // The stated property. A day nobody spoke and a day one person spoke
        // are different facts.
        assert_eq!(step(0.0, 100.0), token("track"));
        assert_eq!(step(5.0, 0.0), token("track"));
        assert_ne!(step(1.0, 100.0), token("track"));
    }

    #[test]
    fn the_top_value_lands_on_the_last_step_rather_than_past_it() {
        // `value / top * len` is exactly `len` at the maximum, which indexes
        // one past the end. The clamp is what stops that being a panic.
        assert_eq!(step(100.0, 100.0), RAMP[4]);
        assert_eq!(step(1.0, 100.0), RAMP[0]);
        assert_eq!(step(50.0, 100.0), RAMP[2]);
    }

    #[test]
    fn a_token_nobody_defined_is_black_rather_than_a_panic() {
        // The stylesheet is written by looking names up, and a report that
        // fails to render because one custom property was renamed is worse
        // than one rule that comes out wrong.
        assert_eq!(token("nosuchtoken"), "#000000");
        assert_eq!(token("accent"), "#ff3347");
    }
}
