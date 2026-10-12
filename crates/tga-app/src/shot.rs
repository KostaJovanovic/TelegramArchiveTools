//! The window photographs itself, for `tools/shot.ps1`.
//!
//! **Reading the screen from outside does not work on this window**, and the
//! exporter found out the hard way: eframe draws with OpenGL, an unoccluded GL
//! surface reaches the display without passing through the compositor, and
//! `CopyFromScreen` comes back black. So the app takes the picture: egui is
//! asked for the frame it just drew, which is written as raw RGBA — `TGXS`,
//! then width and height as little-endian `u32`s, the exporter's format, so
//! one script reads both — and the window closes.
//!
//! - `TGA_SHOT=<file>` turns it on.
//! - `TGA_SHOT_PATH=<folder>` is what the path field is pointed at first.
//! - `TGA_SHOT_SIZE=WxH` opens the window at that size, so the floor — where
//!   layouts break — can be photographed and not only the default.
//!
//! Nothing here runs unless `TGA_SHOT` is set.

use eframe::egui::{self, Context};

/// Frames to wait once the window has nothing left to fetch, so a layout that
/// settles over a frame or two is photographed settled.
const SETTLE: u32 = 3;

pub fn path() -> Option<String> {
    std::env::var("TGA_SHOT").ok().filter(|s| !s.is_empty())
}

pub fn folder() -> Option<String> {
    std::env::var("TGA_SHOT_PATH")
        .ok()
        .filter(|s| !s.is_empty() && path().is_some())
}

pub fn size() -> Option<[f32; 2]> {
    let text = std::env::var("TGA_SHOT_SIZE").ok()?;
    let (w, h) = text.split_once('x')?;
    Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
}

/// Ask on the settling frame, write on the frame the answer arrives.
///
/// `quiet` counts frames since the window last had something in flight.
pub fn tick(ctx: &Context, quiet: u32, to: &str) {
    if quiet == SETTLE {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
    let shot = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    });
    let Some(image) = shot else {
        // Keep frames coming: egui repaints on demand, and a window nobody is
        // touching would otherwise sit still and never reach `SETTLE`.
        ctx.request_repaint();
        return;
    };
    let [w, h] = [image.width() as u32, image.height() as u32];
    let mut out = Vec::with_capacity(12 + (w * h * 4) as usize);
    out.extend_from_slice(b"TGXS");
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    for px in image.pixels.iter() {
        out.extend_from_slice(&px.to_srgba_unmultiplied());
    }
    if let Err(e) = std::fs::write(to, &out) {
        eprintln!("could not write {to}: {e}");
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
}

#[cfg(test)]
mod tests {
    #[test]
    fn nothing_is_photographed_unless_asked() {
        // The hook must be inert in an ordinary launch; the test environment
        // does not set it.
        if std::env::var_os("TGA_SHOT").is_none() {
            assert_eq!(super::path(), None);
            assert_eq!(super::folder(), None);
        }
    }
}
