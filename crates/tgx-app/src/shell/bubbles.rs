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
use tgx_tg::viewer::Row;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Click {
    Select(i64),
    /// The reply block: go to the message it answers.
    Jump(i64),
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
                return 30.0 + 18.0 * (lines(&r.text) - 1.0).max(0.0);
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
            if r.media.is_some() {
                h += MEDIA_H + 6.0;
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
        .layout(label.to_string(), sans(SMALL + 0.5), ink.pill_text, wrap);
    let size = galley.size() + vec2(20.0, 8.0);
    let top = origin.y + 8.0;
    let rect = Rect::from_min_size(pos2(origin.x + (width - size.x) / 2.0, top), size);
    let height = size.y + 16.0;
    if !paint {
        return (height, None);
    }
    let mut click = None;
    if let Some(r) = row {
        let band = Rect::from_min_size(origin, vec2(width, height));
        let hit = scene.hits.contains(&r.id);
        if hit {
            ui.painter().rect_filled(band, CornerRadius::ZERO, ink.hit);
        }
        let response = ui.interact(rect, ui.id().with(("svc", r.id)), Sense::click());
        if response.clicked() {
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
    painter.galley(rect.min + vec2(10.0, 4.0), galley, ink.pill_text);
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
        let g = painter.layout(name.to_string(), semibold(BODY - 0.5), colour, inner);
        let mut w = g.size().x;
        let h = g.size().y;
        parts.push((y, Part::Text(g, 0.0)));
        if let Some(t) = scene.topic_names.and_then(|n| n.get(&r.topic_id)) {
            let tg = painter.layout_no_wrap(format!("  ·  {t}"), sans(SMALL), ink.meta);
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
        let g = painter.layout(format!("Forwarded from {f}"), sans(SMALL), ink.link, inner);
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
        let wg = painter.layout_no_wrap(who, semibold(SMALL), c);
        let tg = painter.layout_no_wrap(what, sans(SMALL), ink.text);
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
    if let Some(m) = &r.media {
        let lg = painter.layout_no_wrap(media_name(m).to_string(), semibold(SMALL + 1.0), ink.text);
        let dg = r
            .media_detail
            .as_ref()
            .map(|d| painter.layout_no_wrap(d.clone(), sans(SMALL), ink.meta));
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
        .map(|(s, c)| painter.layout_no_wrap(s, sans(SMALL - 0.5), c))
        .collect();
    let foot_gap = 6.0;
    let foot_w: f32 =
        foot.iter().map(|g| g.size().x).sum::<f32>() + foot_gap * (foot.len() - 1) as f32;
    let foot_h = foot.iter().map(|g| g.size().y).fold(0.0, f32::max);

    let text_bottom;
    if r.text.is_empty() {
        want_w = want_w.max(foot_w);
        text_bottom = y + foot_h;
        parts.push((y, Part::Foot));
    } else {
        let g = painter.layout(r.text.clone(), sans(BODY), ink.text, inner);
        let last = g.rows.last().map(|row| row.rect().width()).unwrap_or(0.0);
        let text_w = g.size().x;
        let h = g.size().y;
        parts.push((y, Part::Text(g, 0.0)));
        // Beside the last line when it fits, as Telegram does; under it when
        // it does not.
        if last + 12.0 + foot_w <= inner {
            want_w = want_w.max(text_w.max(last + 12.0 + foot_w));
            text_bottom = y + h;
            parts.push((y + h - foot_h, Part::Foot));
        } else {
            want_w = want_w.max(text_w.max(foot_w));
            text_bottom = y + h + foot_h;
            parts.push((y + h, Part::Foot));
        }
    }
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
    painter.rect_filled(rect, corners, fill);
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

    let mut click = None;
    let inner_x = rect.left() + PAD_X;
    for (py, part) in parts {
        let at = pos2(inner_x, rect.top() + py);
        match part {
            Part::Text(g, dx) => painter.galley(at + vec2(dx, 0.0), g, ink.text),
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
                p.galley(at + vec2(10.0, 4.0), who, colour);
                p.galley(at + vec2(10.0, 21.0), what, ink.text);
                let response = ui.interact(block, ui.id().with(("reply", r.id)), Sense::click());
                if response.clicked() {
                    click = Some(Click::Jump(to));
                }
                response.on_hover_cursor(egui::CursorIcon::PointingHand);
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
                        p.galley(pos2(tx, at.y + 5.0), label, ink.text);
                        p.galley(pos2(tx, at.y + 25.0), d, ink.meta);
                    }
                    None => p.galley(pos2(tx, at.y + 13.0), label, ink.text),
                }
            }
            Part::Foot => {
                let mut x = rect.right() - PAD_X - foot_w;
                for g in &foot {
                    let w = g.size().x;
                    painter.galley(pos2(x, at.y), g.clone(), ink.meta);
                    x += w + foot_gap;
                }
            }
        }
    }

    if tail {
        let c = pos2(origin.x + LEFT + AVATAR / 2.0, rect.bottom() - AVATAR / 2.0);
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

    if click.is_none() {
        let response = ui.interact(rect, ui.id().with(("bubble", r.id)), Sense::click());
        if response.clicked() {
            click = Some(Click::Select(r.id));
        }
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
