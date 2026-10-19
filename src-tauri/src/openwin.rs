//! The Open windows widget: the windows Alt+Tab and the taskbar would list, and bringing one to
//! the front or asking it to close.
//!
//! Listing is one `EnumWindows` pass, a fraction of a millisecond, so it runs on the sampler
//! thread instead of one of its own, and only while the dashboard is on screen and the widget is
//! in the layout.

use crate::ffi;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

static WANTED: AtomicBool = AtomicBool::new(false);

/// Whether the widget is on the dashboard. Without it, no window is looked at.
pub fn set_wanted(on: bool) {
    WANTED.store(on, Ordering::Relaxed);
}

const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_APPWINDOW: u32 = 0x0004_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

/// The desktop, taskbars and Start/Search: windows of the shell that aren't "apps".
const SHELL_CLASSES: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Windows.UI.Core.CoreWindow"];

/// One row of the widget.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Win {
    /// The window handle, which the page hands back with `pid` to act on it (see `check`).
    hwnd: u64,
    pid: u32,
    /// The program's file name without ".exe"; the page makes it readable.
    exe: String,
    title: String,
    minimized: bool,
    /// The window in front: the one with the keyboard focus, or while Oriel itself has it, the
    /// one that had it before.
    front: bool,
}

/// What Windows says about one top-level window.
#[derive(Clone, Debug, Default)]
struct Raw {
    hwnd: isize,
    pid: u32,
    owned: bool,
    ex_style: u32,
    cloaked: bool,
    minimized: bool,
    class: String,
    title: String,
}

/// The same test Alt+Tab applies, as far as it's documented: a window with a title that isn't a
/// tool window, a helper of another window, hidden by the system (cloaked: a suspended Store app,
/// another virtual desktop) or part of the shell. Oriel's own windows are left out.
fn listed(w: &Raw, own_pid: u32) -> bool {
    let app = w.ex_style & WS_EX_APPWINDOW != 0;
    !w.title.is_empty()
        && w.pid != own_pid
        && !w.cloaked
        && (app || (w.ex_style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0 && !w.owned))
        && !SHELL_CLASSES.contains(&w.class.as_str())
}

/// The rows for `all` (top-most first, as `EnumWindows` gives them). `foreground` is the window
/// with the keyboard focus and its process.
fn build(all: &[Raw], own_pid: u32, foreground: (isize, u32), mut exe: impl FnMut(u32) -> String) -> Vec<Win> {
    let mut rows: Vec<Win> = all
        .iter()
        .filter(|w| listed(w, own_pid))
        .map(|w| Win {
            hwnd: w.hwnd as u64,
            pid: w.pid,
            exe: exe(w.pid),
            title: w.title.clone(),
            minimized: w.minimized,
            front: w.hwnd == foreground.0,
        })
        .collect();
    // Clicking the dashboard makes it the foreground window; what was in front is then the
    // top-most one that isn't minimized.
    if foreground.1 == own_pid && !rows.iter().any(|r| r.front) {
        if let Some(r) = rows.iter_mut().find(|r| !r.minimized) {
            r.front = true;
        }
    }
    rows
}

/// Whether `current` (what the handle is now, None if it's gone) is still the window the page
/// listed. Windows reuses handles, so a handle alone isn't enough: it has to be a listed window
/// of the same process.
fn check(current: Option<&Raw>, pid: u32, own_pid: u32) -> bool {
    current.is_some_and(|w| w.pid == pid && listed(w, own_pid))
}

// --- Windows ---------------------------------------------------------------------

unsafe extern "system" fn collect(hwnd: isize, out: isize) -> i32 {
    // Hidden windows are most of them; skip those before asking anything else.
    if ffi::IsWindowVisible(hwnd) != 0 {
        if let Some(w) = read(hwnd) {
            (*(out as *mut Vec<Raw>)).push(w);
        }
    }
    1
}

/// Ask Windows about one window. None if it's gone.
fn read(hwnd: isize) -> Option<Raw> {
    unsafe {
        if ffi::IsWindow(hwnd) == 0 {
            return None;
        }
        let mut pid = 0u32;
        ffi::GetWindowThreadProcessId(hwnd, &mut pid);
        let mut cloaked = 0u32;
        ffi::DwmGetWindowAttribute(hwnd, ffi::DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), 4);
        let (mut title, mut class) = ([0u16; 512], [0u16; 256]);
        let t = ffi::GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
        let c = ffi::GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        Some(Raw {
            hwnd,
            pid,
            owned: ffi::GetWindow(hwnd, ffi::GW_OWNER) != 0,
            ex_style: ffi::GetWindowLongW(hwnd, ffi::GWL_EXSTYLE) as u32,
            cloaked: cloaked != 0,
            minimized: ffi::IsIconic(hwnd) != 0,
            class: String::from_utf16_lossy(&class[..c]),
            title: String::from_utf16_lossy(&title[..t]),
        })
    }
}

fn enumerate() -> Vec<Raw> {
    let mut all: Vec<Raw> = Vec::new();
    unsafe { ffi::EnumWindows(collect, (&mut all as *mut Vec<Raw>) as isize) };
    all
}

/// Looks windows up for the sampler.
#[derive(Default)]
pub struct Windows {
    /// Program names by process id, so each is looked up once.
    exes: HashMap<u32, String>,
}

impl Windows {
    /// The open windows, or None while the widget isn't on the dashboard.
    pub fn poll(&mut self) -> Option<Vec<Win>> {
        if !WANTED.load(Ordering::Relaxed) {
            self.exes.clear();
            return None;
        }
        let own = std::process::id();
        let fg = unsafe { ffi::GetForegroundWindow() };
        let fg_pid = read(fg).map_or(0, |w| w.pid);
        let exes = &mut self.exes;
        let rows = build(&enumerate(), own, (fg, fg_pid), |pid| exes.entry(pid).or_insert_with(|| crate::audio::exe_name(pid)).clone());
        self.exes.retain(|pid, _| rows.iter().any(|r| r.pid == *pid));
        Some(rows)
    }
}

/// The window the page showed, if it's still there and still the same program's.
fn target(hwnd: u64, pid: u32) -> Result<isize, String> {
    let hwnd = isize::try_from(hwnd).map_err(|_| "not a window".to_string())?;
    if check(read(hwnd).as_ref(), pid, std::process::id()) {
        Ok(hwnd)
    } else {
        Err("that window has gone".into())
    }
}

/// Bring a window to the front, restoring it first if it's minimized. Windows only lets a
/// program do this when it has just had user input, which a click on the dashboard is.
pub fn focus(hwnd: u64, pid: u32) -> Result<(), String> {
    let hwnd = target(hwnd, pid)?;
    unsafe {
        if ffi::IsIconic(hwnd) != 0 {
            ffi::ShowWindow(hwnd, ffi::SW_RESTORE);
        }
        if ffi::SetForegroundWindow(hwnd) == 0 {
            return Err("Windows wouldn't bring that window to the front".into());
        }
    }
    Ok(())
}

/// Ask a window to close, as its × button does: the program can still offer to save. Never
/// ends the process.
pub fn close(hwnd: u64, pid: u32) -> Result<(), String> {
    let hwnd = target(hwnd, pid)?;
    // Refused for windows of elevated programs, like everything we send them.
    if unsafe { ffi::PostMessageW(hwnd, ffi::WM_CLOSE, 0, 0) } == 0 {
        return Err("Windows wouldn't deliver the request (an elevated program?)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: u32 = 100;

    fn win(hwnd: isize, pid: u32, title: &str) -> Raw {
        Raw { hwnd, pid, title: title.into(), class: "Window".into(), ..Default::default() }
    }

    #[test]
    fn ordinary_windows_are_listed_and_the_rest_are_not() {
        assert!(listed(&win(1, 7, "Notes"), ME));
        // No title, Oriel's own, hidden by the system, a Store app that's suspended.
        assert!(!listed(&win(1, 7, ""), ME));
        assert!(!listed(&win(1, ME, "Oriel"), ME));
        assert!(!listed(&Raw { cloaked: true, ..win(1, 7, "Calculator") }, ME));
        // The desktop and the taskbar have titles of their own.
        assert!(!listed(&Raw { class: "Progman".into(), ..win(1, 7, "Program Manager") }, ME));
        assert!(!listed(&Raw { class: "Shell_TrayWnd".into(), ..win(1, 7, "Taskbar") }, ME));
    }

    #[test]
    fn tool_windows_and_dialogs_follow_alt_tab() {
        let tool = Raw { ex_style: WS_EX_TOOLWINDOW, ..win(1, 7, "Palette") };
        assert!(!listed(&tool, ME));
        // Unless the program says it should be in the taskbar.
        assert!(listed(&Raw { ex_style: WS_EX_TOOLWINDOW | WS_EX_APPWINDOW, ..tool }, ME));
        // A window owned by another is part of it (a dialog), unless it asks to be shown.
        let owned = Raw { owned: true, ..win(2, 7, "Save as") };
        assert!(!listed(&owned, ME));
        assert!(listed(&Raw { ex_style: WS_EX_APPWINDOW, ..owned }, ME));
        assert!(!listed(&Raw { ex_style: WS_EX_NOACTIVATE, ..win(3, 7, "Tooltip") }, ME));
    }

    #[test]
    fn rows_keep_z_order_and_mark_the_front() {
        let all = [win(1, 7, "Notes"), win(2, 8, ""), Raw { minimized: true, ..win(3, 9, "Sheets") }, win(4, ME, "Oriel")];
        let rows = build(&all, ME, (3, 9), |pid| format!("app{pid}"));
        let seen: Vec<_> = rows.iter().map(|r| (r.hwnd, r.exe.as_str(), r.minimized, r.front)).collect();
        assert_eq!(seen, [(1, "app7", false, false), (3, "app9", true, true)]);
    }

    #[test]
    fn with_the_dashboard_in_front_the_last_window_is() {
        let all = [Raw { minimized: true, ..win(1, 7, "Mail") }, win(2, 8, "Notes"), win(3, 9, "Sheets"), win(4, ME, "Oriel")];
        let rows = build(&all, ME, (4, ME), |_| String::new());
        let front: Vec<_> = rows.iter().filter(|r| r.front).map(|r| r.hwnd).collect();
        assert_eq!(front, [2]);
        // Another program's window with no title in front (a menu, say): nothing guessed.
        let rows = build(&all, ME, (50, 60), |_| String::new());
        assert!(rows.iter().all(|r| !r.front));
    }

    #[test]
    fn a_reused_handle_is_not_the_window_that_was_shown() {
        let w = win(1, 7, "Notes");
        assert!(check(Some(&w), 7, ME));
        // The handle now belongs to another process's window.
        assert!(!check(Some(&w), 8, ME));
        // Gone, or no longer something we'd list (closed to the tray and hidden, retitled to nothing).
        assert!(!check(None, 7, ME));
        assert!(!check(Some(&Raw { title: String::new(), ..w.clone() }), 7, ME));
        // Never acts on Oriel's own window, whatever the page asks.
        assert!(!check(Some(&win(1, ME, "Oriel")), ME, ME));
    }

    #[test]
    fn refuses_handles_that_are_not_windows() {
        assert!(focus(0, 1).is_err());
        assert!(focus(u64::MAX, 1).is_err());
        assert!(close(0x7fff_fff0, 1).is_err());
    }

    #[test]
    fn lists_this_pcs_windows() {
        // Whatever is open here; a CI runner may have nothing. The point is that the calls work and
        // what comes back is sane.
        set_wanted(true);
        let mut windows = Windows::default();
        let rows = windows.poll().expect("wanted, so listed");
        let start = std::time::Instant::now();
        for _ in 0..100 {
            windows.poll();
        }
        println!("a poll takes {:?}", start.elapsed() / 100);
        set_wanted(false);
        for r in &rows {
            assert!(!r.title.is_empty() && r.pid != std::process::id() && r.hwnd != 0);
        }
        assert!(rows.iter().filter(|r| r.front).count() <= 1);
        println!("{} open window(s)", rows.len());
    }
}
