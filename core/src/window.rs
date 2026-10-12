//! Window facts shared by the GUI and its tests: titles, the single-window
//! mutex and where the note popup goes. No GUI code here.

/// Exact title of the main window (found with `FindWindowW` by a second start).
pub const MAIN_TITLE: &str = "File Organizer";
/// Title of the note popup; must differ from [`MAIN_TITLE`] so a second
/// start never brings the popup to the front instead of the window.
pub const NOTE_TITLE: &str = "File Organizer – note";
/// Named mutex held by the main window while it runs.
pub const MAIN_MUTEX: &str = "Local\\FileOrganizer.MainWindow";
/// Gap between the popup and the edge of the work area, in pixels.
pub const NOTE_MARGIN_PX: i32 = 16;

/// A rectangle in physical pixels (like a Win32 `RECT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// What a start of the main window should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAction {
    /// No window yet: open one.
    Open,
    /// A window is already open: bring it to the front and exit.
    BringToFront,
    /// Another start holds the mutex but its window is not there (yet, or
    /// any more): exit without opening a second one.
    Exit,
}

pub fn start_action(mutex_already_existed: bool, window_found: bool) -> StartAction {
    match (mutex_already_existed, window_found) {
        (false, _) => StartAction::Open,
        (true, true) => StartAction::BringToFront,
        (true, false) => StartAction::Exit,
    }
}

/// Top-left corner (in points, as egui wants it) for a popup of
/// `size_points` at the bottom right of `work` (the primary monitor's work
/// area in pixels) with a [`NOTE_MARGIN_PX`] margin, at DPI scale `scale`
/// (1.0 = 100 %). The popup always lies fully inside the work area.
pub fn note_position(work: PxRect, scale: f32, size_points: [f32; 2]) -> (f32, f32) {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let w = (size_points[0] * scale).ceil() as i32;
    let h = (size_points[1] * scale).ceil() as i32;
    let x = (work.right - NOTE_MARGIN_PX - w).max(work.left);
    let y = (work.bottom - NOTE_MARGIN_PX - h).max(work.top);
    // Round towards the top-left so the popup never sticks out at the
    // bottom right, but never past the top-left edge either.
    let to_points = |px: i32, min: i32| {
        let p = (px as f32 / scale).floor();
        if p * scale < min as f32 {
            (min as f32 / scale).ceil()
        } else {
            p
        }
    };
    (to_points(x, work.left), to_points(y, work.top))
}

/// The main window's named mutex, held until dropped.
pub struct MainMutex {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    /// Another start already held it.
    pub already_existed: bool,
}

#[cfg(windows)]
impl Drop for MainMutex {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateMutexW and is closed once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
    }
}

/// Creates (or opens) [`MAIN_MUTEX`]. `None` if Windows refused.
#[cfg(windows)]
pub fn acquire_main_mutex() -> Option<MainMutex> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name: Vec<u16> = MAIN_MUTEX.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: valid NUL-terminated name; the handle is closed by Drop.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: no arguments; read right after CreateMutexW.
    let already_existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    Some(MainMutex { handle, already_existed })
}

#[cfg(not(windows))]
pub fn acquire_main_mutex() -> Option<MainMutex> {
    Some(MainMutex { already_existed: false })
}

/// Tab name with the number of results after a scan: "Duplicates (12)".
pub fn tab_title(name: &str, count: Option<usize>) -> String {
    match count {
        Some(n) => format!("{name} ({n})"),
        None => name.to_string(),
    }
}

/// Smallest window size (points); the layout below must fit it.
pub const MIN_WINDOW: [f32; 2] = [760.0, 420.0];

/// How the main window is split for a given width (points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// Width of the History panel on the right.
    pub history_width: f32,
    /// Width left for the tab content.
    pub content_width: f32,
}

/// Content needs at least this much room: the widest fixed table columns
/// (check box, size, date, buttons) plus a readable file name column.
pub const MIN_CONTENT_WIDTH: f32 = 500.0;

pub fn layout(window_width: f32) -> Layout {
    let w = window_width.max(MIN_WINDOW[0]);
    let history_width = (w * 0.24).clamp(170.0, 300.0).min(w - MIN_CONTENT_WIDTH);
    Layout { history_width, content_width: w - history_width }
}

/// Font files to use, if Windows has them: Segoe UI as the main font and
/// Segoe UI Emoji as a fallback. `None` keeps egui's built-in fonts.
pub fn font_files(fonts_dir: &std::path::Path) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    let pick = |name: &str| Some(fonts_dir.join(name)).filter(|p| p.is_file());
    (pick("segoeui.ttf"), pick("seguiemj.ttf"))
}
