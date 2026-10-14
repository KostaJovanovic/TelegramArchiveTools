//! One window per data folder.
//!
//! **Two windows on one `TelegramToolsData/` share everything that must not be
//! shared:** the session database grammers writes authorisation keys into,
//! `settings.json` (the last window to close wins), `tgx.log` — which a second
//! launch *rotates*, pushing the first window's session into `tgx.prev.log`
//! while it is still being written — and, in Database mode, the one
//! `.sqlite` per chat that two syncs would both be merging into.
//!
//! So a second launch raises the first window and leaves. Keyed on the data
//! folder, not on the program: two copies in two folders have two of
//! everything and are free to run side by side.

/// Claim the window for this data folder. `false` means another window
/// already has it, and has been brought to the front.
///
/// Called before the log opens, so the second launch rotates nothing.
pub fn claim(data_dir: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        imp::claim(&name(data_dir))
    }
    #[cfg(not(windows))]
    {
        let _ = data_dir;
        true
    }
}

/// Is a window already open on this data folder? Asks without claiming, so a
/// command run beside nothing does not stop a window opening afterwards.
///
/// For the command line's syncs: they would be a second writer into the
/// databases the window may be merging into.
pub fn window_open(data_dir: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        imp::exists(&name(data_dir))
    }
    #[cfg(not(windows))]
    {
        let _ = data_dir;
        false
    }
}

/// The mutex's name: `Local\` (this sign-in session only) and a hash of the
/// folder, lowercased because Windows paths are case-insensitive and two
/// spellings of one folder are one folder.
fn name(data_dir: &std::path::Path) -> String {
    let path = data_dir.to_string_lossy().to_lowercase();
    // FNV-1a: stable across builds and Rust versions, which `DefaultHasher`
    // does not promise — two builds of this app must agree on the name.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("Local\\TelegramTools-{h:016x}")
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn claim(name: &str) -> bool {
        let name = wide(name);
        // SAFETY: a null security descriptor and a NUL-terminated name. The
        // handle is deliberately never closed: the mutex is the claim, and it
        // has to last as long as the process does.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        // SAFETY: read straight after the call it describes.
        let taken = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if handle.is_null() {
            // Could not ask at all. Running is better than refusing over a
            // guard.
            return true;
        }
        if taken {
            raise();
        }
        !taken
    }

    /// Whether the mutex `claim` makes exists. `OpenMutexW` never creates
    /// one, so asking leaves nothing behind.
    pub fn exists(name: &str) -> bool {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
        let name = wide(name);
        // SAFETY: a NUL-terminated name; the handle is closed straight away.
        unsafe {
            let handle = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr());
            if handle.is_null() {
                return false;
            }
            CloseHandle(handle);
        }
        true
    }

    /// Bring the window that is already open to the front. By title, which
    /// is the app's name and nothing else; finding nothing is fine.
    fn raise() {
        let title = wide(crate::TITLE);
        // SAFETY: a null class and a NUL-terminated title; the handle is
        // checked before use.
        unsafe {
            let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
            if !hwnd.is_null() {
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                SetForegroundWindow(hwnd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn one_folder_is_one_name_whatever_its_case() {
        assert_eq!(
            name(Path::new(r"C:\Tools\TelegramToolsData")),
            name(Path::new(r"c:\tools\telegramtoolsdata"))
        );
    }

    #[test]
    fn two_folders_are_two_windows() {
        assert_ne!(
            name(Path::new(r"C:\A\TelegramToolsData")),
            name(Path::new(r"D:\A\TelegramToolsData"))
        );
    }

    #[test]
    #[cfg(windows)]
    fn asking_whether_a_window_is_open_does_not_claim_one() {
        // A folder no real window uses, so the test cannot collide with one.
        let dir = Path::new(r"Z:\tt-app-test\window-open-probe");
        assert!(!window_open(dir));
        assert!(!window_open(dir), "the first ask must not have created it");
        assert!(claim(dir));
        assert!(window_open(dir));
    }

    #[test]
    fn the_name_stays_in_this_sign_in_session() {
        assert!(name(Path::new("x")).starts_with("Local\\"));
    }
}
