//! Waktu lokal dalam milidetik (sejak 1601, zona waktu lokal), supaya gampang
//! dihitung: hari = ms / DAY, menit dalam hari = (ms % DAY) / MIN.

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Time::SystemTimeToFileTime;

pub const MIN: i64 = 60_000;
pub const DAY: i64 = 86_400_000;

pub fn now_ms() -> i64 {
    unsafe {
        let st = GetLocalTime();
        let mut ft = FILETIME::default();
        let _ = SystemTimeToFileTime(&st, &mut ft);
        (((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64) / 10_000
    }
}

pub fn day(ms: i64) -> i64 {
    ms / DAY
}

/// Hari dalam minggu, 0 = Minggu … 6 = Sabtu (1 Januari 1601 jatuh pada hari Senin).
pub fn weekday(ms: i64) -> u8 {
    ((day(ms) % 7 + 1) % 7) as u8
}

/// Menit sejak tengah malam.
pub fn minute_of_day(ms: i64) -> u32 {
    ((ms % DAY) / MIN) as u32
}

pub fn hhmm(ms: i64) -> String {
    let m = minute_of_day(ms);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// "mm:ss" untuk hitung mundur.
pub fn mmss(total_sec: u64) -> String {
    format!("{:02}:{:02}", total_sec / 60, total_sec % 60)
}
