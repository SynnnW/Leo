//! Mendeteksi jendela lain yang berada di posisi notch (mis. browser yang dimaksimalkan,
//! tab di atas, atau aplikasi layar penuh). Berbasis event dari Windows: kita hanya
//! menghitung ulang saat ada jendela berpindah, berubah ukuran, muncul, atau hilang.

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Timer di jendela notch untuk menghitung ulang (event datang beruntun, jadi ditunda sedikit).
pub const TIMER_RECHECK: usize = 5;
const DEBOUNCE_MS: u32 = 120;

static NOTCH: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    // Hanya event jendela utuh (bukan kursor, caret, atau elemen di dalam jendela).
    if id_object != OBJID_WINDOW.0 || hwnd.is_invalid() {
        return;
    }
    let notch = HWND(NOTCH.load(Ordering::Relaxed) as _);
    if hwnd == notch {
        return;
    }
    SetTimer(notch, TIMER_RECHECK, DEBOUNCE_MS, None);
}

/// Pasang hook event jendela. Callback berjalan di thread UI lewat loop pesan biasa.
pub fn install(notch: HWND) {
    NOTCH.store(notch.0 as isize, Ordering::Relaxed);
    let ranges = [
        // foreground, pindah/ubah ukuran, minimize/restore
        (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND),
        // destroy, show, hide, reorder, ... locationchange
        (EVENT_OBJECT_DESTROY, EVENT_OBJECT_LOCATIONCHANGE),
        // jendela UWP yang disembunyikan tanpa ditutup
        (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
    ];
    for (min, max) in ranges {
        unsafe {
            SetWinEventHook(
                min,
                max,
                None,
                Some(on_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
        }
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn counts(hwnd: HWND, notch: HWND) -> bool {
    unsafe {
        if hwnd == notch || !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        let ex = WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32);
        // Overlay, tooltip, menu, notch lain: bukan "jendela di belakang".
        if ex.contains(WS_EX_TOOLWINDOW) || ex.contains(WS_EX_TRANSPARENT) || ex.contains(WS_EX_NOACTIVATE) {
            return false;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut c_void, 4);
        if cloaked != 0 {
            return false;
        }
        !matches!(
            class_name(hwnd).as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        )
    }
}

struct Search {
    notch: HWND,
    area: RECT,
    found: bool,
}

unsafe extern "system" fn visit(hwnd: HWND, lp: LPARAM) -> BOOL {
    let s = &mut *(lp.0 as *mut Search);
    if counts(hwnd, s.notch) {
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_ok() {
            let a = s.area;
            if r.left < a.right && r.right > a.left && r.top < a.bottom && r.bottom > a.top {
                s.found = true;
                return FALSE;
            }
        }
    }
    TRUE
}

/// Ada jendela lain yang menempati `area` (koordinat layar, piksel)?
pub fn covered(notch: HWND, area: RECT) -> bool {
    let mut s = Search { notch, area, found: false };
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut s as *mut Search as isize));
    }
    s.found
}

pub fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}
