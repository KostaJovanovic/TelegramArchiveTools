//! **Colour emoji in egui text.**
//!
//! egui's text renderer draws every glyph as a one-colour outline, so an emoji
//! from Segoe UI Emoji came out as a flat silhouette in the text's colour. The
//! colour is in the font all the same: Windows 11 keeps it in a COLR v1 table —
//! layers of outlines, clipped, transformed and filled with solid colours or
//! gradients. This module paints that table.
//!
//! **Layout stays egui's.** A text is laid out exactly as before, with the
//! emoji in it set *transparent*: egui still measures, wraps and advances
//! over them using the emoji face's own metrics, which are the same metrics
//! this module draws in. Then each emoji is painted over its own slot from a
//! texture, rasterised once per character and size and kept.
//!
//! **What is not drawn as one emoji:** sequences. A family, a skin tone or a
//! flag is several code points that a shaping engine would join through the
//! font's ligatures, and egui does not shape. Joiners, variation selectors and
//! skin-tone modifiers are dropped, so a sequence shows as its parts — a
//! family as its members, a flag as its two letters — rather than as boxes.

use eframe::egui::{
    self, text::LayoutJob, Color32, ColorImage, FontId, Galley, Painter, Pos2, Rect, TextFormat,
    TextureHandle, TextureOptions,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use ttf_parser::{colr, Face, GlyphId, RgbaColor};

/// The emoji face, parsed once. `None` on a machine without Segoe UI Emoji,
/// where text keeps whatever the fallback fonts draw.
fn face() -> Option<&'static Face<'static>> {
    static FACE: OnceLock<Option<Face<'static>>> = OnceLock::new();
    FACE.get_or_init(|| Face::parse(crate::fonts::emoji_font()?, 0).ok())
        .as_ref()
}

/// Is `c` drawn from colour layers? Below U+2000 nothing is — `©` and `®`
/// stay text, as Telegram sets them.
pub fn is_emoji(c: char) -> bool {
    if (c as u32) < 0x2000 {
        return false;
    }
    let Some(face) = face() else {
        return false;
    };
    face.glyph_index(c).is_some_and(|g| face.is_color_glyph(g))
}

/// The code points that only modify a neighbour: joiners, variation
/// selectors, skin tones, the keycap ring and tag characters. Without
/// shaping they draw as boxes or swatches, so they are dropped.
fn is_modifier(c: char) -> bool {
    matches!(c as u32,
        0x200D | 0xFE0E | 0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F)
}

/// `text` without the modifiers above.
pub fn tidy(text: &str) -> String {
    text.chars().filter(|c| !is_modifier(*c)).collect()
}

/// A layout job for `text` in one font and colour, with every emoji in it
/// transparent — see the module docs.
pub fn job(text: &str, font: FontId, color: Color32, wrap: f32) -> LayoutJob {
    let text = tidy(text);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let plain = TextFormat::simple(font.clone(), color);
    let hidden = TextFormat::simple(font, Color32::TRANSPARENT);
    let mut start = 0;
    let mut in_emoji = false;
    for (i, c) in text.char_indices() {
        let e = is_emoji(c);
        if e != in_emoji && i > start {
            let format = if in_emoji { &hidden } else { &plain };
            job.append(&text[start..i], 0.0, format.clone());
            start = i;
        }
        in_emoji = e;
    }
    if start < text.len() {
        let format = if in_emoji { &hidden } else { &plain };
        job.append(&text[start..], 0.0, format.clone());
    }
    job
}

/// The text-painting calls the window uses, with colour emoji.
pub trait EmojiText {
    /// [`Painter::layout`], with the emoji left to [`Self::galley_e`].
    fn layout_e(&self, text: String, font: FontId, color: Color32, wrap: f32) -> Arc<Galley>;
    /// [`Painter::layout_no_wrap`], likewise.
    fn layout_no_wrap_e(&self, text: String, font: FontId, color: Color32) -> Arc<Galley>;
    /// [`Painter::galley`], then every emoji in it, in colour.
    fn galley_e(&self, pos: Pos2, galley: Arc<Galley>, fallback: Color32);
}

impl EmojiText for Painter {
    fn layout_e(&self, text: String, font: FontId, color: Color32, wrap: f32) -> Arc<Galley> {
        self.layout_job(job(&text, font, color, wrap))
    }

    fn layout_no_wrap_e(&self, text: String, font: FontId, color: Color32) -> Arc<Galley> {
        self.layout_job(job(&text, font, color, f32::INFINITY))
    }

    fn galley_e(&self, pos: Pos2, galley: Arc<Galley>, fallback: Color32) {
        paint_emoji(self, pos, &galley);
        self.galley(pos, galley, fallback);
    }
}

/// Every emoji glyph in `galley`, painted at its slot.
fn paint_emoji(painter: &Painter, pos: Pos2, galley: &Galley) {
    let ppp = painter.ctx().pixels_per_point();
    for placed in &galley.rows {
        for g in &placed.row.glyphs {
            if !is_emoji(g.chr) {
                continue;
            }
            // The slot the emoji face's own metrics gave it: the advance wide,
            // the face's line tall, the baseline where egui put it.
            let min =
                pos + placed.pos.to_vec2() + egui::vec2(g.pos.x, g.pos.y - g.font_impl_ascent);
            let rect = Rect::from_min_size(min, egui::vec2(g.advance_width, g.font_impl_height));
            let px = [
                (rect.width() * ppp).round().max(1.0) as u32,
                (rect.height() * ppp).round().max(1.0) as u32,
            ];
            let ascent_px = g.font_impl_ascent * ppp;
            if let Some(tex) = texture(painter.ctx(), g.chr, px, ascent_px) {
                painter.image(
                    tex.id(),
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
    }
}

/// One emoji at one size, rasterised on first use and kept for the process.
/// Keyed on the pixel size, so a zoom or a second monitor gets its own.
fn texture(ctx: &egui::Context, c: char, px: [u32; 2], ascent_px: f32) -> Option<TextureHandle> {
    type Cache = HashMap<(char, [u32; 2]), Option<TextureHandle>>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok()?.get(&(c, px)) {
        return hit.clone();
    }
    let tex = rasterise(c, px, ascent_px).map(|img| {
        ctx.load_texture(
            format!("emoji-{:x}-{}x{}", c as u32, px[0], px[1]),
            img,
            TextureOptions::LINEAR,
        )
    });
    cache.lock().ok()?.insert((c, px), tex.clone());
    tex
}

/// Paint `c` from the colour table into a `px`-sized image whose baseline is
/// `ascent_px` from the top.
pub fn rasterise(c: char, px: [u32; 2], ascent_px: f32) -> Option<ColorImage> {
    let face = face()?;
    let glyph = face.glyph_index(c)?;
    let advance = f32::from(face.glyph_hor_advance(glyph)?);
    if advance <= 0.0 {
        return None;
    }
    // Font units to pixels: the advance fills the width, y flips, and the
    // baseline lands where egui's layout put it.
    let s = px[0] as f32 / advance;
    let base = tiny_skia::Transform::from_row(s, 0.0, 0.0, -s, 0.0, ascent_px);
    let mut painter = Canvas::new(face, px, base)?;
    face.paint_color_glyph(glyph, 0, RgbaColor::new(0, 0, 0, 255), &mut painter)?;
    let pixmap = painter.layers.pop()?;
    Some(ColorImage::from_rgba_premultiplied(
        [px[0] as usize, px[1] as usize],
        pixmap.data(),
    ))
}

/// The COLR painter: a stack of layers, clips and transforms over tiny-skia.
struct Canvas<'a> {
    face: &'a Face<'a>,
    size: [u32; 2],
    base: tiny_skia::Transform,
    layers: Vec<tiny_skia::Pixmap>,
    modes: Vec<tiny_skia::BlendMode>,
    clips: Vec<tiny_skia::Mask>,
    transforms: Vec<tiny_skia::Transform>,
    /// The last glyph outlined, in font units.
    outline: Option<tiny_skia::Path>,
}

impl<'a> Canvas<'a> {
    fn new(face: &'a Face<'a>, size: [u32; 2], base: tiny_skia::Transform) -> Option<Self> {
        Some(Self {
            face,
            size,
            base,
            layers: vec![tiny_skia::Pixmap::new(size[0], size[1])?],
            modes: Vec::new(),
            clips: Vec::new(),
            transforms: vec![base],
            outline: None,
        })
    }

    fn transform(&self) -> tiny_skia::Transform {
        *self.transforms.last().unwrap_or(&self.base)
    }

    /// Add `path` (in the current transform) to the clip stack, intersected
    /// with the clip already there.
    fn clip_to(&mut self, path: &tiny_skia::Path) {
        let ts = self.transform();
        let mask = match self.clips.last() {
            Some(prev) => {
                let mut m = prev.clone();
                m.intersect_path(path, tiny_skia::FillRule::Winding, true, ts);
                Some(m)
            }
            None => tiny_skia::Mask::new(self.size[0], self.size[1]).map(|mut m| {
                m.fill_path(path, tiny_skia::FillRule::Winding, true, ts);
                m
            }),
        };
        // A clip that could not be made clips everything, not nothing.
        let mask = mask.or_else(|| tiny_skia::Mask::new(self.size[0], self.size[1]));
        if let Some(m) = mask {
            self.clips.push(m);
        }
    }

    fn shader(&self, paint: &colr::Paint) -> Option<tiny_skia::Shader<'static>> {
        use tiny_skia::{Color, GradientStop, Point, SpreadMode};
        let colour = |c: RgbaColor| Color::from_rgba8(c.red, c.green, c.blue, c.alpha);
        let spread = |e: colr::GradientExtend| match e {
            colr::GradientExtend::Pad => SpreadMode::Pad,
            colr::GradientExtend::Repeat => SpreadMode::Repeat,
            colr::GradientExtend::Reflect => SpreadMode::Reflect,
        };
        let coords = self.face.variation_coordinates();
        // ttf-parser leaves the stops in the font's order; tiny-skia wants
        // them ascending.
        let stops = |it: colr::GradientStopsIter| {
            let mut v: Vec<colr::ColorStop> = it.collect();
            v.sort_by(|a, b| a.stop_offset.total_cmp(&b.stop_offset));
            v.into_iter()
                .map(|s| GradientStop::new(s.stop_offset, colour(s.color)))
                .collect::<Vec<_>>()
        };
        let ts = self.transform();
        match paint {
            colr::Paint::Solid(c) => Some(tiny_skia::Shader::SolidColor(colour(*c))),
            colr::Paint::LinearGradient(g) => {
                // COLR's three-point form: the gradient runs from p0 towards
                // p1, along the normal of p0→p2.
                let (p0, p1, p2) = ((g.x0, g.y0), (g.x1, g.y1), (g.x2, g.y2));
                let (dx2, dy2) = (p2.0 - p0.0, p2.1 - p0.1);
                let perp = (dy2, -dx2);
                let len = perp.0 * perp.0 + perp.1 * perp.1;
                let end = if len <= f32::EPSILON {
                    p1
                } else {
                    let k = ((p1.0 - p0.0) * perp.0 + (p1.1 - p0.1) * perp.1) / len;
                    (p0.0 + perp.0 * k, p0.1 + perp.1 * k)
                };
                let st = stops(g.stops(0, coords));
                match st.len() {
                    0 => None,
                    1 => Some(tiny_skia::Shader::SolidColor(first_colour(
                        g.stops(0, coords),
                    )?)),
                    _ => tiny_skia::LinearGradient::new(
                        Point::from_xy(p0.0, p0.1),
                        Point::from_xy(end.0, end.1),
                        st,
                        spread(g.extend),
                        ts,
                    ),
                }
            }
            colr::Paint::RadialGradient(g) => {
                let st = stops(g.stops(0, coords));
                match st.len() {
                    0 => None,
                    1 => Some(tiny_skia::Shader::SolidColor(first_colour(
                        g.stops(0, coords),
                    )?)),
                    // tiny-skia's radial is focal-point-to-circle; COLR's
                    // start circle is almost always a point, which is exactly
                    // that.
                    _ => tiny_skia::RadialGradient::new(
                        Point::from_xy(g.x0, g.y0),
                        Point::from_xy(g.x1, g.y1),
                        g.r1.max(0.01),
                        st,
                        spread(g.extend),
                        ts,
                    ),
                }
            }
            // No sweep gradients in tiny-skia: its first colour, which is
            // the closest one flat fill can come.
            colr::Paint::SweepGradient(g) => Some(tiny_skia::Shader::SolidColor(first_colour(
                g.stops(0, coords),
            )?)),
        }
    }
}

fn first_colour(mut it: colr::GradientStopsIter) -> Option<tiny_skia::Color> {
    let s = it.next()?;
    Some(tiny_skia::Color::from_rgba8(
        s.color.red,
        s.color.green,
        s.color.blue,
        s.color.alpha,
    ))
}

/// ttf-parser's outline callbacks, into a tiny-skia path.
struct Outline(tiny_skia::PathBuilder);

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

impl<'a> colr::Painter<'a> for Canvas<'a> {
    fn outline_glyph(&mut self, glyph_id: GlyphId) {
        let mut b = Outline(tiny_skia::PathBuilder::new());
        self.outline = self
            .face
            .outline_glyph(glyph_id, &mut b)
            .and_then(|_| b.0.finish());
    }

    /// **Two meanings, told apart by the clip stack.** A v0 layer is
    /// "outline, then paint", and fills the outline. A v1 paint arrives
    /// inside a glyph clip (or the glyph's clip box) and fills whatever the
    /// clip lets through — the outline has already been spent on the clip.
    fn paint(&mut self, paint: colr::Paint<'a>) {
        let Some(shader) = self.shader(&paint) else {
            return;
        };
        let fill = tiny_skia::Paint {
            shader,
            anti_alias: true,
            ..Default::default()
        };
        let clip = self.clips.last();
        let Some(layer) = self.layers.last_mut() else {
            return;
        };
        match (clip, &self.outline) {
            (Some(mask), _) => {
                if let Some(r) =
                    tiny_skia::Rect::from_xywh(0.0, 0.0, self.size[0] as f32, self.size[1] as f32)
                {
                    layer.fill_rect(r, &fill, tiny_skia::Transform::identity(), Some(mask));
                }
            }
            (None, Some(path)) => {
                let ts = *self.transforms.last().unwrap_or(&self.base);
                layer.fill_path(path, &fill, tiny_skia::FillRule::Winding, ts, None);
            }
            (None, None) => {}
        }
    }

    fn push_clip(&mut self) {
        match self.outline.clone() {
            Some(path) => self.clip_to(&path),
            // An empty glyph clips everything away.
            None => {
                if let Some(m) = tiny_skia::Mask::new(self.size[0], self.size[1]) {
                    self.clips.push(m);
                }
            }
        }
    }

    fn push_clip_box(&mut self, clipbox: colr::ClipBox) {
        let rect =
            tiny_skia::Rect::from_ltrb(clipbox.x_min, clipbox.y_min, clipbox.x_max, clipbox.y_max);
        match rect.map(tiny_skia::PathBuilder::from_rect) {
            Some(path) => self.clip_to(&path),
            None => {
                if let Some(m) = tiny_skia::Mask::new(self.size[0], self.size[1]) {
                    self.clips.push(m);
                }
            }
        }
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn push_layer(&mut self, mode: colr::CompositeMode) {
        use colr::CompositeMode as C;
        use tiny_skia::BlendMode as B;
        let blend = match mode {
            C::Clear => B::Clear,
            C::Source => B::Source,
            C::Destination => B::Destination,
            C::SourceOver => B::SourceOver,
            C::DestinationOver => B::DestinationOver,
            C::SourceIn => B::SourceIn,
            C::DestinationIn => B::DestinationIn,
            C::SourceOut => B::SourceOut,
            C::DestinationOut => B::DestinationOut,
            C::SourceAtop => B::SourceAtop,
            C::DestinationAtop => B::DestinationAtop,
            C::Xor => B::Xor,
            C::Plus => B::Plus,
            C::Screen => B::Screen,
            C::Overlay => B::Overlay,
            C::Darken => B::Darken,
            C::Lighten => B::Lighten,
            C::ColorDodge => B::ColorDodge,
            C::ColorBurn => B::ColorBurn,
            C::HardLight => B::HardLight,
            C::SoftLight => B::SoftLight,
            C::Difference => B::Difference,
            C::Exclusion => B::Exclusion,
            C::Multiply => B::Multiply,
            C::Hue => B::Hue,
            C::Saturation => B::Saturation,
            C::Color => B::Color,
            C::Luminosity => B::Luminosity,
        };
        if let Some(p) = tiny_skia::Pixmap::new(self.size[0], self.size[1]) {
            self.layers.push(p);
            self.modes.push(blend);
        }
    }

    fn pop_layer(&mut self) {
        if self.layers.len() < 2 {
            return;
        }
        let (Some(top), Some(blend_mode)) = (self.layers.pop(), self.modes.pop()) else {
            return;
        };
        if let Some(under) = self.layers.last_mut() {
            under.draw_pixmap(
                0,
                0,
                top.as_ref(),
                &tiny_skia::PixmapPaint {
                    blend_mode,
                    ..Default::default()
                },
                tiny_skia::Transform::identity(),
                None,
            );
        }
    }

    fn push_transform(&mut self, t: ttf_parser::Transform) {
        let next = self
            .transform()
            .pre_concat(tiny_skia::Transform::from_row(t.a, t.b, t.c, t.d, t.e, t.f));
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_go_and_the_emoji_stay() {
        // 👍🏽 is a thumb and a skin tone; ❤️ a heart and a variation
        // selector; 👨‍👩‍👧 three people and two joiners.
        assert_eq!(tidy("👍🏽 ❤️ 👨\u{200d}👩\u{200d}👧"), "👍 ❤ 👨👩👧");
        assert_eq!(tidy("plain"), "plain");
    }

    #[test]
    fn copyright_and_ascii_are_text() {
        for c in ['a', '1', '#', '©', '®'] {
            assert!(!is_emoji(c), "{c:?}");
        }
    }

    #[test]
    fn an_emoji_paints_in_more_than_one_colour_where_the_font_is_installed() {
        // Only meaningful on a machine with Segoe UI Emoji; elsewhere there
        // is nothing to paint and the text keeps its fallback.
        if face().is_none() {
            return;
        }
        assert!(is_emoji('😀'));
        let img = rasterise('😀', [64, 64], 52.0).expect("a colour glyph rasterises");
        let opaque: std::collections::HashSet<[u8; 3]> = img
            .pixels
            .iter()
            .filter(|p| p.a() > 200)
            .map(|p| [p.r() / 32, p.g() / 32, p.b() / 32])
            .collect();
        assert!(
            opaque.len() > 2,
            "a face, an outline and eyes: {} colours",
            opaque.len()
        );
    }

    #[test]
    fn a_job_hides_the_emoji_and_keeps_the_text() {
        let font = FontId::proportional(14.0);
        let j = job("hi 😀 there", font, Color32::WHITE, 200.0);
        assert_eq!(j.text, "hi 😀 there");
        if face().is_some() {
            assert!(j
                .sections
                .iter()
                .any(|s| s.format.color == Color32::TRANSPARENT));
            assert!(j.sections.iter().any(|s| s.format.color == Color32::WHITE));
        }
    }
}
