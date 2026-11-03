//! Close to the tray: the X puts the window in the notification area instead
//! of quitting, so an export and the scheduled sync carry on with nobody
//! looking. Settings › Appearance › **Close to the tray**, off by default.
//!
//! **Minimised, not hidden.** eframe runs `update` only on a `WM_PAINT`, and
//! Windows sends none to a hidden window — so a hidden window would stop
//! draining the bridge and never reach the scheduled sync, which is the whole
//! reason to keep it running. A minimised window is still painted on request
//! (measured on Windows 11, 2026-10-29: `RDW_INTERNALPAINT`, which is how
//! winit asks, reached a minimised window every time and a hidden one never).
//! So "in the tray" is the window minimised with its taskbar button taken
//! away — `ITaskbarList::DeleteTab`, the call winit's own skip-taskbar
//! makes — and an icon in the notification area to bring it back.
//! `ViewportCommand::Visible(false)` is the wrong fix for this; do not use it.
//!
//! The icon's messages go to a hidden window of this module's own, on the UI
//! thread: winit's window procedure is not ours to add a message to.

#[cfg(windows)]
pub use imp::Tray;

/// Nothing to put an icon in: the setting does nothing, and the X closes.
#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
impl Tray {
    pub fn new(_frame: &eframe::Frame, _ctx: &eframe::egui::Context) -> Option<Self> {
        None
    }
    pub fn hide(&mut self) {}
    pub fn follow(&mut self) {}
    pub fn quit_asked(&mut self) -> bool {
        false
    }
    pub fn tip(&mut self, _text: &str) {}
    pub fn balloon(&self, _title: &str, _body: &str) {}
}

#[cfg(windows)]
mod imp {
    use crate::notify::imp::put;
    use eframe::egui;
    use std::cell::RefCell;
    use std::ptr::null_mut;
    use windows_sys::core::GUID;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, TaskbarList, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP,
        NIIF_INFO, NIIF_LARGE_ICON, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT,
        NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        GetSystemMetrics, IsIconic, LoadIconW, LoadImageW, PostMessageW, RegisterClassW,
        RegisterWindowMessageW, SetForegroundWindow, SetMenuDefaultItem, ShowWindow,
        TrackPopupMenu, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, MF_SEPARATOR, MF_STRING,
        SM_CXSMICON, SM_CYSMICON, SW_MINIMIZE, SW_RESTORE, TPM_NONOTIFY, TPM_RETURNCMD,
        TPM_RIGHTBUTTON, WM_APP, WM_CONTEXTMENU, WM_NULL, WNDCLASSW,
    };

    /// Which notification-area icon of our window it is. One is all there is.
    const ID: u32 = 0x7455;
    /// What the icon sends our window when it is clicked.
    const CALLBACK: u32 = WM_APP + 1;
    /// Take the taskbar button away again, after a restarted Explorer's
    /// `TaskbarCreated` — winit answers that broadcast by putting it back.
    const HIDE_AGAIN: u32 = WM_APP + 2;
    /// `NIN_SELECT | NINF_KEY`: the icon chosen with Enter or Space.
    const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
    /// The menu's two entries.
    const OPEN: usize = 1;
    const QUIT: usize = 2;

    /// What the window procedure and the shell both need. **On the UI
    /// thread only**, which is the only thread either runs on.
    struct State {
        /// Ours: the hidden window the icon talks to.
        window: HWND,
        /// eframe's.
        main: HWND,
        ctx: egui::Context,
        /// Minimised with the taskbar button gone.
        hidden: bool,
        /// Quit was chosen from the menu; `update` has not seen it yet.
        quit: bool,
        tip: String,
        /// `RegisterWindowMessageW("TaskbarCreated")`: Explorer restarted,
        /// and every notification-area icon has to be added again.
        taskbar_created: u32,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    /// Read the state, briefly. Never held across a Win32 call: most of them
    /// can send a message straight back into [`proc`].
    fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
        STATE.with(|s| s.borrow_mut().as_mut().map(f))
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The icon, while the setting is on. Dropping it takes the icon away and
    /// gives a hidden window its taskbar button back.
    pub struct Tray {
        /// The how-to balloon has been shown this session.
        hinted: bool,
    }

    impl Tray {
        pub fn new(frame: &eframe::Frame, ctx: &egui::Context) -> Option<Self> {
            let main = crate::notify::hwnd(frame)? as HWND;
            let class = wide("TelegramToolsTray");
            let title = wide("TelegramTools tray");
            // SAFETY: a zeroed WNDCLASSW is valid — integers, handles and
            // nulls — and the strings outlive the calls that read them. The
            // class may already exist from a tray turned off and on again;
            // registering it twice fails harmlessly and creation still works.
            let window = unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let mut wc: WNDCLASSW = std::mem::zeroed();
                wc.lpfnWndProc = Some(proc);
                wc.hInstance = instance;
                wc.lpszClassName = class.as_ptr();
                RegisterClassW(&wc);
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    // Not the app's title: `instance::raise` finds the window
                    // by that, and must not find this one.
                    title.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    null_mut(),
                    null_mut(),
                    instance,
                    std::ptr::null(),
                )
            };
            if window.is_null() {
                return None;
            }
            let created = wide("TaskbarCreated");
            // SAFETY: a NUL-terminated name.
            let taskbar_created = unsafe { RegisterWindowMessageW(created.as_ptr()) };
            STATE.with(|s| {
                *s.borrow_mut() = Some(State {
                    window,
                    main,
                    ctx: ctx.clone(),
                    hidden: false,
                    quit: false,
                    tip: crate::TITLE.into(),
                    taskbar_created,
                })
            });
            if !add() {
                STATE.with(|s| s.borrow_mut().take());
                // SAFETY: created above, on this thread.
                unsafe { DestroyWindow(window) };
                return None;
            }
            Some(Self { hinted: false })
        }

        /// Into the tray: minimised, no taskbar button. The first time in a
        /// session says where it went and how to get it back.
        pub fn hide(&mut self) {
            let Some(main) = with(|s| s.main) else {
                return;
            };
            // SAFETY: eframe's window, alive while the shell is.
            unsafe { ShowWindow(main, SW_MINIMIZE) };
            tab(main, false);
            with(|s| s.hidden = true);
            if !std::mem::replace(&mut self.hinted, true) {
                self.balloon(
                    "Still running",
                    "Exports and the scheduled sync carry on. Click the icon to open; \
                     right-click it to quit.",
                );
            }
        }

        /// Once a frame: a window brought back some other way — a second
        /// launch raising it, Alt+Tab — gets its taskbar button back.
        pub fn follow(&mut self) {
            let Some((main, hidden)) = with(|s| (s.main, s.hidden)) else {
                return;
            };
            // SAFETY: a plain query on eframe's window.
            if hidden && unsafe { IsIconic(main) } == 0 {
                with(|s| s.hidden = false);
                tab(main, true);
            }
        }

        /// Was Quit chosen from the icon's menu? Asking clears it.
        pub fn quit_asked(&mut self) -> bool {
            with(|s| std::mem::take(&mut s.quit)).unwrap_or(false)
        }

        /// What hovering the icon says. Sent only when it changes.
        pub fn tip(&mut self, text: &str) {
            let changed = with(|s| {
                let changed = s.tip != text;
                if changed {
                    s.tip = text.to_string();
                }
                changed
            });
            if changed != Some(true) {
                return;
            }
            let Some(mut d) = data() else {
                return;
            };
            d.uFlags = NIF_TIP | NIF_SHOWTIP;
            put(&mut d.szTip, text);
            // SAFETY: a fully initialised NOTIFYICONDATAW of the size it
            // declares.
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
        }

        /// A notification from the icon already in the tray, rather than
        /// [`crate::notify`]'s second icon for the length of one.
        pub fn balloon(&self, title: &str, body: &str) {
            let Some(mut d) = data() else {
                return;
            };
            d.uFlags = NIF_INFO;
            put(&mut d.szInfoTitle, title);
            put(&mut d.szInfo, body);
            d.dwInfoFlags = NIIF_INFO | NIIF_LARGE_ICON;
            // SAFETY: as in `tip`.
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) } == 0 {
                log::warn!("could not show a notification from the tray icon");
            }
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            let Some(state) = STATE.with(|s| s.borrow_mut().take()) else {
                return;
            };
            let d = blank(state.window);
            // SAFETY: deleting needs only the window and the id; the window
            // is ours, made on this thread.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &d);
                DestroyWindow(state.window);
            }
            // The setting turned off with the window in the tray must not
            // leave it with no way back.
            if state.hidden {
                tab(state.main, true);
            }
        }
    }

    /// A NOTIFYICONDATAW naming our icon and nothing else.
    fn blank(window: HWND) -> NOTIFYICONDATAW {
        // SAFETY: an all-zero NOTIFYICONDATAW is valid — every field is an
        // integer, a handle or an array of them.
        let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        d.hWnd = window;
        d.uID = ID;
        d
    }

    fn data() -> Option<NOTIFYICONDATAW> {
        with(|s| blank(s.window))
    }

    /// Put the icon in the notification area: on creation, and again after
    /// Explorer restarts.
    fn add() -> bool {
        let Some((window, tip)) = with(|s| (s.window, s.tip.clone())) else {
            return false;
        };
        let mut d = blank(window);
        d.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_SHOWTIP;
        d.uCallbackMessage = CALLBACK;
        d.hIcon = icon();
        put(&mut d.szTip, &tip);
        // SAFETY: a fully initialised NOTIFYICONDATAW of the size it declares.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &d) } == 0 {
            log::warn!("could not add the tray icon");
            return false;
        }
        // Version 4: a left click is `NIN_SELECT`, the keyboard has its own,
        // and the menu's place arrives with the message.
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: as above.
        unsafe { Shell_NotifyIconW(NIM_SETVERSION, &d) };
        true
    }

    /// The exe's own icon at the notification area's size, or Windows' plain
    /// one if it is missing.
    fn icon() -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
        // SAFETY: a module handle or null and a resource id; a missing
        // resource returns null, which is checked. `1` is the icon `build.rs`
        // embeds (winresource's default id).
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let own = LoadImageW(
                module,
                1 as _,
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            );
            if own.is_null() {
                LoadIconW(null_mut(), IDI_APPLICATION)
            } else {
                own
            }
        }
    }

    /// Out of the tray: the taskbar button back, the window restored and in
    /// front.
    fn open() {
        let Some((main, hidden)) = with(|s| (s.main, std::mem::take(&mut s.hidden))) else {
            return;
        };
        if hidden {
            tab(main, true);
        }
        // SAFETY: eframe's window. A tray click hands this process the right
        // to take the foreground.
        unsafe {
            if IsIconic(main) != 0 {
                ShowWindow(main, SW_RESTORE);
            }
            SetForegroundWindow(main);
        }
        with(|s| s.ctx.request_repaint());
    }

    /// The icon's right-click menu: Open, and Quit.
    fn menu(window: HWND, at: POINT) {
        let open_label = wide(&format!("Open {}", crate::TITLE));
        let quit_label = wide("Quit");
        // SAFETY: the menu is made, shown and destroyed here; the labels
        // outlive it. `SetForegroundWindow` first and `WM_NULL` after are what
        // the documentation asks for, or the menu will not close when clicked
        // away from.
        let chosen = unsafe {
            let m = CreatePopupMenu();
            if m.is_null() {
                return;
            }
            AppendMenuW(m, MF_STRING, OPEN, open_label.as_ptr());
            AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(m, MF_STRING, QUIT, quit_label.as_ptr());
            SetMenuDefaultItem(m, OPEN as u32, 0);
            SetForegroundWindow(window);
            let chosen = TrackPopupMenu(
                m,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                at.x,
                at.y,
                0,
                window,
                std::ptr::null(),
            );
            PostMessageW(window, WM_NULL, 0, 0);
            DestroyMenu(m);
            chosen
        };
        match chosen as usize {
            OPEN => open(),
            QUIT => {
                // Through the window's own close, so `on_exit` cancels and
                // drains exactly as the X did before this setting existed.
                with(|s| {
                    s.quit = true;
                    s.ctx.request_repaint();
                });
            }
            _ => {}
        }
    }

    unsafe extern "system" fn proc(window: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == CALLBACK {
            match (lp as u32) & 0xFFFF {
                NIN_SELECT | NIN_KEYSELECT => open(),
                WM_CONTEXTMENU => {
                    // Version 4 puts the anchor in `wParam`, signed: a second
                    // monitor left of the first has negative x.
                    let at = POINT {
                        x: i32::from(wp as u16 as i16),
                        y: i32::from((wp >> 16) as u16 as i16),
                    };
                    menu(window, at);
                }
                _ => {}
            }
            return 0;
        }
        if msg == HIDE_AGAIN {
            if let Some((main, true)) = with(|s| (s.main, s.hidden)) {
                tab(main, false);
            }
            return 0;
        }
        if with(|s| s.taskbar_created != 0 && msg == s.taskbar_created) == Some(true) {
            add();
            // After winit has answered the same broadcast by adding the
            // button back.
            // SAFETY: our own window.
            unsafe { PostMessageW(window, HIDE_AGAIN, 0, 0) };
            return 0;
        }
        // SAFETY: the arguments this procedure was given.
        unsafe { DefWindowProcW(window, msg, wp, lp) }
    }

    /// `ITaskbarList`, as far as this needs it. windows-sys has the class id
    /// but no COM interfaces, so the table is spelled out, as winit does.
    #[repr(C)]
    struct TaskbarListObj {
        vtbl: *const TaskbarListVtbl,
    }

    type Call = unsafe extern "system" fn(*mut TaskbarListObj) -> i32;
    type TabCall = unsafe extern "system" fn(*mut TaskbarListObj, HWND) -> i32;

    #[repr(C)]
    struct TaskbarListVtbl {
        _query_interface: usize,
        _add_ref: usize,
        release: unsafe extern "system" fn(*mut TaskbarListObj) -> u32,
        hr_init: Call,
        add_tab: TabCall,
        delete_tab: TabCall,
    }

    const IID_ITASKBARLIST: GUID = GUID::from_u128(0x56fdf342_fd6d_11d0_958a_006097c9a090);

    /// Give `main` its taskbar button (`show`) or take it away.
    fn tab(main: HWND, show: bool) {
        // SAFETY: COM is initialised for the length of the call and balanced
        // after it (winit and OLE have usually done it first, so this only
        // counts up and down). The object is released once; every pointer is
        // checked before use.
        unsafe {
            let init = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as _);
            let mut list: *mut TaskbarListObj = null_mut();
            let made = CoCreateInstance(
                &TaskbarList,
                null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_ITASKBARLIST,
                &mut list as *mut _ as *mut _,
            );
            if made >= 0 && !list.is_null() {
                let vtbl = &*(*list).vtbl;
                if (vtbl.hr_init)(list) >= 0 {
                    let done = if show {
                        (vtbl.add_tab)(list, main)
                    } else {
                        (vtbl.delete_tab)(list, main)
                    };
                    if done < 0 {
                        log::warn!("the taskbar refused to change the window's button: {done:#x}");
                    }
                }
                (vtbl.release)(list);
            } else {
                log::warn!("no taskbar to ask: {made:#x}");
            }
            if init >= 0 {
                CoUninitialize();
            }
        }
    }
}
