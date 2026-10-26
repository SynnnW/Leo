//! Diagnostik: bagaimana membaca status Caps Lock dari hook keyboard low-level.
//! Menekan Caps Lock dua kali lewat SendInput (status akhirnya kembali seperti semula).

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

static LOCAL: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        if k.vkCode == VK_CAPITAL.0 as u32 {
            let down = wp.0 as u32 == WM_KEYDOWN;
            if down {
                LOCAL.fetch_xor(true, Ordering::Relaxed);
            }
            println!(
                "{} GetKeyState={} hitungan_sendiri={}",
                if down { "down" } else { "up  " },
                GetKeyState(VK_CAPITAL.0 as i32) & 1,
                LOCAL.load(Ordering::Relaxed) as u8
            );
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

fn press() {
    let mk = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_CAPITAL, dwFlags: flags, ..Default::default() } },
    };
    unsafe {
        SendInput(&[mk(KEYBD_EVENT_FLAGS(0)), mk(KEYEVENTF_KEYUP)], std::mem::size_of::<INPUT>() as i32);
    }
}

fn main() {
    unsafe {
        let start = GetKeyState(VK_CAPITAL.0 as i32) & 1;
        LOCAL.store(start == 1, Ordering::Relaxed);
        println!("awal GetKeyState={start}");
        let h = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), GetModuleHandleW(None).unwrap(), 0).unwrap();
        std::thread::spawn(|| {
            for _ in 0..2 {
                std::thread::sleep(std::time::Duration::from_millis(400));
                press();
            }
        });
        let t = SetTimer(None, 0, 1500, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER {
                break;
            }
        }
        let _ = KillTimer(None, t);
        let _ = UnhookWindowsHookEx(h);
        println!("akhir GetKeyState={}", GetKeyState(VK_CAPITAL.0 as i32) & 1);
    }
}
