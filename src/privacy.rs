//! Indikator mikrofon/kamera. Windows mencatat setiap aplikasi yang memakai mic atau
//! kamera di registry (ConsentStore): selama dipakai, `LastUsedTimeStop` bernilai 0.
//! Kita menunggu perubahan registry itu (tanpa polling) di thread terpisah.

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects, INFINITE};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Ada perubahan; baca `current()`.
pub const WM_APP_PRIVACY: u32 = WM_APP + 9;

const STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\";

#[derive(Clone, Default, PartialEq, Debug)]
pub struct InUse {
    pub mic: Vec<String>,
    pub cam: Vec<String>,
}

static TARGET: AtomicIsize = AtomicIsize::new(0);
static STATE: Mutex<Option<InUse>> = Mutex::new(None);

pub fn current() -> InUse {
    STATE.lock().unwrap().clone().unwrap_or_default()
}

/// Nama yang enak dibaca dari nama subkey registry.
/// "C:#Program Files#Zoom#bin#Zoom.exe" → "Zoom",
/// "5319275A.WhatsAppDesktop_cv1g1gvanyjgm" → "WhatsAppDesktop".
pub fn app_name(key: &str) -> String {
    if key.contains('#') {
        let file = key.rsplit('#').next().unwrap_or(key);
        return file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file).to_string();
    }
    let family = key.split('_').next().unwrap_or(key);
    family.rsplit('.').next().unwrap_or(family).to_string()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open(path: &str) -> Option<HKEY> {
    let mut k = HKEY::default();
    let p = wide(path);
    unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), 0, KEY_READ | KEY_NOTIFY, &mut k).ok().ok()? };
    Some(k)
}

fn subkeys(k: HKEY) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0.. {
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let r = unsafe { RegEnumKeyExW(k, i, PWSTR(buf.as_mut_ptr()), &mut len, None, PWSTR::null(), None, None) };
        if r.is_err() {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    out
}

fn qword(k: HKEY, sub: &str, name: PCWSTR) -> u64 {
    let mut v = 0u64;
    let mut len = 8u32;
    let s = wide(sub);
    unsafe {
        let _ = RegGetValueW(k, PCWSTR(s.as_ptr()), name, RRF_RT_REG_QWORD, None, Some(&mut v as *mut u64 as *mut c_void), Some(&mut len));
    }
    v
}

/// Aplikasi yang sedang memakai `capability` ("microphone" atau "webcam").
fn scan(capability: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(root) = open(&format!("{STORE}{capability}")) else { return out };
    let check = |k: HKEY, sub: &str, out: &mut Vec<String>| {
        let start = qword(k, sub, w!("LastUsedTimeStart"));
        let stop = qword(k, sub, w!("LastUsedTimeStop"));
        if start != 0 && stop == 0 {
            let n = app_name(sub);
            if !out.contains(&n) {
                out.push(n);
            }
        }
    };
    for sub in subkeys(root) {
        if sub == "NonPackaged" {
            if let Some(np) = open(&format!("{STORE}{capability}\\NonPackaged")) {
                for s in subkeys(np) {
                    check(np, &s, &mut out);
                }
                unsafe {
                    let _ = RegCloseKey(np);
                }
            }
        } else {
            check(root, &sub, &mut out);
        }
    }
    unsafe {
        let _ = RegCloseKey(root);
    }
    out
}

fn refresh() -> bool {
    let now = InUse { mic: scan("microphone"), cam: scan("webcam") };
    let mut st = STATE.lock().unwrap();
    let changed = st.as_ref() != Some(&now);
    *st = Some(now);
    changed
}

pub fn watch(hwnd: HWND) {
    TARGET.store(hwnd.0 as isize, Ordering::Relaxed);
    refresh();
    std::thread::spawn(|| unsafe {
        let keys: Vec<HKEY> = ["microphone", "webcam"].iter().filter_map(|c| open(&format!("{STORE}{c}"))).collect();
        let events: Vec<HANDLE> = keys.iter().filter_map(|_| CreateEventW(None, false, false, None).ok()).collect();
        if keys.is_empty() || events.len() != keys.len() {
            return;
        }
        loop {
            for (k, e) in keys.iter().zip(&events) {
                let _ = RegNotifyChangeKeyValue(*k, true, REG_NOTIFY_CHANGE_LAST_SET, *e, true);
            }
            WaitForMultipleObjects(&events, false, INFINITE);
            if refresh() {
                let _ = PostMessageW(HWND(TARGET.load(Ordering::Relaxed) as _), WM_APP_PRIVACY, WPARAM(0), LPARAM(0));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::app_name;

    #[test]
    fn names() {
        assert_eq!(app_name("C:#Program Files#Zoom#bin#Zoom.exe"), "Zoom");
        assert_eq!(app_name("5319275A.WhatsAppDesktop_cv1g1gvanyjgm"), "WhatsAppDesktop");
        assert_eq!(app_name("MSTeams_8wekyb3d8bbwe"), "MSTeams");
        assert_eq!(app_name("Microsoft.WindowsCamera_8wekyb3d8bbwe"), "WindowsCamera");
    }
}
