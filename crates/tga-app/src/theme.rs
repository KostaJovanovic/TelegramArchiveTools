//! The window's design: TelegramExporter's, applied whole.
//!
//! **It used to be its own.** This module carried a palette, a type scale and
//! a `theme::install` of its own, and the exporter carried a near-copy that
//! grew rows, menus, a nav bar and three type roles this one never had. Two
//! windows meant to read as one product read as two. So the design now comes
//! from `tgx-ui` — see the workspace manifest for why that is a copy — and what
//! is left here is the two things that are this window's alone.
//!
//! **Dark only**, still. The exporter has a light appearance because it has a
//! stored setting for one; this window has no settings file, and the report it
//! writes is dark. A second appearance is a second design to keep in step.

use eframe::egui;
use tgx_ui::Palette;

/// The one appearance.
pub const PALETTE: Palette = Palette::dark();

/// The size it opens at: room for the library's columns without a scrollbar
/// across a whole chat title.
pub const WINDOW: [f32; 2] = [1040.0, 680.0];

/// The floor, which is the exporter's: the two windows sit side by side.
pub fn min_window() -> [f32; 2] {
    let (w, h) = tgx_ui::min_window();
    [w, h]
}

pub fn install(ctx: &egui::Context) {
    tgx_ui::install(ctx, &PALETTE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;

    /// The report's own value for a token, as it writes it into the stylesheet.
    fn from_report(name: &str) -> Color32 {
        let text = tga_report::palette::token(name);
        let rgb = u32::from_str_radix(text.trim_start_matches('#'), 16).expect("hex");
        tgx_ui::tokens::hex(rgb)
    }

    #[test]
    fn the_window_and_the_report_name_the_same_colours() {
        // The window's palette now comes from the exporter and the report's
        // from its own stylesheet. Three copies of one design is what this test
        // is for: if the exporter's dark palette moves, the report it writes
        // here and the window that wrote it stop being one product, and this
        // is where that shows.
        for (name, ours) in [
            ("bg", PALETTE.bg),
            ("fg", PALETTE.fg),
            ("muted", PALETTE.muted),
            ("hairline", PALETTE.hairline),
            ("rule", PALETTE.rule),
            ("surface", PALETTE.surface),
            ("accent", PALETTE.accent),
            ("accent_fg", PALETTE.accent_fg),
        ] {
            assert_eq!(
                ours,
                from_report(name),
                "{name} has drifted from the report's stylesheet"
            );
        }
    }

    #[test]
    fn it_opens_no_smaller_than_its_floor() {
        let floor = min_window();
        assert!(WINDOW[0] >= floor[0] && WINDOW[1] >= floor[1]);
    }
}
