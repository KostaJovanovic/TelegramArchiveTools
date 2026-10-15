//! **The Viewer's chat, drawn the way Telegram draws one**: bubbles, a run of
//! messages from one person under one name with one userpic, day separators
//! and service messages as centred pills.
//!
//! A deliberate exception to the rest of the window, which is square,
//! hairlined and red-on-ink. That design is for controls; this is for reading
//! a conversation, and a conversation is easiest to read in the shape people
//! already read conversations in. The exception stops at this pane's edge.
//!
//! **Painted, not laid out with widgets.** A bubble's height depends on its
//! text, its reply block, its media and whether the time fits beside the last
//! line — the one thing a virtualised list needs to know before it draws.
//! [`draw`] answers it with or without painting, from the same code, so the
//! height a list reserves and the height a bubble takes cannot disagree.
//!
//! The text is other people's conversation, for the screen only.

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Galley, Pos2, Rect, Sense, Stroke, Ui,
};
use std::collections::HashMap;
use std::sync::Arc;
use tgx_tg::viewer::{Attachment, Row, Visual};
// Every piece of text in a bubble goes through these, so an emoji anywhere —
// the message, a name, a reply's quote, a reaction, a poll — is in colour.
use tgx_ui::emoji::EmojiText;
use tgx_ui::tokens::Palette;

/// The userpic column, and the userpic in it.
const AVATAR: f32 = 34.0;
/// From the pane's left edge to the userpic.
const LEFT: f32 = 12.0;
/// Between the userpic and its bubble.
const GAP: f32 = 8.0;
/// Inside a bubble.
const PAD_X: f32 = 11.0;
const PAD_Y: f32 = 7.0;
/// A bubble never grows past this, however wide the window: a line of text
/// longer than this is harder to read, not easier.
const BUBBLE_MAX: f32 = 560.0;
const RADIUS: u8 = 14;
/// The corner a run of bubbles hangs from — Telegram's tail, as a corner.
const TAIL_RADIUS: u8 = 4;
/// Between two bubbles of one run, and after the last of a run.
const IN_RUN: f32 = 3.0;
const AFTER_RUN: f32 = 10.0;
/// The reply block and the media chip.
const REPLY_H: f32 = 40.0;
const MEDIA_H: f32 = 44.0;
/// Consecutive messages from one sender this close together share a run.
const RUN_MINUTES: i32 = 10;

const BODY: f32 = tgx_ui::tokens::window::READING;
/// The small sans inside a bubble: the time, a forward, a reply's text.
const SMALL: f32 = 12.0;

/// One thing in the list: a day separator, or a message and its place in a
/// run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Item {
    Day(String),
    Msg {
        /// Index into the viewer's rows.
        row: usize,
        /// First of its run: carries the name.
        head: bool,
        /// Last of its run: carries the userpic and the tail corner.
        tail: bool,
    },
}

/// The list's items for the rows `shown` keeps, and for each entry of
/// `shown`, which item it became.
pub(super) fn build(rows: &[Row], shown: &[usize]) -> (Vec<Item>, Vec<usize>) {
    let mut items = Vec::with_capacity(shown.len() + shown.len() / 20);
    let mut item_of = Vec::with_capacity(shown.len());
    let mut day = "";
    for (n, &r) in shown.iter().enumerate() {
        let row = &rows[r];
        if row.day != day && !row.day.is_empty() {
            day = &row.day;
            items.push(Item::Day(row.day.clone()));
        }
        let joins = |a: &Row, b: &Row| {
            !a.service
                && !b.service
                && a.from_id == b.from_id
                && a.day == b.day
                && (minutes(&b.time) - minutes(&a.time)).abs() <= RUN_MINUTES
        };
        let head = match items.last() {
            Some(Item::Msg { row: prev, .. }) => !joins(&rows[*prev], row),
            _ => true,
        };
        let tail = match shown.get(n + 1) {
            Some(&next) => !joins(row, &rows[next]) || rows[next].day != row.day,
            None => true,
        };
        item_of.push(items.len());
        items.push(Item::Msg { row: r, head, tail });
    }
    (items, item_of)
}

fn minutes(hhmm: &str) -> i32 {
    let h: i32 = hhmm.get(..2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let m: i32 = hhmm.get(3..5).and_then(|s| s.parse().ok()).unwrap_or(0);
    h * 60 + m
}

/// The chat pane's colours: Telegram's own night and day themes, close
/// enough to be recognised, chosen by the window's appearance.
#[derive(Debug, Clone, Copy)]
pub(super) struct Ink {
    pub bg: Color32,
    bubble: Color32,
    bubble_selected: Color32,
    text: Color32,
    meta: Color32,
    pill: Color32,
    pill_text: Color32,
    link: Color32,
    deleted: Color32,
    hit: Color32,
}

pub(super) fn ink(p: &Palette) -> Ink {
    use tgx_ui::tokens::hex;
    if p.is_light() {
        Ink {
            bg: hex(0xd9e3ec),
            bubble: hex(0xffffff),
            bubble_selected: hex(0xe3effa),
            text: hex(0x000000),
            meta: hex(0x8696a6),
            pill: Color32::from_black_alpha(70),
            pill_text: hex(0xffffff),
            link: hex(0x2f80d1),
            deleted: p.accent,
            hit: Color32::from_rgba_unmultiplied(255, 200, 40, 60),
        }
    } else {
        Ink {
            bg: hex(0x0e1621),
            bubble: hex(0x182533),
            bubble_selected: hex(0x22364a),
            text: hex(0xf2f5f8),
            meta: hex(0x6f8396),
            pill: hex(0x1e2c3a),
            pill_text: hex(0xdfe7ee),
            link: hex(0x6ab3f3),
            deleted: p.accent,
            hit: Color32::from_rgba_unmultiplied(255, 200, 40, 40),
        }
    }
}

/// Desktop's eight userpic colours, `userpic1` to `userpic8` in its
/// stylesheet. A name is drawn in its userpic's colour, as Telegram does.
pub(super) fn name_colour(c: u8) -> Color32 {
    use tgx_ui::tokens::hex;
    match c {
        1 => hex(0xff5555),
        2 => hex(0x64bf47),
        3 => hex(0xffab00),
        4 => hex(0x4f9cd9),
        5 => hex(0x9884e8),
        6 => hex(0xe671a5),
        7 => hex(0x47bcd1),
        _ => hex(0xff8c44),
    }
}

/// What a click on an item asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Click {
    Select(i64),
    /// The reply block: go to the message it answers.
    Jump(i64),
    /// The media: open the file the message carries.
    Open(i64),
    /// A location: open it on a map, as Desktop's HTML links it.
    Url(String),
}

/// One message's picture, as far as the window has got with it.
pub(super) enum Pic {
    /// Asked for; the bubble shows a placeholder of the right shape.
    Loading,
    /// Decoded off the UI thread, waiting for the frame that uploads it.
    Decoded(Arc<egui::ColorImage>),
    Ready(egui::TextureHandle),
    /// The file is stored but nothing about it decodes — a GIF or video with
    /// no thumbnail. Drawn as a tile to play, never as missing.
    NoPreview,
    /// Nothing in the database decodes: the bubble draws a card instead.
    Missing,
}

/// What [`draw`] needs besides the item.
pub(super) struct Scene<'a> {
    pub rows: &'a [Row],
    /// Message id to its index in `rows`, for a reply's quote.
    pub index: &'a HashMap<i64, usize>,
    pub selected: Option<i64>,
    pub hits: &'a std::collections::HashSet<i64>,
    /// Each topic's name, when every topic is shown at once: the bubble says
    /// which one it was posted in.
    pub topic_names: Option<&'a HashMap<i64, String>>,
    pub ink: Ink,
    pub pictures: &'a HashMap<i64, Pic>,
    /// Every picture painted this frame, so the list can ask for the ones it
    /// does not have and keep the ones it does.
    pub painted: &'a std::cell::RefCell<Vec<i64>>,
    /// Profile pictures by sender (`"user1234"`), for the chat's database
    /// holds them. Anyone missing keeps their initials.
    pub userpics: &'a HashMap<String, Pic>,
}

/// Lay out one item at `origin`, `width` wide, and return its height. With
/// `paint`, also draw it and report a click.
///
/// The same code measures and paints, so the space the list reserves is the
/// space the item takes.
pub(super) fn draw(
    ui: &Ui,
    item: &Item,
    scene: &Scene,
    origin: Pos2,
    width: f32,
    paint: bool,
) -> (f32, Option<Click>) {
    match item {
        Item::Day(day) => pill(ui, &day_label(day), scene, origin, width, paint, None),
        Item::Msg { row, head, tail } => {
            let r = &scene.rows[*row];
            if r.service {
                pill(ui, &r.text, scene, origin, width, paint, Some(r))
            } else {
                bubble(ui, r, *head, *tail, scene, origin, width, paint)
            }
        }
    }
}

/// A cheap guess at an item's height, for the items not yet drawn — so the
/// scroll bar is roughly the right length before every message has been
/// measured. Drawn items replace their guess with the truth.
pub(super) fn estimate(item: &Item, rows: &[Row], width: f32) -> f32 {
    const LINE: f32 = 19.0;
    let inner = (width - LEFT - AVATAR - GAP - 48.0).clamp(120.0, BUBBLE_MAX) - 2.0 * PAD_X;
    let per_line = (inner / 7.6).max(10.0);
    let lines = |text: &str| -> f32 {
        text.lines()
            .map(|l| (l.chars().count() as f32 / per_line).ceil().max(1.0))
            .sum()
    };
    match item {
        Item::Day(_) => 36.0,
        Item::Msg { row, head, tail } => {
            let r = &rows[*row];
            if r.service {
                let photo = match &r.attachment {
                    Some(Attachment::Visual { .. }) => 166.0,
                    _ => 0.0,
                };
                return 30.0 + 18.0 * (lines(&r.text) - 1.0).max(0.0) + photo;
            }
            let mut h = 2.0 * PAD_Y;
            if *head {
                h += 20.0;
            }
            if r.forwarded.is_some() {
                h += 17.0;
            }
            if r.reply_to.is_some() {
                h += REPLY_H + 6.0;
            }
            h += match &r.attachment {
                Some(Attachment::Visual { kind, size, .. }) => {
                    visual_size(*kind, *size, inner).y + 6.0
                }
                Some(Attachment::Poll(poll)) => 70.0 + 40.0 * poll.answers.len() as f32,
                Some(Attachment::Location { .. }) => MAP_H + 60.0,
                Some(_) => MEDIA_H + 6.0,
                None if r.media.is_some() => MEDIA_H + 6.0,
                None => 0.0,
            };
            if !r.reactions.is_empty() {
                h += 30.0;
            }
            h += if r.text.is_empty() {
                16.0
            } else {
                lines(&r.text) * LINE
            };
            h + if *tail { AFTER_RUN } else { IN_RUN }
        }
    }
}

/// A day separator or a service message: a centred pill.
fn pill(
    ui: &Ui,
    label: &str,
    scene: &Scene,
    origin: Pos2,
    width: f32,
    paint: bool,
    row: Option<&Row>,
) -> (f32, Option<Click>) {
    let ink = scene.ink;
    let wrap = (width - 80.0).clamp(120.0, 440.0);
    let galley = ui
        .painter()
        .layout_e(label.to_string(), sans(SMALL + 0.5), ink.pill_text, wrap);
    let size = galley.size() + vec2(20.0, 8.0);
    let top = origin.y + 8.0;
    let rect = Rect::from_min_size(pos2(origin.x + (width - size.x) / 2.0, top), size);
    // **A new group photo is shown under the sentence announcing it**, the
    // way Telegram shows it: small, round-cornered, centred.
    let photo = row.and_then(|r| match &r.attachment {
        Some(Attachment::Visual { kind, size, .. }) => {
            let src = match scene.pictures.get(&r.id) {
                Some(Pic::Ready(tex)) => [tex.size()[0] as u32, tex.size()[1] as u32],
                _ => *size,
            };
            let s = visual_size(*kind, src, 160.0).min(vec2(160.0, 160.0));
            Some((*kind, s))
        }
        _ => None,
    });
    let height = size.y + 16.0 + photo.map_or(0.0, |(_, s)| s.y + 6.0);
    if !paint {
        return (height, None);
    }
    let mut click = None;
    if let (Some(r), Some((kind, s))) = (row, photo) {
        let area =
            Rect::from_min_size(pos2(origin.x + (width - s.x) / 2.0, rect.bottom() + 6.0), s);
        if paint_picture(ui, r.id, kind, area, None, r.file_skipped, scene) {
            click = Some(Click::Open(r.id));
        }
    }
    if let Some(r) = row {
        let band = Rect::from_min_size(origin, vec2(width, height));
        let hit = scene.hits.contains(&r.id);
        if hit {
            ui.painter().rect_filled(band, CornerRadius::ZERO, ink.hit);
        }
        let response = ui.interact(rect, ui.id().with(("svc", r.id)), Sense::click());
        if response.clicked() && click.is_none() {
            click = Some(Click::Select(r.id));
        }
    }
    let selected = row.is_some_and(|r| scene.selected == Some(r.id));
    let deleted = row.is_some_and(|r| r.deleted.is_some());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(12), ink.pill);
    if selected {
        painter.rect_stroke(
            rect,
            CornerRadius::same(12),
            Stroke::new(1.5_f32, ink.link),
            egui::StrokeKind::Outside,
        );
    } else if deleted {
        painter.rect_stroke(
            rect,
            CornerRadius::same(12),
            Stroke::new(1.0_f32, ink.deleted),
            egui::StrokeKind::Outside,
        );
    }
    painter.galley_e(rect.min + vec2(10.0, 4.0), galley, ink.pill_text);
    (height, click)
}

#[allow(clippy::too_many_arguments)]
fn bubble(
    ui: &Ui,
    r: &Row,
    head: bool,
    tail: bool,
    scene: &Scene,
    origin: Pos2,
    width: f32,
    paint: bool,
) -> (f32, Option<Click>) {
    let ink = scene.ink;
    let painter = ui.painter();
    let x0 = origin.x + LEFT + AVATAR + GAP;
    let max_w = (width - (x0 - origin.x) - 48.0).clamp(120.0, BUBBLE_MAX);
    let inner = max_w - 2.0 * PAD_X;
    let colour = name_colour(r.colour);

    // -- lay out every part, top to bottom, tracking the widest -----------
    let mut want_w: f32 = 0.0;
    let mut y = PAD_Y;
    let mut parts: Vec<(f32, Part)> = Vec::new();

    if head {
        let name = if r.from.is_empty() {
            "Unknown"
        } else {
            &r.from
        };
        let g = painter.layout_e(name.to_string(), semibold(BODY - 0.5), colour, inner);
        let mut w = g.size().x;
        let h = g.size().y;
        parts.push((y, Part::Text(g, 0.0)));
        if let Some(t) = scene.topic_names.and_then(|n| n.get(&r.topic_id)) {
            let tg = painter.layout_no_wrap_e(format!("  ·  {t}"), sans(SMALL), ink.meta);
            let tw = tg.size().x;
            if w + tw <= inner {
                parts.push((y + (h - tg.size().y) / 2.0 + 1.0, Part::Text(tg, w)));
                w += tw;
            }
        }
        want_w = want_w.max(w);
        y += h + 2.0;
    }
    if let Some(f) = &r.forwarded {
        let g = painter.layout_e(format!("Forwarded from {f}"), sans(SMALL), ink.link, inner);
        want_w = want_w.max(g.size().x);
        let h = g.size().y;
        parts.push((y, Part::Text(g, 0.0)));
        y += h + 3.0;
    }
    if let Some(to) = r.reply_to {
        let (who, what, c) = match scene.index.get(&to).map(|&i| &scene.rows[i]) {
            Some(t) => {
                let what = match (&t.media, t.preview.is_empty()) {
                    (Some(m), true) => media_name(m).to_string(),
                    _ => t.preview.clone(),
                };
                (t.from.clone(), what, name_colour(t.colour))
            }
            None => (
                "Reply".to_string(),
                format!("Message {to}, not in this database"),
                ink.meta,
            ),
        };
        let wg = painter.layout_no_wrap_e(who, semibold(SMALL), c);
        let tg = painter.layout_no_wrap_e(what, sans(SMALL), ink.text);
        let w = (wg.size().x.max(tg.size().x) + 14.0).clamp(160.0, inner);
        want_w = want_w.max(w);
        parts.push((
            y,
            Part::Reply {
                to,
                who: wg,
                what: tg,
                colour: c,
                w,
            },
        ));
        y += REPLY_H + 6.0;
    }
    // **A sticker or a round video on its own is drawn without a bubble**, as
    // Telegram draws it: the picture is the message.
    let mut bare = false;
    if let Some(a) = &r.attachment {
        let (part, w, h) = attachment(ui, r, a, scene, inner);
        if let Part::Picture {
            kind: Visual::Sticker | Visual::Round,
            ..
        } = &part
        {
            bare = r.text.is_empty() && r.reply_to.is_none() && r.forwarded.is_none();
        }
        want_w = want_w.max(w);
        parts.push((y, part));
        y += h + 6.0;
    } else if let Some(m) = &r.media {
        let lg =
            painter.layout_no_wrap_e(media_name(m).to_string(), semibold(SMALL + 1.0), ink.text);
        let dg = r
            .media_detail
            .as_ref()
            .map(|d| painter.layout_no_wrap_e(d.clone(), sans(SMALL), ink.meta));
        let w = (MEDIA_H + 10.0 + lg.size().x.max(dg.as_ref().map_or(0.0, |g| g.size().x)) + 8.0)
            .min(inner);
        want_w = want_w.max(w);
        parts.push((
            y,
            Part::Media {
                label: lg,
                detail: dg,
                fill: media_colour(m),
                w,
            },
        ));
        y += MEDIA_H + 6.0;
    }

    // The footer: markers, then the time, in the bottom-right corner.
    let mut foot: Vec<(String, Color32)> = Vec::new();
    if r.recovered {
        foot.push(("recovered".into(), ink.link));
    }
    if let Some(at) = &r.deleted {
        foot.push((
            format!("deleted {}", at.get(..10).unwrap_or(at)),
            ink.deleted,
        ));
    }
    if r.edited_at.is_some() || r.version > 1 {
        foot.push(("edited".into(), ink.meta));
    }
    foot.push((r.time.clone(), ink.meta));
    let foot: Vec<Arc<Galley>> = foot
        .into_iter()
        .map(|(s, c)| painter.layout_no_wrap_e(s, sans(SMALL - 0.5), c))
        .collect();
    let foot_gap = 6.0;
    let foot_w: f32 =
        foot.iter().map(|g| g.size().x).sum::<f32>() + foot_gap * (foot.len() - 1) as f32;
    let foot_h = foot.iter().map(|g| g.size().y).fold(0.0, f32::max);

    // The last line drawn — text or reactions — as (its width, where it
    // ends), so the time can sit beside it.
    let mut last: Option<(f32, f32)> = None;
    if !r.text.is_empty() {
        let g = painter.layout_e(r.text.clone(), sans(BODY), ink.text, inner);
        let last_w = g.rows.last().map(|row| row.rect().width()).unwrap_or(0.0);
        want_w = want_w.max(g.size().x);
        let h = g.size().y;
        parts.push((y, Part::Text(g, 0.0)));
        y += h;
        last = Some((last_w, y));
    }
    if !r.reactions.is_empty() {
        if last.is_some() {
            y += 6.0;
        }
        // Pills in a flow, wrapping at the bubble's width.
        let (mut x, mut row_y, mut row_h) = (0.0_f32, y, 0.0_f32);
        for (emoji, count) in &r.reactions {
            let g = painter.layout_no_wrap_e(format!("{emoji} {count}"), sans(SMALL), ink.link);
            let size = g.size() + vec2(16.0, 8.0);
            if x > 0.0 && x + size.x > inner {
                row_y += row_h + 4.0;
                x = 0.0;
                row_h = 0.0;
            }
            parts.push((row_y, Part::Reaction(g, x, size)));
            x += size.x + 4.0;
            row_h = row_h.max(size.y);
            want_w = want_w.max(x - 4.0);
        }
        y = row_y + row_h;
        last = Some((x - 4.0, y));
    }
    let text_bottom = match last {
        // Beside the last line when it fits, as Telegram does; under it when
        // it does not.
        Some((w, end)) if w + 12.0 + foot_w <= inner => {
            want_w = want_w.max(w + 12.0 + foot_w);
            parts.push((end - foot_h, Part::Foot));
            end
        }
        Some((_, end)) => {
            want_w = want_w.max(foot_w);
            parts.push((end, Part::Foot));
            end + foot_h
        }
        None => {
            want_w = want_w.max(foot_w);
            parts.push((y, Part::Foot));
            y + foot_h
        }
    };
    let bubble_w = (want_w + 2.0 * PAD_X).min(max_w);
    let bubble_h = text_bottom + PAD_Y;
    let height = bubble_h + if tail { AFTER_RUN } else { IN_RUN };
    if !paint {
        return (height, None);
    }

    // -- paint ----------------------------------------------------------
    let rect = Rect::from_min_size(pos2(x0, origin.y), vec2(bubble_w, bubble_h));
    let band = Rect::from_min_size(origin, vec2(width, bubble_h + IN_RUN));
    let selected = scene.selected == Some(r.id);
    if scene.hits.contains(&r.id) {
        painter.rect_filled(band, CornerRadius::ZERO, ink.hit);
    }
    let corners = CornerRadius {
        nw: RADIUS,
        ne: RADIUS,
        se: RADIUS,
        sw: if tail { TAIL_RADIUS } else { RADIUS },
    };
    let fill = if selected {
        ink.bubble_selected
    } else {
        ink.bubble
    };
    if !bare {
        painter.rect_filled(rect, corners, fill);
    }
    if selected {
        painter.rect_stroke(
            rect,
            corners,
            Stroke::new(1.5_f32, ink.link),
            egui::StrokeKind::Outside,
        );
    } else if r.deleted.is_some() {
        painter.rect_stroke(
            rect,
            corners,
            Stroke::new(1.0_f32, ink.deleted),
            egui::StrokeKind::Outside,
        );
    }

    // The bubble's own area — a click selects the message — goes in
    // **before** its parts: egui gives a click to the last area registered
    // under the pointer, so the reply quote (jump to the message it answers),
    // a picture and a card, all registered below, each win over it.
    let whole = ui.interact(rect, ui.id().with(("bubble", r.id)), Sense::click());
    let mut click = None;
    let inner_x = rect.left() + PAD_X;
    for (py, part) in parts {
        let at = pos2(inner_x, rect.top() + py);
        match part {
            Part::Text(g, dx) => painter.galley_e(at + vec2(dx, 0.0), g, ink.text),
            Part::Reply {
                to,
                who,
                what,
                colour,
                w,
            } => {
                let block = Rect::from_min_size(at, vec2(w, REPLY_H));
                painter.rect_filled(block, CornerRadius::same(6), colour.gamma_multiply(0.12));
                painter.rect_filled(
                    Rect::from_min_size(at, vec2(3.0, REPLY_H)),
                    CornerRadius {
                        nw: 6,
                        sw: 6,
                        ne: 0,
                        se: 0,
                    },
                    colour,
                );
                let clip = block.shrink2(vec2(4.0, 0.0));
                let p = painter.with_clip_rect(clip);
                p.galley_e(at + vec2(10.0, 4.0), who, colour);
                p.galley_e(at + vec2(10.0, 21.0), what, ink.text);
                let response = ui.interact(block, ui.id().with(("reply", r.id)), Sense::click());
                if response.clicked() {
                    click = Some(Click::Jump(to));
                }
                response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Go to the message this answers");
            }
            Part::Media {
                label,
                detail,
                fill,
                w,
            } => {
                let chip = Rect::from_min_size(at, vec2(w, MEDIA_H));
                let icon = Rect::from_min_size(at, vec2(MEDIA_H, MEDIA_H));
                painter.rect_filled(icon, CornerRadius::same(8), fill);
                let p = painter.with_clip_rect(chip);
                let tx = at.x + MEDIA_H + 10.0;
                match detail {
                    Some(d) => {
                        p.galley_e(pos2(tx, at.y + 5.0), label, ink.text);
                        p.galley_e(pos2(tx, at.y + 25.0), d, ink.meta);
                    }
                    None => p.galley_e(pos2(tx, at.y + 13.0), label, ink.text),
                }
            }
            Part::Foot => {
                let mut x = rect.right() - PAD_X - foot_w;
                for g in &foot {
                    let w = g.size().x;
                    painter.galley_e(pos2(x, at.y), g.clone(), ink.meta);
                    x += w + foot_gap;
                }
            }
            Part::Reaction(g, dx, size) => {
                let pill = Rect::from_min_size(at + vec2(dx, 0.0), size);
                painter.rect_filled(pill, CornerRadius::same(12), ink.link.gamma_multiply(0.16));
                painter.galley_e(pill.min + vec2(8.0, 4.0), g, ink.link);
            }
            Part::Picture {
                kind,
                size,
                badge,
                skipped,
            } => {
                let area = Rect::from_min_size(at, size);
                if paint_picture(ui, r.id, kind, area, badge, skipped, scene) {
                    click = Some(Click::Open(r.id));
                }
            }
            Part::Card {
                w,
                h,
                banner,
                icon,
                lines,
                wave,
                on_click,
            } => {
                let block = Rect::from_min_size(at, vec2(w, h));
                let mut top = at.y;
                if banner > 0.0 {
                    let tile = Rect::from_min_size(at, vec2(w, banner));
                    map_tile(painter, tile, &ink);
                    top += banner + 6.0;
                }
                let mut tx = at.x;
                if let Some(icon) = icon {
                    let c = pos2(at.x + MEDIA_H / 2.0, top + MEDIA_H / 2.0);
                    icon.paint(painter, c);
                    tx += MEDIA_H + 10.0;
                }
                let p = painter.with_clip_rect(block.expand(1.0));
                for (g, dy) in lines {
                    p.galley_e(pos2(tx, top + dy), g, ink.text);
                }
                if let Some((seed, ww)) = wave {
                    waveform(painter, pos2(tx, top + 4.0), ww, seed, ink.link);
                }
                if let Some(c) = on_click {
                    let response = ui.interact(block, ui.id().with(("card", r.id)), Sense::click());
                    if response.clicked() {
                        click = Some(c);
                    }
                    response.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
            }
            Part::Poll {
                w,
                question,
                sub,
                answers,
                total,
                total_y,
            } => {
                painter.galley_e(at, question.0, ink.text);
                painter.galley_e(at + vec2(0.0, question.1), sub, ink.meta);
                for a in answers {
                    let y0 = at.y + a.y;
                    painter.galley_e(pos2(at.x, y0), a.pct, ink.text);
                    painter.galley_e(pos2(at.x + POLL_PCT, y0), a.text, ink.text);
                    let bar_y = y0 + a.h + 4.0;
                    let full = w - POLL_PCT;
                    painter.rect_filled(
                        Rect::from_min_size(pos2(at.x + POLL_PCT, bar_y), vec2(full, 3.0)),
                        CornerRadius::same(2),
                        ink.meta.gamma_multiply(0.3),
                    );
                    painter.rect_filled(
                        Rect::from_min_size(
                            pos2(at.x + POLL_PCT, bar_y),
                            vec2((full * a.frac).max(3.0), 3.0),
                        ),
                        CornerRadius::same(2),
                        ink.link,
                    );
                    if a.chosen {
                        painter.circle_filled(
                            pos2(at.x + POLL_PCT - 8.0, bar_y + 1.5),
                            4.0,
                            ink.link,
                        );
                    }
                }
                painter.galley_e(at + vec2(0.0, total_y), total, ink.meta);
            }
        }
    }

    if tail {
        let c = pos2(origin.x + LEFT + AVATAR / 2.0, rect.bottom() - AVATAR / 2.0);
        if let Some(Pic::Ready(tex)) = scene.userpics.get(&r.from_id) {
            // Telegram's profile pictures are square, so the circle is the
            // whole picture with its corners rounded off.
            egui::Image::from_texture(tex)
                .corner_radius(CornerRadius::same((AVATAR / 2.0) as u8))
                .paint_at(ui, Rect::from_center_size(c, vec2(AVATAR, AVATAR)));
        } else {
            painter.circle_filled(c, AVATAR / 2.0, colour);
            let letters = if r.initials.is_empty() {
                "?".to_string()
            } else {
                r.initials.to_uppercase()
            };
            painter.text(
                c,
                Align2::CENTER_CENTER,
                letters,
                semibold(SMALL + 1.0),
                Color32::WHITE,
            );
        }
    }

    if click.is_none() && whole.clicked() {
        click = Some(Click::Select(r.id));
    }
    (height, click)
}

/// One laid-out piece of a bubble, at a height inside it.
enum Part {
    /// A galley, and how far right of the text's left edge it starts.
    Text(Arc<Galley>, f32),
    Reply {
        to: i64,
        who: Arc<Galley>,
        what: Arc<Galley>,
        colour: Color32,
        w: f32,
    },
    Media {
        label: Arc<Galley>,
        detail: Option<Arc<Galley>>,
        fill: Color32,
        w: f32,
    },
    /// The markers and the time, right-aligned.
    Foot,
    /// One reaction pill: its label, how far right, its size.
    Reaction(Arc<Galley>, f32, egui::Vec2),
    /// A photo, sticker, video frame or GIF, at its display size.
    Picture {
        kind: Visual,
        size: egui::Vec2,
        /// `GIF`, or a video's duration, over the top-left corner.
        badge: Option<Arc<Galley>>,
        /// The export left the file out, as opposed to the database lacking it.
        skipped: bool,
    },
    /// Anything drawn as an icon beside lines of text: a voice message, a
    /// song, a document, a contact, a location under its map, a game, an
    /// invoice, a die.
    Card {
        w: f32,
        h: f32,
        /// A map tile's height above the rest; 0 for none.
        banner: f32,
        icon: Option<Icon>,
        /// Each line and its offset below the top of the text column.
        lines: Vec<(Arc<Galley>, f32)>,
        /// A voice message's waveform: its seed and width.
        wave: Option<(i64, f32)>,
        on_click: Option<Click>,
    },
    Poll {
        w: f32,
        /// The question, and where the line under it starts.
        question: (Arc<Galley>, f32),
        sub: Arc<Galley>,
        answers: Vec<PollRow>,
        total: Arc<Galley>,
        total_y: f32,
    },
}

struct PollRow {
    y: f32,
    h: f32,
    pct: Arc<Galley>,
    text: Arc<Galley>,
    frac: f32,
    chosen: bool,
}

/// The round or square thing at the left of a [`Part::Card`].
enum Icon {
    /// A play button on a filled circle.
    Play(Color32),
    /// A file's extension on a rounded square of its colour.
    Doc(Arc<Galley>, Color32),
    /// A contact's initials on a userpic circle.
    Avatar(Arc<Galley>, Color32),
    /// A die's value on a rounded square.
    Die(Arc<Galley>, Color32),
}

impl Icon {
    fn paint(self, painter: &egui::Painter, c: Pos2) {
        let half = MEDIA_H / 2.0;
        match self {
            Icon::Play(fill) => {
                painter.circle_filled(c, half, fill);
                triangle(painter, c, 9.0, Color32::WHITE);
            }
            Icon::Doc(g, fill) | Icon::Die(g, fill) => {
                let r = Rect::from_center_size(c, vec2(MEDIA_H, MEDIA_H));
                painter.rect_filled(r, CornerRadius::same(8), fill);
                painter.galley_e(c - g.size() / 2.0, g, Color32::WHITE);
            }
            Icon::Avatar(g, fill) => {
                painter.circle_filled(c, half, fill);
                painter.galley_e(c - g.size() / 2.0, g, Color32::WHITE);
            }
        }
    }
}

/// Paint one picture into `area` — the texture, or its placeholder while it
/// loads, or a card saying why there is none — and say whether it was
/// clicked.
fn paint_picture(
    ui: &Ui,
    id: i64,
    kind: Visual,
    area: Rect,
    badge: Option<Arc<Galley>>,
    skipped: bool,
    scene: &Scene,
) -> bool {
    let ink = scene.ink;
    let painter = ui.painter();
    let radius = match kind {
        Visual::Round => CornerRadius::same((area.width() / 2.0).min(255.0) as u8),
        Visual::Sticker => CornerRadius::ZERO,
        _ => CornerRadius::same(8),
    };
    scene.painted.borrow_mut().push(id);
    let ready = match scene.pictures.get(&id) {
        Some(Pic::Ready(tex)) => {
            egui::Image::from_texture(tex)
                .corner_radius(radius)
                .paint_at(ui, area);
            true
        }
        Some(Pic::NoPreview) => {
            painter.rect_filled(area, radius, Color32::from_gray(24));
            if matches!(kind, Visual::Video | Visual::Round | Visual::Gif) {
                play_button(painter, area.center(), 22.0);
            } else {
                let g = painter.layout_e(
                    format!("{} — open to view", visual_name(kind)),
                    sans(SMALL),
                    ink.pill_text,
                    area.width() - 16.0,
                );
                let gs = g.size();
                painter.galley_e(area.center() - gs / 2.0, g, ink.pill_text);
            }
            false
        }
        Some(Pic::Missing) => {
            painter.rect_filled(area, radius, ink.pill.gamma_multiply(0.6));
            let what = if skipped {
                format!("{} — not included in the export", visual_name(kind))
            } else {
                format!("{} — not saved in the database", visual_name(kind))
            };
            let g = painter.layout_e(what, sans(SMALL), ink.pill_text, area.width() - 16.0);
            let gs = g.size();
            painter.galley_e(area.center() - gs / 2.0, g, ink.pill_text);
            false
        }
        _ => {
            painter.rect_filled(area, radius, ink.pill.gamma_multiply(0.45));
            false
        }
    };
    if ready && matches!(kind, Visual::Video | Visual::Round) {
        play_button(painter, area.center(), 22.0);
    }
    if let Some(b) = badge {
        let bs = b.size() + vec2(10.0, 4.0);
        let pill = Rect::from_min_size(area.min + vec2(6.0, 6.0), bs);
        painter.rect_filled(pill, CornerRadius::same(8), Color32::from_black_alpha(140));
        painter.galley_e(pill.min + vec2(5.0, 2.0), b, Color32::WHITE);
    }
    let response = ui.interact(area, ui.id().with(("pic", id)), Sense::click());
    let clicked = response.clicked();
    response.on_hover_cursor(egui::CursorIcon::PointingHand);
    clicked
}

/// The map tile's height over a location card.
const MAP_H: f32 = 110.0;
/// The percentage column of a poll.
const POLL_PCT: f32 = 46.0;

/// How big a picture is drawn, from the size it has — the decoded texture's,
/// or until then the payload's.
fn visual_size(kind: Visual, src: [u32; 2], inner: f32) -> egui::Vec2 {
    let (max_w, max_h, fallback) = match kind {
        Visual::Sticker => (180.0, 180.0, [180, 180]),
        Visual::Round => (200.0, 200.0, [200, 200]),
        _ => (inner.min(400.0), 400.0, [320, 220]),
    };
    let [w, h] = if src[0] == 0 || src[1] == 0 {
        fallback
    } else {
        src
    };
    let (w, h) = (w as f32, h as f32);
    let scale = (max_w / w).min(max_h / h);
    // A round video is a circle whatever its frame says.
    if kind == Visual::Round {
        return vec2(max_w, max_w);
    }
    vec2((w * scale).max(60.0), (h * scale).max(40.0))
}

fn visual_name(kind: Visual) -> &'static str {
    match kind {
        Visual::Photo | Visual::Image => "Photo",
        Visual::Sticker => "Sticker",
        Visual::Video => "Video",
        Visual::Gif => "GIF",
        Visual::Round => "Video message",
    }
}

fn duration_text(secs: i64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn triangle(painter: &egui::Painter, c: Pos2, r: f32, fill: Color32) {
    let pts = vec![
        pos2(c.x - r * 0.6, c.y - r),
        pos2(c.x + r, c.y),
        pos2(c.x - r * 0.6, c.y + r),
    ];
    painter.add(egui::Shape::convex_polygon(pts, fill, Stroke::NONE));
}

fn play_button(painter: &egui::Painter, c: Pos2, r: f32) {
    painter.circle_filled(c, r, Color32::from_black_alpha(130));
    triangle(painter, c + vec2(2.0, 0.0), r * 0.42, Color32::WHITE);
}

/// A voice message's bars. The export does not carry the waveform, so the
/// shape is drawn from the message id: the same message always looks the
/// same, and nobody mistakes it for a measurement.
fn waveform(painter: &egui::Painter, at: Pos2, w: f32, seed: i64, colour: Color32) {
    let bars = ((w / 4.0) as usize).max(8);
    let mut s = seed as u64 ^ 0x9e37_79b9_7f4a_7c15;
    for i in 0..bars {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let h = 4.0 + ((s >> 33) % 15) as f32;
        let x = at.x + i as f32 * 4.0;
        painter.rect_filled(
            Rect::from_min_size(pos2(x, at.y + 20.0 - h), vec2(2.0, h)),
            CornerRadius::same(1),
            colour,
        );
    }
}

/// A location's tile: no map is stored, so a stylised one — streets on a
/// ground colour, and the pin in the middle.
fn map_tile(painter: &egui::Painter, tile: Rect, ink: &Ink) {
    painter.rect_filled(
        tile,
        CornerRadius::same(8),
        name_colour(2).gamma_multiply(0.25),
    );
    let street = Stroke::new(3.0_f32, ink.bg.gamma_multiply(0.5));
    let p = painter.with_clip_rect(tile);
    for i in 1..4 {
        let x = tile.left() + tile.width() * i as f32 / 4.0;
        p.line_segment(
            [pos2(x - 20.0, tile.top()), pos2(x + 20.0, tile.bottom())],
            street,
        );
    }
    for i in 1..3 {
        let y = tile.top() + tile.height() * i as f32 / 3.0;
        p.line_segment([pos2(tile.left(), y), pos2(tile.right(), y + 10.0)], street);
    }
    let c = tile.center();
    painter.circle_filled(c - vec2(0.0, 10.0), 9.0, name_colour(1));
    painter.add(egui::Shape::convex_polygon(
        vec![
            c + vec2(-7.0, -5.0),
            c + vec2(7.0, -5.0),
            c + vec2(0.0, 8.0),
        ],
        name_colour(1),
        Stroke::NONE,
    ));
    painter.circle_filled(c - vec2(0.0, 10.0), 3.5, Color32::WHITE);
}

/// Lay out a message's attachment: the part, its width and its height.
fn attachment(ui: &Ui, r: &Row, a: &Attachment, scene: &Scene, inner: f32) -> (Part, f32, f32) {
    let painter = ui.painter();
    let ink = scene.ink;
    let text = |s: String, font: FontId, wrap: f32| painter.layout_e(s, font, ink.text, wrap);
    let meta = |s: String| painter.layout_e(s, sans(SMALL), ink.meta, inner - MEDIA_H - 10.0);
    // A card's text column: lines stacked, and the whole as tall as the
    // icon at least.
    let card = |icon: Option<Icon>,
                lines: Vec<Arc<Galley>>,
                banner: f32,
                on_click: Option<Click>,
                min_w: f32| {
        let mut y = if lines.len() == 1 { 13.0 } else { 3.0 };
        let mut w: f32 = min_w;
        let mut placed = Vec::new();
        for g in lines {
            w = w.max(g.size().x);
            let h = g.size().y;
            placed.push((g, y));
            y += h + 2.0;
        }
        let icon_w = if icon.is_some() { MEDIA_H + 10.0 } else { 0.0 };
        let total_w = (icon_w + w).min(inner);
        let body_h = (y + 3.0).max(if icon.is_some() { MEDIA_H } else { 0.0 });
        let h = banner + if banner > 0.0 { 6.0 } else { 0.0 } + body_h;
        (
            Part::Card {
                w: total_w,
                h,
                banner,
                icon,
                lines: placed,
                wave: None,
                on_click,
            },
            total_w,
            h,
        )
    };
    let col = inner - MEDIA_H - 10.0;
    match a {
        Attachment::Visual {
            kind,
            size,
            duration,
        } => {
            let src = match scene.pictures.get(&r.id) {
                Some(Pic::Ready(tex)) => {
                    let s = tex.size();
                    [s[0] as u32, s[1] as u32]
                }
                _ => *size,
            };
            let shown = visual_size(*kind, src, inner);
            let badge = match (kind, duration) {
                (Visual::Gif, _) => Some("GIF".to_string()),
                (Visual::Video | Visual::Round, Some(d)) => Some(duration_text(*d)),
                _ => None,
            }
            .map(|s| painter.layout_no_wrap_e(s, semibold(SMALL - 1.0), Color32::WHITE));
            (
                Part::Picture {
                    kind: *kind,
                    size: shown,
                    badge,
                    skipped: r.file_skipped,
                },
                shown.x,
                shown.y,
            )
        }
        Attachment::Voice { duration } => {
            let ww = col.min(200.0);
            let d = meta(duration.map(duration_text).unwrap_or_default());
            let (mut part, w, h) = card(
                Some(Icon::Play(ink.link)),
                vec![],
                0.0,
                Some(Click::Open(r.id)),
                ww,
            );
            if let Part::Card { lines, wave, .. } = &mut part {
                lines.push((d, 26.0));
                *wave = Some((r.id, ww));
            }
            (part, w, h)
        }
        Attachment::Audio {
            title,
            performer,
            duration,
        } => {
            let t = if title.is_empty() {
                r.media_detail.clone().unwrap_or_else(|| "Audio".into())
            } else {
                title.clone()
            };
            let mut sub = performer.clone();
            if let Some(d) = duration {
                if !sub.is_empty() {
                    sub.push_str("  ·  ");
                }
                sub.push_str(&duration_text(*d));
            }
            card(
                Some(Icon::Play(ink.link)),
                vec![text(t, semibold(BODY - 1.0), col), meta(sub)],
                0.0,
                Some(Click::Open(r.id)),
                0.0,
            )
        }
        Attachment::Document { name, size, ext } => {
            let label =
                painter.layout_no_wrap_e(ext.clone(), semibold(SMALL - 1.0), Color32::WHITE);
            let mut sub = size
                .map(|s| crate::list::human_bytes(s.max(0) as u64))
                .unwrap_or_default();
            if r.file_skipped {
                sub = "not included in the export".into();
            }
            card(
                Some(Icon::Doc(label, doc_colour(ext))),
                vec![text(name.clone(), semibold(BODY - 1.0), col), meta(sub)],
                0.0,
                Some(Click::Open(r.id)),
                0.0,
            )
        }
        Attachment::Contact {
            name,
            initials,
            phone,
        } => {
            let g = painter.layout_no_wrap_e(
                initials.to_uppercase(),
                semibold(SMALL + 1.0),
                Color32::WHITE,
            );
            card(
                Some(Icon::Avatar(
                    g,
                    name_colour(r.colour.wrapping_add(3) % 8 + 1),
                )),
                vec![
                    text(name.clone(), semibold(BODY - 1.0), col),
                    meta(phone.clone()),
                ],
                0.0,
                None,
                0.0,
            )
        }
        Attachment::Location {
            title,
            address,
            lat,
            lon,
            live,
        } => {
            let mut lines = vec![text(title.clone(), semibold(BODY - 1.0), inner)];
            if !address.is_empty() {
                lines.push(painter.layout_e(address.clone(), sans(SMALL), ink.meta, inner));
            }
            lines.push(painter.layout_e(
                format!("{lat}, {lon}{}", if *live { "  ·  live" } else { "" }),
                sans(SMALL),
                ink.meta,
                inner,
            ));
            let url = format!("https://maps.google.com/maps?q={lat},{lon}&ll={lat},{lon}&z=16");
            card(None, lines, MAP_H, Some(Click::Url(url)), inner.min(260.0))
        }
        Attachment::Game { title, description } => card(
            None,
            vec![
                painter.layout_no_wrap_e("Game".into(), semibold(SMALL), ink.link),
                text(title.clone(), semibold(BODY - 1.0), inner),
                text(description.clone(), sans(SMALL + 1.0), inner),
            ],
            0.0,
            None,
            0.0,
        ),
        Attachment::Invoice {
            title,
            description,
            amount,
        } => card(
            None,
            vec![
                painter.layout_no_wrap_e("Invoice".into(), semibold(SMALL), ink.link),
                text(title.clone(), semibold(BODY - 1.0), inner),
                text(description.clone(), sans(SMALL + 1.0), inner),
                painter.layout_no_wrap_e(amount.clone(), semibold(BODY - 1.0), ink.link),
            ],
            0.0,
            None,
            0.0,
        ),
        Attachment::Dice { emoji, value } => {
            let g =
                painter.layout_no_wrap_e(value.to_string(), semibold(BODY + 6.0), Color32::WHITE);
            card(
                Some(Icon::Die(g, name_colour(5))),
                vec![
                    text(
                        format!("{emoji}  {}", dice_name(emoji)),
                        semibold(BODY - 1.0),
                        col,
                    ),
                    meta(format!("rolled {value}")),
                ],
                0.0,
                None,
                0.0,
            )
        }
        Attachment::Poll(poll) => {
            let w = inner.min(340.0);
            let q = text(poll.question.clone(), semibold(BODY), w);
            let sub_y = q.size().y + 2.0;
            let sub = painter.layout_no_wrap_e(
                if poll.closed { "Final results" } else { "Poll" }.into(),
                sans(SMALL),
                ink.meta,
            );
            let mut y = sub_y + sub.size().y + 10.0;
            let most = poll
                .answers
                .iter()
                .map(|a| a.voters)
                .max()
                .unwrap_or(0)
                .max(1);
            let total = poll.total.max(1) as f32;
            let mut answers = Vec::new();
            for a in &poll.answers {
                let pct = (a.voters as f32 / total * 100.0).round();
                let pg =
                    painter.layout_no_wrap_e(format!("{pct}%"), semibold(SMALL + 1.0), ink.text);
                let tg = painter.layout_e(a.text.clone(), sans(BODY - 1.0), ink.text, w - POLL_PCT);
                let h = tg.size().y.max(pg.size().y);
                answers.push(PollRow {
                    y,
                    h,
                    pct: pg,
                    text: tg,
                    frac: a.voters as f32 / most as f32,
                    chosen: a.chosen,
                });
                y += h + 14.0;
            }
            let votes = match poll.total {
                1 => "1 vote".to_string(),
                n => format!("{n} votes"),
            };
            let tg = painter.layout_no_wrap_e(votes, sans(SMALL), ink.meta);
            let total_y = y;
            let h = y + tg.size().y;
            (
                Part::Poll {
                    w,
                    question: (q, sub_y),
                    sub,
                    answers,
                    total: tg,
                    total_y,
                },
                w,
                h,
            )
        }
    }
}

/// Desktop's names for the dice Telegram rolls.
fn dice_name(emoji: &str) -> &'static str {
    match emoji {
        "🎯" => "Darts",
        "🏀" => "Basketball",
        "⚽" => "Football",
        "🎳" => "Bowling",
        "🎰" => "Slot machine",
        _ => "Dice",
    }
}

/// A file's colour by its kind, as Telegram tints its document icons.
fn doc_colour(ext: &str) -> Color32 {
    match ext {
        "PDF" => name_colour(1),
        "DOC" | "DOCX" | "TXT" | "RTF" | "ODT" => name_colour(4),
        "XLS" | "XLSX" | "CSV" | "ODS" => name_colour(2),
        "ZIP" | "RAR" | "7Z" | "TAR" | "GZ" => name_colour(8),
        "PPT" | "PPTX" | "KEY" => name_colour(3),
        "APK" | "EXE" | "MSI" => name_colour(5),
        _ => name_colour(7),
    }
}

/// `[voice message]` as `Voice message`.
fn media_name(label: &str) -> &str {
    match label.trim_matches(['[', ']']) {
        "photo" => "Photo",
        "video file" => "Video",
        "video message" => "Video message",
        "voice message" => "Voice message",
        "audio file" => "Audio",
        "animation" => "GIF",
        "sticker" => "Sticker",
        "poll" => "Poll",
        "location" => "Location",
        "contact" => "Contact",
        other => other,
    }
}

/// Desktop's colour for a kind of media, from the same stylesheet's
/// `.media_* .fill`.
fn media_colour(label: &str) -> Color32 {
    match media_name(label) {
        "Photo" => name_colour(2),
        "Video" | "Video message" | "GIF" | "Location" => name_colour(7),
        "Voice message" | "Audio" => name_colour(4),
        "Sticker" => name_colour(3),
        "Contact" => name_colour(8),
        "Poll" => name_colour(5),
        _ => name_colour(1),
    }
}

/// `2025-12-18` as `18 December 2025`, Desktop's separator.
fn day_label(day: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let mut p = day.split('-');
    let (Some(y), Some(m), Some(d)) = (p.next(), p.next(), p.next()) else {
        return day.to_string();
    };
    let month = m
        .parse::<usize>()
        .ok()
        .and_then(|m| MONTHS.get(m.wrapping_sub(1)));
    match (d.parse::<u32>(), month) {
        (Ok(d), Some(month)) => format!("{d} {month} {y}"),
        _ => day.to_string(),
    }
}

fn sans(size: f32) -> FontId {
    tgx_ui::fonts::sans(size)
}

fn semibold(size: f32) -> FontId {
    tgx_ui::fonts::semibold(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which of two overlapping click areas a click inside both reaches,
    /// with `outer` registered first or last.
    fn clicked(outer_first: bool) -> (bool, bool) {
        let ctx = egui::Context::default();
        let outer = Rect::from_min_size(pos2(10.0, 10.0), vec2(200.0, 100.0));
        let inner = Rect::from_min_size(pos2(20.0, 20.0), vec2(80.0, 30.0));
        let at = pos2(40.0, 30.0);
        let mut got = (false, false);
        let frames = [
            vec![],
            vec![egui::Event::PointerMoved(at)],
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ];
        for events in frames {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let add = |rect, name: &str| {
                        ui.interact(rect, ui.id().with(name), Sense::click())
                            .clicked()
                    };
                    let (o, i) = if outer_first {
                        let o = add(outer, "outer");
                        (o, add(inner, "inner"))
                    } else {
                        let i = add(inner, "inner");
                        (add(outer, "outer"), i)
                    };
                    got.0 |= o;
                    got.1 |= i;
                });
            });
        }
        got
    }

    /// **The last area registered under the pointer takes the click.** The
    /// bubble's own area (select the message) once came after its parts, so
    /// a click on a reply's quote — meant to jump to the message it answers —
    /// or on a picture selected the bubble instead.
    #[test]
    fn a_part_registered_after_the_bubble_takes_the_click() {
        assert_eq!(
            clicked(false),
            (true, false),
            "registered last, the bubble swallows it"
        );
        assert_eq!(
            clicked(true),
            (false, true),
            "registered first, the part gets it"
        );
    }

    fn row(id: i64, from: &str, time: &str) -> Row {
        Row {
            id,
            topic_id: 1,
            date: format!("2025-12-18 {time}"),
            day: "2025-12-18".into(),
            time: time.into(),
            from: from.into(),
            from_id: format!("user{}", from.len()),
            colour: 1,
            initials: from.get(..1).unwrap_or("").into(),
            text: "x".into(),
            preview: "x".into(),
            media: None,
            media_detail: None,
            attachment: None,
            file_skipped: false,
            reactions: Vec::new(),
            reply_to: None,
            forwarded: None,
            edited_at: None,
            deleted: None,
            version: 1,
            recovered: false,
            service: false,
        }
    }

    #[test]
    fn a_run_is_one_sender_close_together_on_one_day() {
        let mut rows = vec![
            row(1, "Ana", "10:00"),
            row(2, "Ana", "10:05"),
            row(3, "Marko", "10:06"),
            row(4, "Marko", "11:30"),
        ];
        rows.push(Row {
            day: "2025-12-19".into(),
            ..row(5, "Marko", "11:31")
        });
        let shown: Vec<usize> = (0..rows.len()).collect();
        let (items, item_of) = build(&rows, &shown);
        let msg = |i: usize| match &items[item_of[i]] {
            Item::Msg { head, tail, .. } => (*head, *tail),
            Item::Day(_) => panic!("not a message"),
        };
        assert_eq!(items[0], Item::Day("2025-12-18".into()));
        assert_eq!(msg(0), (true, false), "Ana opens a run");
        assert_eq!(msg(1), (false, true), "and closes it");
        assert_eq!(msg(2), (true, true), "Marko alone: 84 minutes to the next");
        assert_eq!(msg(3), (true, true));
        assert_eq!(items[item_of[4] - 1], Item::Day("2025-12-19".into()));
        assert_eq!(msg(4), (true, true), "a new day starts a new run");
    }

    #[test]
    fn a_day_reads_as_desktop_writes_it() {
        assert_eq!(day_label("2025-12-08"), "8 December 2025");
        assert_eq!(day_label("garbage"), "garbage");
    }
}
