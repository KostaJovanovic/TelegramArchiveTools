//! A Windows notification when a run finishes, for someone who has walked
//! away from the window.
//!
//! **A notification-area balloon, not a WinRT toast.** A toast needs an
//! AppUserModelID registered with a Start-menu shortcut, which a portable exe
//! copied onto a stick does not have; the usual workaround borrows
//! PowerShell's, and the notification then says it came from PowerShell. A
//! balloon on Windows 10 and 11 is shown as a toast anyway, named and drawn
//! with this exe's own icon, and needs nothing but the window handle. The
//! icon it needs in the notification area is added for the balloon and
//! taken away again.
//!
//! Nothing here is allowed to fail loudly: the taskbar flash has already
//! happened, and a notification that cannot be shown is a notification not
//! shown.

/// Show `body` as "TelegramTools — run finished".
pub fn run_finished(frame: &eframe::Frame, body: &str) {
    #[cfg(windows)]
    if let Some(hwnd) = hwnd(frame) {
        imp::balloon(hwnd, "Export finished", body);
    }
    #[cfg(not(windows))]
    let _ = (frame, body);
}

#[cfg(windows)]
fn hwnd(frame: &eframe::Frame) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIIF_LARGE_ICON, NIM_ADD,
        NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_APPLICATION};

    /// Which notification-area icon of this window it is. One is all there
    /// ever is.
    const ID: u32 = 0x7454;
    /// How long the icon stays once the balloon is up. Long enough for
    /// Windows to have shown it; the notification itself then lives in the
    /// notification centre on its own.
    const SHOWN_FOR: std::time::Duration = std::time::Duration::from_secs(12);

    /// Copy `s` into a fixed UTF-16 field, cut to fit and NUL-terminated.
    fn put(dst: &mut [u16], s: &str) {
        let room = dst.len() - 1;
        let mut n = 0;
        for unit in s.encode_utf16().take(room) {
            dst[n] = unit;
            n += 1;
        }
        dst[n] = 0;
    }

    fn data(hwnd: isize) -> NOTIFYICONDATAW {
        // SAFETY: an all-zero NOTIFYICONDATAW is valid — every field is an
        // integer, a handle or an array of them.
        let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        d.hWnd = hwnd as _;
        d.uID = ID;
        d
    }

    pub fn balloon(hwnd: isize, title: &str, body: &str) {
        let mut d = data(hwnd);
        d.uFlags = NIF_ICON | NIF_TIP | NIF_INFO;
        // SAFETY: both calls take a module handle or null and a resource id;
        // a missing resource returns null, which is checked.
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            // `1` is the icon `build.rs` embeds (winresource's default id).
            let own = LoadIconW(module, 1 as _);
            d.hIcon = if own.is_null() {
                LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
            } else {
                own
            };
        }
        put(&mut d.szTip, crate::TITLE);
        put(&mut d.szInfoTitle, title);
        put(&mut d.szInfo, body);
        d.dwInfoFlags = NIIF_INFO | NIIF_LARGE_ICON;
        // SAFETY: `d` is a fully initialised NOTIFYICONDATAW of the size it
        // declares.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &d) } == 0 {
            log::warn!("could not show the run-finished notification");
            return;
        }
        std::thread::spawn(move || {
            std::thread::sleep(SHOWN_FOR);
            let d = data(hwnd);
            // SAFETY: as above; deleting needs only the window and the id.
            unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
        });
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn a_long_body_is_cut_to_the_field_and_terminated() {
            let mut field = [0xFFFFu16; 8];
            super::put(&mut field, "a sentence far longer than eight");
            assert_eq!(field[7], 0);
            assert_eq!(String::from_utf16_lossy(&field[..7]), "a sente");
        }
    }
}
