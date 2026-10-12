//! `--tray`: background sorting of Downloads. No window and no egui: just a
//! tray icon, a hidden Win32 window for its messages and a 15 s timer, so
//! the process sleeps in `GetMessageW` almost all the time. All file logic
//! is in `organizer_core::autosort`.

use organizer_core::autosort::{AutoSorter, CHECK_EVERY};
use organizer_core::journal::{Journal, JOURNAL_FILE, TAG_AUTO};
use organizer_core::paths::app_data_dir;
use organizer_core::{autostart, Roots, Settings};
use std::cell::RefCell;
use std::path::PathBuf;
use std::time::SystemTime;
use windows_sys::core::PCWSTR;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, LoadIconW, LoadImageW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetTimer, TrackPopupMenu,
    TranslateMessage, ASFW_ANY, HICON, IDI_APPLICATION, IMAGE_ICON, MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG,
    SM_CXSMICON, SM_CYSMICON, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_NULL,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

const WM_TRAY: u32 = WM_APP + 1;
const TIMER_ID: usize = 1;
const CMD_OPEN: usize = 1;
const CMD_PAUSE: usize = 2;
const CMD_UNDO: usize = 3;
const CMD_AUTOSTART: usize = 4;
const CMD_QUIT: usize = 5;

struct Tray {
    hwnd: HWND,
    icon: HICON,
    roots: Option<Roots>,
    journal: Option<Journal>,
    settings_path: Option<PathBuf>,
    sorter: AutoSorter,
    taskbar_created: u32,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_into(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

fn spawn_self(args: &[&str]) {
    // Lets the started window (or an already open one) come to the front.
    // SAFETY: no pointers involved.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(args).spawn();
    }
}

impl Tray {
    fn settings(&self) -> Settings {
        let text = self.settings_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok());
        text.map(|t| Settings::from_json(&t).0).unwrap_or_default()
    }

    fn icon_data(&self) -> NOTIFYICONDATAW {
        let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        d.hWnd = self.hwnd;
        d.uID = 1;
        d
    }

    fn show_icon(&self, add: bool) {
        let mut d = self.icon_data();
        d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        d.uCallbackMessage = WM_TRAY;
        d.hIcon = self.icon;
        let tip = if self.settings().background_paused {
            "File Organizer – background sorting paused"
        } else {
            "File Organizer – sorting Downloads in the background"
        };
        copy_into(&mut d.szTip, tip);
        // SAFETY: `d` is fully initialised and lives for the call.
        unsafe { Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &d) };
    }

    fn remove_icon(&self) {
        let d = self.icon_data();
        // SAFETY: as above.
        unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
    }

    fn tick(&mut self) {
        let (Some(roots), Some(journal)) = (&self.roots, &self.journal) else {
            return;
        };
        // The window is running a batch: try again at the next tick.
        if !matches!(journal.try_lock(), Ok(Some(_))) {
            return;
        }
        let settings = self.settings();
        let paused = settings.background_paused;
        if let Some(report) = self.sorter.tick(roots, &settings, journal, paused, SystemTime::now()) {
            if !report.moved.is_empty() {
                spawn_self(&["--note", &report.batch.to_string()]);
            }
        }
    }

    fn toggle_pause(&mut self) {
        if let Some(path) = &self.settings_path {
            let _ = Settings::update(path, |s| s.background_paused = !s.background_paused);
        }
        self.show_icon(false);
    }

    fn undo_last(&self) {
        let (Some(journal), Some(roots)) = (&self.journal, &self.roots) else {
            return;
        };
        if let Ok(Some(report)) = journal.undo_last_tagged(TAG_AUTO, roots) {
            spawn_self(&["--note", &report.batch.to_string()]);
        }
    }
}

/// Shows the right-click menu and returns the chosen command (0 = none).
fn menu(hwnd: HWND, paused: bool, autostart_on: bool) -> usize {
    // SAFETY: plain Win32 menu calls on a menu we create and destroy here.
    unsafe {
        let m = CreatePopupMenu();
        let item = |id: usize, text: &str, checked: bool| {
            let t = wide(text);
            AppendMenuW(m, MF_STRING | if checked { MF_CHECKED } else { 0 }, id, t.as_ptr());
        };
        item(CMD_OPEN, "Open Organizer", false);
        item(CMD_PAUSE, "Pause", paused);
        item(CMD_UNDO, "Undo last auto-sort", false);
        item(CMD_AUTOSTART, "Start with Windows", autostart_on);
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        item(CMD_QUIT, "Quit", false);
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd); // so the menu closes when clicking elsewhere
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, std::ptr::null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
        cmd.max(0) as usize
    }
}

fn with_tray<R>(f: impl FnOnce(&mut Tray) -> R) -> Option<R> {
    TRAY.with(|t| t.try_borrow_mut().ok().and_then(|mut t| t.as_mut().map(f)))
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wp == TIMER_ID => {
            with_tray(|t| t.tick());
            0
        }
        WM_TRAY => {
            match lp as u32 {
                WM_LBUTTONDBLCLK => spawn_self(&[]),
                WM_RBUTTONUP => {
                    // No borrow is held while the menu is open (its loop
                    // keeps delivering timer messages).
                    let paused = with_tray(|t| t.settings().background_paused).unwrap_or(false);
                    match menu(hwnd, paused, autostart::is_enabled()) {
                        CMD_OPEN => spawn_self(&[]),
                        CMD_PAUSE => {
                            with_tray(|t| t.toggle_pause());
                        }
                        CMD_UNDO => {
                            with_tray(|t| t.undo_last());
                        }
                        CMD_AUTOSTART => {
                            if let Ok(exe) = std::env::current_exe() {
                                let _ = autostart::set_enabled(!autostart::is_enabled(), &exe);
                            }
                        }
                        CMD_QUIT => {
                            DestroyWindow(hwnd);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            with_tray(|t| t.remove_icon());
            PostQuitMessage(0);
            0
        }
        _ => {
            // Explorer restarted: put the icon back.
            if with_tray(|t| t.taskbar_created != 0 && msg == t.taskbar_created) == Some(true) {
                with_tray(|t| t.show_icon(true));
                return 0;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

/// Runs the tray until Quit. Returns at once if it is already running.
pub fn run() {
    let mutex_name = wide("Local\\FileOrganizer.BackgroundSorting");
    // SAFETY: valid NUL-terminated name; the handle is kept until exit.
    let _mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return; // single instance
    }
    let roots = Roots::from_system().and_then(|r| r.validate().map(|_| r)).ok();
    let dir = app_data_dir().ok();
    let journal = dir.as_ref().map(|d| Journal::new(d.join(JOURNAL_FILE)));
    let settings_path = dir.as_ref().map(|d| d.join("settings.json"));
    if let Some(p) = &settings_path {
        let _ = Settings::load_or_create(p);
    }
    // SAFETY: standard window-class / window creation; all strings outlive the calls.
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide("FileOrganizerTray");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let title = wide("File Organizer background sorting");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        let (cx, cy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        let mut icon = LoadImageW(hinst, 1 as PCWSTR, IMAGE_ICON, cx, cy, 0) as HICON;
        if icon.is_null() {
            icon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
        }
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        let tray = Tray { hwnd, icon, roots, journal, settings_path, sorter: AutoSorter::new(), taskbar_created };
        tray.show_icon(true);
        TRAY.with(|t| *t.borrow_mut() = Some(tray));
        SetTimer(hwnd, TIMER_ID, CHECK_EVERY.as_millis() as u32, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
