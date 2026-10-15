//! Who holds the model server's port, and stopping it when asked.
//!
//! The Summaries view uses a llama-server it did not start as it is —
//! another program's — and never stops one on its own
//! (`tga_sum::server::Keeper`). But such a server holds 15 GB of the 16 GB
//! card, and while it does, the OCR cannot load to read photos; with another
//! model in it, a summary cannot run on that port at all. So the view names
//! it, and offers **Stop**, which the account holder presses twice. This
//! module finds the process listening on the port and ends it — only a
//! `llama-server.exe`, and only on that press.

use std::path::PathBuf;

use tga_run::tga_sum::Client;

/// The process listening on the port, and what it is serving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    /// Its executable; empty if Windows would not say.
    pub exe: PathBuf,
    /// The model file it has loaded; empty while it is still loading, or if
    /// it is not a model server.
    pub model: String,
    /// It answers `/health`: loaded and ready.
    pub ready: bool,
}

impl Holder {
    pub fn is_llama(&self) -> bool {
        self.exe
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("llama-server.exe"))
    }

    /// `Another program's llama-server`, or the executable's name. Its path
    /// is in the Stop button's hover text.
    pub fn who(&self) -> String {
        if self.is_llama() {
            return "Another program's llama-server".into();
        }
        self.exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("Process {}", self.pid))
    }
}

/// Who listens on `127.0.0.1:port`, or on every address at that port, and
/// what model it serves. `None` when nothing does.
pub fn probe(port: u16) -> Option<Holder> {
    let pid = imp::listener(port)?;
    let exe = imp::image(pid).unwrap_or_default();
    let client = Client::new(port);
    let ready = client.healthy();
    let model = if ready {
        client.props().map(|p| p.model).unwrap_or_default()
    } else {
        String::new()
    };
    Some(Holder {
        pid,
        exe,
        model,
        ready,
    })
}

/// End `holder`'s process and wait for it to go, so its video memory is free
/// when this returns. Refuses anything but a llama-server, and a process that
/// is no longer the one probed.
pub fn stop(holder: &Holder) -> Result<(), String> {
    if !holder.is_llama() {
        return Err(format!("{} is not a llama-server", holder.who()));
    }
    // The PID may have been reused since the probe.
    if imp::image(holder.pid).as_ref() != Some(&holder.exe) {
        return Err("it has already stopped".into());
    }
    imp::terminate(holder.pid)
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE,
    };

    /// `AF_INET`. llama-server's `--host 127.0.0.1` listens on IPv4 only.
    const AF_INET: u32 = 2;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    /// How long a stopped server may take to let go of the card.
    const EXIT_WAIT_MS: u32 = 10_000;

    /// The process listening on loopback or on every address at `port`.
    pub fn listener(port: u16) -> Option<u32> {
        let mut size: u32 = 0;
        let mut buf: Vec<u32> = Vec::new();
        // The table can grow between the call that sizes it and the one that
        // fills it; a second try covers that.
        for _ in 0..3 {
            buf.resize((size as usize).div_ceil(4).max(1), 0);
            // SAFETY: `buf` holds `size` bytes, u32-aligned as the rows are.
            let r = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr().cast(),
                    &mut size,
                    0,
                    AF_INET,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            match r {
                0 => {
                    let table = buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
                    // SAFETY: the call succeeded, so the buffer holds the
                    // count followed by that many rows.
                    let rows: &[MIB_TCPROW_OWNER_PID] = unsafe {
                        std::slice::from_raw_parts(
                            (*table).table.as_ptr(),
                            (*table).dwNumEntries as usize,
                        )
                    };
                    let loopback = u32::from_ne_bytes([127, 0, 0, 1]);
                    return rows
                        .iter()
                        .find(|r| {
                            // The port is in network order, in the low 16 bits.
                            u16::from_be(r.dwLocalPort as u16) == port
                                && (r.dwLocalAddr == loopback || r.dwLocalAddr == 0)
                        })
                        .map(|r| r.dwOwningPid);
                }
                ERROR_INSUFFICIENT_BUFFER => continue,
                _ => return None,
            }
        }
        None
    }

    /// The executable a process was started from.
    pub fn image(pid: u32) -> Option<PathBuf> {
        // SAFETY: a plain open; the handle is closed below.
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
        (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&name[..len as usize])))
    }

    pub fn terminate(pid: u32) -> Result<(), String> {
        // SAFETY: a plain open; the handle is closed below.
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return Err(format!(
                "Windows would not let this app stop it: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: a handle opened with PROCESS_TERMINATE.
        let ended = unsafe { TerminateProcess(handle, 1) } != 0;
        let error = std::io::Error::last_os_error();
        if ended {
            // SAFETY: a handle opened with PROCESS_SYNCHRONIZE.
            unsafe { WaitForSingleObject(handle, EXIT_WAIT_MS) };
        }
        // SAFETY: opened above, closed once.
        unsafe { CloseHandle(handle) };
        if ended {
            Ok(())
        } else {
            Err(format!("Windows would not stop it: {error}"))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    pub fn listener(_port: u16) -> Option<u32> {
        None
    }

    pub fn image(_pid: u32) -> Option<PathBuf> {
        None
    }

    pub fn terminate(_pid: u32) -> Result<(), String> {
        Err("stopping another program's server needs Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holder(exe: &str) -> Holder {
        Holder {
            pid: 4242,
            exe: PathBuf::from(exe),
            model: "gemma-4-26B-A4B-it-qat-UD-Q4_K_XL.gguf".into(),
            ready: true,
        }
    }

    #[test]
    fn a_llama_server_is_named_as_one_and_anything_else_by_its_exe() {
        let h = holder(r"C:\tools\llama\LLAMA-SERVER.EXE");
        assert!(h.is_llama());
        assert_eq!(h.who(), "Another program's llama-server");
        assert_eq!(
            holder(r"C:\Program Files\Thing\thing.exe").who(),
            "thing.exe"
        );
    }

    #[test]
    fn nothing_but_a_llama_server_is_stopped() {
        let err = stop(&holder(r"C:\Windows\explorer.exe")).unwrap_err();
        assert!(err.contains("not a llama-server"), "{err}");
    }

    #[test]
    fn a_listener_on_this_process_is_found_as_this_process() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        if cfg!(windows) {
            assert_eq!(imp::listener(port), Some(std::process::id()));
            let me = imp::image(std::process::id()).unwrap();
            assert_eq!(
                me.file_name().map(|n| n.to_ascii_lowercase()),
                std::env::current_exe()
                    .unwrap()
                    .file_name()
                    .map(|n| n.to_ascii_lowercase())
            );
        }
        drop(listener);
        assert_eq!(imp::listener(port), None);
    }
}
