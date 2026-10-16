//! Text selection as the window does it: chrome selects nothing, content
//! selects like Windows — a double-click takes one word.
//!
//! Driven headless through egui's own input, so it holds the behaviour and
//! not a guess at it. The theme has to be installed: `eframe` is built
//! without egui's default fonts, and with no font every label is zero wide.

use eframe::egui::{self, Event, PointerButton, Pos2, RawInput};
use tgx_ui::{components::selectable, theme, tokens::Palette};

const TEXT: &str = "Ana: što je bilo juče";

/// One frame of a label, inside `selectable` or not. Its rect, and whatever
/// was copied.
fn frame(ctx: &egui::Context, t: f64, events: Vec<Event>, content: bool) -> (egui::Rect, String) {
    let input = RawInput {
        time: Some(t),
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(600.0, 200.0),
        )),
        events,
        ..Default::default()
    };
    let mut rect = egui::Rect::NOTHING;
    let out = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            rect = if content {
                selectable(ui, |ui| ui.label(TEXT)).rect
            } else {
                ui.label(TEXT).rect
            };
        });
    });
    let copied = out
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    (rect, copied)
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

/// Double-click the `at`th character, then Ctrl+C.
fn double_click(content: bool, at: usize) -> String {
    let ctx = egui::Context::default();
    theme::install(&ctx, &Palette::dark());
    let mut rect = egui::Rect::NOTHING;
    for i in 0..3 {
        rect = frame(&ctx, f64::from(i) * 0.01, vec![], content).0;
    }
    let font = egui::TextStyle::Body.resolve(&ctx.style());
    let galley = ctx.fonts_mut(|f| f.layout_no_wrap(TEXT.into(), font, egui::Color32::WHITE));
    let x = galley
        .pos_from_cursor(egui::text::CCursor::new(at))
        .center()
        .x;
    let pos = Pos2::new(rect.left() + x, rect.center().y);
    let mut t = 0.1;
    let mut step = |events: Vec<Event>| {
        t += 0.02;
        frame(&ctx, t, events, content).1
    };
    step(vec![Event::PointerMoved(pos)]);
    step(vec![button(pos, true)]);
    step(vec![button(pos, false)]);
    step(vec![button(pos, true)]);
    step(vec![button(pos, false)]);
    step(vec![]);
    step(vec![Event::Copy])
}

#[test]
fn a_double_click_in_content_takes_one_word() {
    assert_eq!(double_click(true, 1), "Ana");
    assert_eq!(double_click(true, 6), "što");
    assert_eq!(double_click(true, 19), "juče");
}

#[test]
fn the_chrome_selects_nothing() {
    assert_eq!(double_click(false, 6), "");
}
