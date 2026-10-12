//! Stage 4, part 2: single main window and the note popup position.

use organizer_core::window::{
    note_position, start_action, PxRect, StartAction, MAIN_MUTEX, MAIN_TITLE, NOTE_MARGIN_PX,
    NOTE_TITLE,
};

const SIZE: [f32; 2] = [430.0, 120.0];

/// The popup (converted back to pixels) lies fully inside `work`, at the
/// bottom right, with about a 16 px margin.
fn check(work: PxRect, scale: f32) {
    let (x, y) = note_position(work, scale, SIZE);
    let (l, t) = (x * scale, y * scale);
    let (r, b) = (l + SIZE[0] * scale, t + SIZE[1] * scale);
    assert!(
        l >= work.left as f32 && t >= work.top as f32,
        "{scale}: {l},{t}"
    );
    assert!(
        r <= (work.right - NOTE_MARGIN_PX) as f32 + 0.01,
        "{scale}: right {r}"
    );
    assert!(
        b <= (work.bottom - NOTE_MARGIN_PX) as f32 + 0.01,
        "{scale}: bottom {b}"
    );
    assert!(
        r >= (work.right - NOTE_MARGIN_PX) as f32 - 2.0 * scale,
        "{scale}: not at the right edge: {r}"
    );
    assert!(
        b >= (work.bottom - NOTE_MARGIN_PX) as f32 - 2.0 * scale,
        "{scale}: not at the bottom: {b}"
    );
}

#[test]
fn note_sits_bottom_right_inside_work_area_at_all_scales() {
    // 1920x1080 screen with a 48 px taskbar at the bottom (pixels).
    let fhd = PxRect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1032,
    };
    let uhd = PxRect {
        left: 0,
        top: 0,
        right: 3840,
        bottom: 2088,
    };
    let odd = PxRect {
        left: 0,
        top: 0,
        right: 1366,
        bottom: 727,
    };
    for scale in [1.0, 1.25, 1.5, 2.0] {
        check(fhd, scale);
        check(uhd, scale);
        check(odd, scale);
    }
}

#[test]
fn note_exact_positions() {
    let fhd = PxRect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1032,
    };
    assert_eq!(note_position(fhd, 1.0, SIZE), (1474.0, 896.0));
    assert_eq!(note_position(fhd, 2.0, SIZE), (522.0, 388.0));
    // 125 %: 430 pt = 537.5 -> 538 px; 1920-16-538 = 1366 px = 1092.8 pt -> 1092.
    assert_eq!(note_position(fhd, 1.25, SIZE), (1092.0, 692.0));
    assert_eq!(note_position(fhd, 1.5, SIZE), (839.0, 557.0));
}

#[test]
fn note_never_leaves_a_small_or_shifted_work_area() {
    // Taskbar on the left / top, and a work area smaller than the popup.
    let left_bar = PxRect {
        left: 62,
        top: 0,
        right: 1920,
        bottom: 1080,
    };
    let top_bar = PxRect {
        left: 0,
        top: 40,
        right: 1280,
        bottom: 720,
    };
    for scale in [1.0, 1.25, 1.5, 2.0] {
        check(left_bar, scale);
        check(top_bar, scale);
        let tiny = PxRect {
            left: 10,
            top: 10,
            right: 300,
            bottom: 100,
        };
        let (x, y) = note_position(tiny, scale, SIZE);
        assert!(x * scale >= 10.0 && y * scale >= 10.0);
    }
    // A broken scale falls back to 100 %.
    let fhd = PxRect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1032,
    };
    assert_eq!(note_position(fhd, 0.0, SIZE), note_position(fhd, 1.0, SIZE));
}

#[test]
fn second_start_brings_the_window_to_the_front() {
    assert_eq!(start_action(false, false), StartAction::Open);
    assert_eq!(start_action(false, true), StartAction::Open);
    assert_eq!(start_action(true, true), StartAction::BringToFront);
    assert_eq!(start_action(true, false), StartAction::Exit);
    assert_ne!(
        MAIN_TITLE, NOTE_TITLE,
        "the popup must never be found instead of the window"
    );
    assert_eq!(MAIN_TITLE, "File Organizer");
    assert_eq!(MAIN_MUTEX, "Local\\FileOrganizer.MainWindow");
}

#[cfg(windows)]
#[test]
fn main_window_mutex_is_single_instance() {
    // The same calls the GUI makes: the second create sees ERROR_ALREADY_EXISTS.
    let a = organizer_core::window::acquire_main_mutex();
    assert!(a.as_ref().is_some_and(|m| !m.already_existed));
    let b = organizer_core::window::acquire_main_mutex();
    assert!(b.as_ref().is_some_and(|m| m.already_existed));
    drop(a);
    drop(b);
    let c = organizer_core::window::acquire_main_mutex();
    assert!(c.is_some_and(|m| !m.already_existed));
}
