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
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HWND};
    use windows_sys::Win32::System::Threading::{
        CreateMutexW, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, GetWindowThreadProcessId, IsIconic, SetForegroundWindow, ShowWindow,
        SW_RESTORE,
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

    /// Bring the window that is already open to the front; finding nothing
    /// is fine.
    ///
    /// **By title and by whose window it is.** The title alone is the app's
    /// name, and so is this repository's folder: with it open in Explorer, a
    /// second launch raised Explorer and left the app behind it. So every
    /// top-level window with the title is asked which program owns it, and
    /// the one this program owns wins — see [`super::rank`].
    fn raise() {
        let Ok(ours) = std::env::current_exe() else {
            return;
        };
        let title = wide(crate::TITLE);
        let mut best: Option<(u8, HWND)> = None;
        let mut after: HWND = std::ptr::null_mut();
        // SAFETY: a null parent (top-level windows only), a null class and a
        // NUL-terminated title; every handle is checked before use. Bounded,
        // because a window list that changes under the walk must not loop.
        unsafe {
            for _ in 0..64 {
                after = FindWindowExW(
                    std::ptr::null_mut(),
                    after,
                    std::ptr::null(),
                    title.as_ptr(),
                );
                if after.is_null() {
                    break;
                }
                let mut pid = 0u32;
                GetWindowThreadProcessId(after, &mut pid);
                let rank = image(pid).map_or(0, |theirs| super::rank(&ours, &theirs));
                if rank > best.map_or(0, |(r, _)| r) {
                    best = Some((rank, after));
                }
            }
            if let Some((_, hwnd)) = best {
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                SetForegroundWindow(hwnd);
            }
        }
    }

    /// The executable a process was started from.
    fn image(pid: u32) -> Option<std::path::PathBuf> {
        // SAFETY: a plain open with the least access that can name the image;
        // the handle is closed below.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let mut name = [0u16; 1024];
        let mut len = name.len() as u32;
        // SAFETY: the buffer's length is what is passed.
        let ok = unsafe {
            QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, name.as_mut_ptr(), &mut len)
        };
        // SAFETY: opened above, closed once.
        unsafe { CloseHandle(handle) };
        (ok != 0).then(|| std::path::PathBuf::from(String::from_utf16_lossy(&name[..len as usize])))
    }
}

/// How surely a window owned by the program at `theirs` is this app's, given
/// that this program is `ours`: 2 for the same executable, 1 for one of the same
/// name elsewhere, 0 for anything else — Explorer showing a folder called
/// TelegramTools, say.
///
/// A copy elsewhere still counts, below the exact one, because the claim is
/// per data folder and not per file: a debug and a release build under
/// `target\` share the workspace's.
#[cfg_attr(not(windows), allow(dead_code))]
fn rank(ours: &std::path::Path, theirs: &std::path::Path) -> u8 {
    let fold = |p: &std::path::Path| p.to_string_lossy().to_lowercase();
    let name = |p: &std::path::Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
    if fold(ours) == fold(theirs) {
        2
    } else if name(ours).is_some() && name(ours) == name(theirs) {
        1
    } else {
        0
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
    fn a_window_titled_like_the_app_is_raised_only_if_the_app_owns_it() {
        let ours = Path::new(r"C:\Tools\TelegramTools.exe");
        // Explorer showing a folder named TelegramTools has the same title.
        assert_eq!(rank(ours, Path::new(r"C:\Windows\explorer.exe")), 0);
        assert_eq!(rank(ours, Path::new(r"c:\tools\telegramtools.EXE")), 2);
        // A debug and a release build share the workspace's data folder.
        assert_eq!(rank(ours, Path::new(r"C:\Other\TelegramTools.exe")), 1);
    }

    #[test]
    fn the_name_stays_in_this_sign_in_session() {
        assert!(name(Path::new("x")).starts_with("Local\\"));
    }
}
