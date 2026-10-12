//! Escaping, clipping and the two number formats every mark uses.

/// Everything interpolated into markup goes through this, no exceptions.
///
/// A report opens as a local file, so anything that survives as markup runs
/// with that origin — and every name, filename, emoji and link in here came
/// from strangers on the internet.
pub fn esc(text: &str) -> String {
    // `&` first, or the ampersands this function introduces get escaped again.
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Trim `text` to what fits in `room` px, with an ellipsis.
///
/// SVG has no text wrapping and this report sets `overflow:visible` so marks
/// can carry labels, which together mean an over-long name is drawn straight
/// off the left edge of the page instead of being clipped. The full string
/// stays in the tooltip. `size` is the measured average advance of Geist Mono
/// at 11px.
pub fn clip(text: &str, room: f64, size: f64) -> String {
    let limit = ((room / size) as usize).max(4);
    // Characters, not bytes: a name in Cyrillic is half as many characters as
    // it is bytes, and slicing on bytes would cut one in half.
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit - 1).collect();
    format!("{head}…")
}

/// The default advance, for callers that do not measure their own.
pub fn clip_default(text: &str, room: f64) -> String {
    clip(text, room, 6.05)
}

/// A coordinate, trimmed. Three decimals is well under a pixel.
pub fn num(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0');
    let text = text.trim_end_matches('.');
    if text.is_empty() {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// A count, grouped for reading.
///
/// The separator is **U+00A0**, not a space: a group separator that wraps puts
/// "6" at the end of one line and "643" at the start of the next.
pub fn thousands(value: i64) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if negative {
        out.push('-');
    }
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{a0}');
        }
        out.push(ch);
    }
    out
}

/// `thousands` of a float, truncated toward zero as Python's `int()` is.
pub fn thousands_f(value: f64) -> String {
    thousands(value.trunc() as i64)
}
