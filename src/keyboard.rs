//! Caps Lock dan bahasa keyboard. Hook keyboard berjalan di thread sendiri dan hanya
//! bereaksi pada tombol Caps Lock serta tombol pengganti bahasa (Shift, Alt, Ctrl,
//! Win, Spasi) saat dilepas. Tombol lain langsung diteruskan dan tidak pernah dicatat.

use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use windows::Win32::Foundation::*;
use windows::Win32::Globalization::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// wParam = 1 bila Caps Lock nyala.
pub const WM_APP_CAPS: u32 = WM_APP + 6;
/// lParam = HKL (layout keyboard) yang baru.
pub const WM_APP_LANG: u32 = WM_APP + 7;
/// Tunggu sebentar setelah tombol dilepas supaya Windows selesai mengganti bahasa.
const LANG_DELAY_MS: u32 = 150;

static TARGET: AtomicIsize = AtomicIsize::new(0);
static LAST_LAYOUT: AtomicIsize = AtomicIsize::new(0);
/// Id timer pengecekan bahasa yang sedang berjalan (0 = tidak ada). Dipakai ulang supaya
/// mengetik spasi beruntun hanya menunda satu timer, bukan membuat timer baru tiap tombol.
static LANG_TIMER: AtomicUsize = AtomicUsize::new(0);

fn post(msg: u32, wp: usize, lp: isize) {
    unsafe {
        let _ = PostMessageW(HWND(TARGET.load(Ordering::Relaxed) as _), msg, WPARAM(wp), LPARAM(lp));
    }
}

fn foreground_layout() -> isize {
    unsafe {
        let tid = GetWindowThreadProcessId(GetForegroundWindow(), None);
        GetKeyboardLayout(tid).0 as isize
    }
}

unsafe extern "system" fn check_language(_: HWND, _: u32, id: usize, _: u32) {
    let _ = KillTimer(None, id);
    LANG_TIMER.store(0, Ordering::Relaxed);
    let hkl = foreground_layout();
    if hkl != 0 && LAST_LAYOUT.swap(hkl, Ordering::Relaxed) != hkl {
        post(WM_APP_LANG, 0, hkl);
    }
}

unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let up = matches!(wp.0 as u32, WM_KEYUP | WM_SYSKEYUP);
    if code == HC_ACTION as i32 && up {
        let vk = VIRTUAL_KEY((*(lp.0 as *const KBDLLHOOKSTRUCT)).vkCode as u16);
        if vk == VK_CAPITAL {
            // Di dalam hook, status toggle sudah diperbarui saat tombol dilepas.
            post(WM_APP_CAPS, (GetKeyState(VK_CAPITAL.0 as i32) & 1) as usize, 0);
        } else if matches!(
            vk,
            VK_SPACE | VK_LSHIFT | VK_RSHIFT | VK_LMENU | VK_RMENU | VK_LCONTROL | VK_RCONTROL | VK_LWIN | VK_RWIN
        ) {
            // Id yang sama = timer lama diulang dari awal.
            let id = SetTimer(None, LANG_TIMER.load(Ordering::Relaxed), LANG_DELAY_MS, Some(check_language));
            LANG_TIMER.store(id, Ordering::Relaxed);
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

/// Pasang hook di thread terpisah, supaya mengetik tidak pernah tertahan oleh UI notch.
pub fn spawn(hwnd: HWND) {
    TARGET.store(hwnd.0 as isize, Ordering::Relaxed);
    LAST_LAYOUT.store(foreground_layout(), Ordering::Relaxed);
    std::thread::spawn(|| unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let Ok(_hook) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), module, 0) else { return };
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    });
}

pub fn current_layout() -> isize {
    foreground_layout()
}

/// Nama bahasa dan kodenya dari layout keyboard, mis. ("Indonesia", "ID").
pub fn language(hkl: isize) -> (String, String) {
    let lcid = (hkl as usize & 0xffff) as u32;
    let mut name = [0u16; 85];
    let n = unsafe { LCIDToLocaleName(lcid, Some(&mut name), 0) } as usize;
    if n == 0 {
        return ("Bahasa keyboard".into(), String::new());
    }
    let locale = String::from_utf16_lossy(&name[..n - 1]);
    let code = locale.split('-').next().unwrap_or("").to_uppercase();
    let mut buf = [0u16; 85];
    let m = unsafe { GetLocaleInfoEx(windows::core::PCWSTR(name.as_ptr()), LOCALE_SNATIVELANGUAGENAME, Some(&mut buf)) }
        as usize;
    let mut native = if m > 1 { String::from_utf16_lossy(&buf[..m - 1]) } else { locale.clone() };
    // Huruf kapital di awal ("english" → "English").
    if let Some(c) = native.chars().next() {
        native = c.to_uppercase().collect::<String>() + &native[c.len_utf8()..];
    }
    (native, code)
}

#[cfg(test)]
mod tests {
    use super::language;

    #[test]
    fn language_names() {
        assert_eq!(language(0x0421), ("Indonesia".to_string(), "ID".to_string()));
        assert_eq!(language(0x0409), ("English".to_string(), "EN".to_string()));
    }
}
