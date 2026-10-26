//! Catatan kejadian penting di %LOCALAPPDATA%\Leo\log.txt, terutama panic.
//! Build release memakai `panic = "abort"`, jadi tanpa ini crash hilang tanpa jejak.

use std::io::Write;

use windows::Win32::System::SystemInformation::GetLocalTime;

/// Log dipangkas bila melebihi ini, supaya tidak tumbuh tanpa batas.
const MAX_BYTES: u64 = 256 * 1024;

pub fn write(line: &str) {
    let Some(dir) = crate::settings::data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("log.txt");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, dir.join("log.old.txt"));
    }
    let t = unsafe { GetLocalTime() };
    let stamp = format!(
        "{}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    );
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{stamp}] Leo {} {line}", env!("CARGO_PKG_VERSION"));
    }
}

/// Catat setiap panic (di thread mana pun) sebelum proses berhenti.
pub fn install() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        write(&format!("PANIC di thread '{thread}' ({at}): {msg}"));
        default(info);
    }));
}
