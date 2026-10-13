//! What the destination's drive is, before a run writes to it.
//!
//! Three things a run cannot fix once it has started, each said in the log
//! when the run starts rather than discovered halfway through it:
//!
//! - **Little free space.** A busy group's media runs to tens of gigabytes,
//!   and a full disk part-way through leaves a folder that is valid as far as
//!   it goes and a log full of failed files.
//! - **FAT32**, which cannot hold a file over 4 GB, and **exFAT**, which has
//!   no permissions at all — an export there is readable by anyone who has
//!   the drive.
//! - **A network drive**, which is slow for a folder export and unsafe for a
//!   database: SQLite's locking is not reliable over a network share, and a
//!   sync interrupted there can damage the file.
//!
//! Warnings, never refusals: someone exporting text only onto a nearly full
//! stick knows better than this module does.

use tgx_tg::config::Settings;

/// Below this much free space a run with media is warned about.
const LOW_SPACE: u64 = 2 * 1024 * 1024 * 1024;

/// What was learnt about the destination's drive. Every field is `None` or
/// `false` when it could not be asked.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Volume {
    /// The drive's root, for naming it: `D:\`.
    pub root: Option<String>,
    pub free: Option<u64>,
    /// `NTFS`, `FAT32`, `exFAT`, …
    pub file_system: Option<String>,
    pub remote: bool,
}

/// Ask Windows about the drive `dir` is on. `dir` need not exist yet — the
/// first export makes it — so the nearest folder that does is asked instead.
pub fn inspect(dir: &std::path::Path) -> Volume {
    let mut at = dir;
    while !at.exists() {
        match at.parent() {
            Some(up) => at = up,
            None => return Volume::default(),
        }
    }
    #[cfg(windows)]
    {
        imp::inspect(at)
    }
    #[cfg(not(windows))]
    {
        let _ = at;
        Volume::default()
    }
}

/// What to say about `v` before a run with these settings.
pub fn warnings(v: &Volume, settings: &Settings) -> Vec<String> {
    let drive = v.root.as_deref().unwrap_or("the destination drive");
    let mut out = Vec::new();
    if let Some(free) = v.free {
        if settings.download_media && free < LOW_SPACE {
            out.push(format!(
                "Only {} free on {drive}. A chat's media can run to tens of \
                 gigabytes; files that do not fit are listed as missing.",
                gigabytes(free)
            ));
        }
    }
    match v.file_system.as_deref().map(str::to_ascii_uppercase).as_deref() {
        Some("FAT32" | "FAT") => out.push(format!(
            "{drive} is FAT32, which cannot hold a file over 4 GB — larger \
             files will fail{}.",
            if settings.export_db {
                ", and a chat's database cannot grow past it"
            } else {
                ""
            }
        )),
        Some("EXFAT") => out.push(format!(
            "{drive} is exFAT, which has no file permissions: anyone with \
             the drive can read these exports."
        )),
        _ => {}
    }
    if v.remote {
        out.push(if settings.export_db {
            format!(
                "{drive} is a network drive. SQLite's locking is not reliable \
                 over a network share, and a sync interrupted there can damage \
                 the database — export to a local folder and copy it."
            )
        } else {
            format!("{drive} is a network drive; a folder export there is slower.")
        })
    }
    out
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

#[cfg(windows)]
mod imp {
    use super::Volume;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW,
    };

    /// `DRIVE_REMOTE`, from `GetDriveTypeW`. Declared here because it sits
    /// in a different feature of the bindings than the call that returns it.
    const DRIVE_REMOTE: u32 = 4;

    fn wide(p: &std::path::Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn string(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    pub fn inspect(at: &std::path::Path) -> Volume {
        let path = wide(at);
        let mut v = Volume::default();

        let mut free: u64 = 0;
        // SAFETY: a NUL-terminated path and one out-pointer; the other two
        // out-pointers are optional and null.
        if unsafe {
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } != 0
        {
            v.free = Some(free);
        }

        let mut root = [0u16; 261];
        // SAFETY: the buffer's length is what is passed.
        if unsafe { GetVolumePathNameW(path.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0
        {
            return v;
        }
        v.root = Some(string(&root));
        // SAFETY: `root` is NUL-terminated by the call above.
        v.remote = unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_REMOTE;

        let mut fs = [0u16; 64];
        // SAFETY: every out-pointer is null except the file system name,
        // whose length is passed.
        if unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        } != 0
        {
            v.file_system = Some(string(&fs));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(free_gb: u64, fs: &str, remote: bool) -> Volume {
        Volume {
            root: Some(r"E:\".into()),
            free: Some(free_gb * 1024 * 1024 * 1024),
            file_system: Some(fs.into()),
            remote,
        }
    }

    #[test]
    fn a_roomy_local_ntfs_drive_says_nothing() {
        assert!(warnings(&volume(500, "NTFS", false), &Settings::default()).is_empty());
    }

    #[test]
    fn low_space_matters_only_when_media_is_downloaded() {
        let mut s = Settings::default();
        assert_eq!(warnings(&volume(1, "NTFS", false), &s).len(), 1);
        s.download_media = false;
        assert!(warnings(&volume(1, "NTFS", false), &s).is_empty());
    }

    #[test]
    fn fat32_and_exfat_are_each_named() {
        let s = Settings::default();
        assert!(warnings(&volume(500, "FAT32", false), &s)[0].contains("4 GB"));
        assert!(warnings(&volume(500, "exFAT", false), &s)[0].contains("permissions"));
    }

    #[test]
    fn a_database_on_a_network_drive_is_warned_about_harder() {
        let mut s = Settings::default();
        let folder = warnings(&volume(500, "NTFS", true), &s);
        s.export_db = true;
        let db = warnings(&volume(500, "NTFS", true), &s);
        assert!(!folder[0].contains("damage"));
        assert!(db[0].contains("damage"));
    }

    #[test]
    fn a_drive_that_could_not_be_asked_says_nothing() {
        assert!(warnings(&Volume::default(), &Settings::default()).is_empty());
    }

    #[test]
    fn a_destination_not_made_yet_is_asked_about_its_nearest_parent() {
        let dir = std::env::temp_dir().join("tt-volume-not-there").join("deeper");
        let v = inspect(&dir);
        if cfg!(windows) {
            assert!(v.root.is_some() && v.free.is_some(), "{v:?}");
        }
    }
}
