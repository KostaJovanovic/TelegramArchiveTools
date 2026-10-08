//! Inline preview sizing.
//!
//! Reproduced exactly because the result is written into the `style` attribute
//! of every inline preview, and rounding differently shows up as a one-pixel
//! difference on most images — which is thousands of diffing lines.

use serde_json::{json, Map, Value};

/// Desktop scales an inline preview to fit this box.
pub const PREVIEW_BOX: i64 = 260;

/// Stickers get a smaller one.
pub const STICKER_BOX: i64 = 192;

/// A photo smaller than this in either direction is drawn as a media row rather
/// than inlined.
///
/// The reference pins the threshold only loosely: its one row case is 260×74
/// and its smallest inlined photo is 188×232, so the cut sits somewhere in
/// (74, 188]. 100 is Desktop's usual minimum-photo constant.
pub const MIN_INLINE_PHOTO: i64 = 100;

/// Qt's `QSize::scaled(box, box, KeepAspectRatio)`, with integer division.
pub fn fit_box(width: i64, height: i64, box_size: i64) -> (i64, i64) {
    if width <= 0 || height <= 0 {
        return (box_size, box_size);
    }
    // `width` and `height` are the wire's word for it, not ours: they come out
    // of a `result.json` or off the connection, and nothing between there and
    // here bounds them. `box_size * width` overflows i64 above a declared width
    // of roughly 1.8e16 — which panics a debug build and, in release, wraps to a
    // negative pixel size that goes straight into a `style` attribute. A
    // dimension that cannot be scaled is degenerate in exactly the way
    // `width <= 0` is, so it takes the same exit.
    let Some(bw) = box_size.checked_mul(width) else {
        return (box_size, box_size);
    };
    let rw = bw / height;
    if rw <= box_size {
        (rw, box_size)
    } else {
        // `rw > box_size` means width > height, so this product is smaller than
        // the one above and cannot overflow once that one has not. Checked
        // rather than argued, so the reasoning is not load-bearing.
        (
            box_size,
            box_size
                .checked_mul(height)
                .map_or(box_size, |bh| bh / width),
        )
    }
}

/// `(file pixels, css pixels)` for an inline preview.
///
/// Desktop stores the preview at **twice** its displayed size for high-density
/// screens, but **never upscales**: a 480×320 photo is kept at 480×320 and
/// shown at 240×160, not stretched to fill the 520-pixel box. Missing that
/// clamp put one image in the reference a full 20 pixels wide.
pub fn preview_size(width: i64, height: i64, box_size: i64) -> ((i64, i64), (i64, i64)) {
    let (w, h) = (width.max(0), height.max(0));
    let mut px = fit_box(w, h, box_size * 2);
    if w > 0 && w < px.0 {
        px = (w, h);
    }
    (px, ((px.0 / 2).max(1), (px.1 / 2).max(1)))
}

/// The `<img>` Desktop puts inline for a message, rebuilt from its JSON alone:
/// `{"src", "width", "height"}`, the shape `_p.preview` takes.
///
/// For any reader that has a message's `result.json` form and nothing else —
/// the html leg replaying a reference export, and `tgx unpack` writing a
/// database back out as a folder. **The engine does not use this**: it has the
/// planner's own preview name, which carries a `(1)` collision suffix this
/// cannot know about, and `src` here is only the `<stem>_thumb<ext>` Desktop
/// would name a preview that took no suffix. The *size* is exact either way —
/// it comes from `width` and `height` through [`preview_size`] — which is why
/// the html leg still tests it while overriding the name.
pub fn preview_of(m: &Map<String, Value>) -> Option<Value> {
    let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("");
    let kind = s("media_type");
    let photo = s("photo");
    let file = s("file");
    let is_placeholder = |v: &str| v.starts_with("(File");

    let sized = |src: &str, box_size: i64| {
        let w = m.get("width").and_then(Value::as_i64).unwrap_or(0);
        let h = m.get("height").and_then(Value::as_i64).unwrap_or(0);
        let (_, (w, h)) = preview_size(w, h, box_size);
        Some(json!({ "src": src, "width": w, "height": h }))
    };
    let thumb_of = |path: &str| -> String {
        match path.rsplit_once('.') {
            Some((stem, ext)) => format!("{stem}_thumb.{ext}"),
            None => format!("{path}_thumb"),
        }
    };

    if !photo.is_empty() && !is_placeholder(photo) {
        return sized(&thumb_of(photo), PREVIEW_BOX);
    }
    if file.is_empty() || is_placeholder(file) {
        return None;
    }
    let lower = file.to_lowercase();
    if kind == "sticker" && (lower.ends_with(".webp") || lower.ends_with(".png")) {
        return sized(&thumb_of(file), STICKER_BOX);
    }
    if matches!(kind, "video_file" | "video_message" | "animation") {
        // Telegram's own thumbnail, already named in the JSON — and only when
        // a path is recorded, never a guessed one.
        if let Some(thumb) = m.get("thumbnail").and_then(Value::as_str) {
            return sized(thumb, PREVIEW_BOX);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn a_photo_previews_as_its_thumb_at_the_fitted_size() {
        let m = obj(json!({ "photo": "photos/photo_1.jpg", "width": 480, "height": 320 }));
        assert_eq!(
            preview_of(&m),
            Some(json!({ "src": "photos/photo_1_thumb.jpg", "width": 240, "height": 160 }))
        );
    }

    #[test]
    fn a_file_that_was_not_saved_has_no_preview() {
        let m = obj(json!({
            "photo": "(File not included. Change data exporting settings to download.)",
            "width": 480, "height": 320,
        }));
        assert_eq!(preview_of(&m), None);
        let m = obj(json!({ "file": "video_files/a.mp4", "media_type": "video_file" }));
        assert_eq!(preview_of(&m), None, "a video with no thumbnail key");
    }

    #[test]
    fn a_static_sticker_gets_the_smaller_box_and_an_animated_one_none() {
        let m = obj(json!({
            "file": "stickers/s.webp", "media_type": "sticker", "width": 512, "height": 512,
        }));
        assert_eq!(
            preview_of(&m),
            Some(json!({ "src": "stickers/s_thumb.webp", "width": 192, "height": 192 }))
        );
        let m = obj(json!({ "file": "stickers/s.tgs", "media_type": "sticker" }));
        assert_eq!(preview_of(&m), None);
    }

    #[test]
    fn a_small_photo_is_never_upscaled() {
        // The measured case: 480x320 shows at 240x160, not stretched to the
        // 520-pixel box. Dropping the clamp is what made one reference image
        // twenty pixels wide.
        let (file, css) = preview_size(480, 320, PREVIEW_BOX);
        assert_eq!(file, (480, 320));
        assert_eq!(css, (240, 160));
    }

    #[test]
    fn a_large_photo_fits_the_doubled_box() {
        let (file, css) = preview_size(2560, 1706, PREVIEW_BOX);
        // 520-wide box, aspect kept by integer division.
        assert_eq!(file.0, 520);
        assert_eq!(file.1, 520 * 1706 / 2560);
        assert_eq!(css, (260, file.1 / 2));
    }

    #[test]
    fn integer_division_not_rounding() {
        // 520 * 1706 / 2560 = 346.5 -> 346, not 347. One pixel here is
        // thousands of diffing lines.
        assert_eq!(fit_box(2560, 1706, 520), (520, 346));
    }

    #[test]
    fn a_tall_image_is_bounded_by_height() {
        let (w, h) = fit_box(100, 400, 200);
        assert_eq!(h, 200);
        assert_eq!(w, 50);
    }

    #[test]
    fn css_size_never_collapses_to_zero() {
        // A 1px-tall image halves to 0, which would emit `height: 0px`.
        let (_, css) = preview_size(1, 1, PREVIEW_BOX);
        assert!(css.0 >= 1 && css.1 >= 1, "got {css:?}");
    }

    #[test]
    fn missing_dimensions_fall_back_to_the_box() {
        assert_eq!(fit_box(0, 0, 200), (200, 200));
        assert_eq!(fit_box(-5, 10, 200), (200, 200));
    }

    #[test]
    fn a_hostile_dimension_falls_back_instead_of_overflowing() {
        // These are declared sizes from a message, so an i64 is all that bounds
        // them. `box_size * width` used to be a plain multiply: this panicked in
        // debug and wrapped to a negative `width: -…px` in release.
        assert_eq!(fit_box(i64::MAX, 1, 520), (520, 520));
        assert_eq!(fit_box(i64::MAX, i64::MAX, 520), (520, 520));
        assert_eq!(fit_box(i64::MAX / 3, 7, 520), (520, 520));
        // preview_size doubles the box before calling in, so the same input has
        // to survive the whole path a real preview takes.
        let (file, css) = preview_size(i64::MAX, 1, PREVIEW_BOX);
        assert_eq!(file, (520, 520));
        assert_eq!(css, (260, 260));
    }

    /// Would Desktop inline a photo of this size, or draw it as a row?
    ///
    /// Mirrors the decision in `writer::render_media` so the threshold can be
    /// tested against the measured cases rather than against itself.
    fn inlines(width: i64, height: i64) -> bool {
        width.min(height) >= MIN_INLINE_PHOTO
    }

    #[test]
    fn the_inline_cut_sits_inside_the_measured_bracket() {
        // The reference's one row case is 260x74 and its smallest inlined
        // photo is 188x232, so the threshold must fall in (74, 188]. A const
        // assertion rather than a runtime one: it is a statement about the
        // constant, so it should fail the build, not a test run.
        const _: () = assert!(MIN_INLINE_PHOTO > 74 && MIN_INLINE_PHOTO <= 188);
        // It must also classify both measured cases correctly. Asserted through
        // the decision function, not against the constants themselves — the
        // first version of this test compared two literals and so could not
        // fail whatever the threshold was.
        assert!(!inlines(260, 74), "the measured row case must not inline");
        assert!(
            inlines(188, 232),
            "the measured inline case must not become a row"
        );
        // Orientation must not matter.
        assert!(!inlines(74, 260));
        assert!(inlines(232, 188));
    }
}
