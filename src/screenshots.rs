//! Screenshot baru (Win+Shift+S / PrtScn yang disimpan otomatis oleh Windows) masuk Shelf.
//! Kita menunggu notifikasi perubahan folder Screenshots, tanpa polling.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{FOLDERID_Screenshots, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Ada screenshot baru; ambil dengan `take()`.
pub const WM_APP_SCREENSHOT: u32 = WM_APP + 10;
/// File yang lebih tua dari ini bukan screenshot baru (mis. hasil sinkron OneDrive).
const FRESH: Duration = Duration::from_secs(15);

static TARGET: AtomicIsize = AtomicIsize::new(0);
static NEW: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn take() -> Vec<PathBuf> {
    std::mem::take(&mut NEW.lock().unwrap())
}

fn folder() -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Screenshots, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

fn images(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.ends_with(".png") || n.ends_with(".jpg") || n.ends_with(".jpeg")
        })
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .collect()
}

pub fn watch(hwnd: HWND) {
    TARGET.store(hwnd.0 as isize, Ordering::Relaxed);
    let Some(dir) = folder() else { return };
    std::thread::spawn(move || unsafe {
        let mut seen: HashSet<PathBuf> = images(&dir).into_iter().map(|i| i.0).collect();
        let w: Vec<u16> = dir.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let Ok(h) = FindFirstChangeNotificationW(PCWSTR(w.as_ptr()), false, FILE_NOTIFY_CHANGE_FILE_NAME) else {
            return;
        };
        loop {
            if WaitForSingleObject(h, INFINITE) != WAIT_OBJECT_0 {
                break;
            }
            // Beri waktu Snipping Tool selesai menulis file.
            std::thread::sleep(Duration::from_millis(400));
            let now = SystemTime::now();
            let fresh: Vec<PathBuf> = images(&dir)
                .into_iter()
                .filter(|(p, t)| !seen.contains(p) && now.duration_since(*t).map_or(true, |d| d < FRESH))
                .map(|(p, _)| p)
                .collect();
            if !fresh.is_empty() {
                seen.extend(fresh.iter().cloned());
                NEW.lock().unwrap().extend(fresh);
                let _ = PostMessageW(HWND(TARGET.load(Ordering::Relaxed) as _), WM_APP_SCREENSHOT, WPARAM(0), LPARAM(0));
            }
            if FindNextChangeNotification(h).is_err() {
                break;
            }
        }
    });
}
