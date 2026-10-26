//! "Mulai bersama Windows" lewat HKCU\...\Run (tanpa hak admin).

use std::ffi::c_void;

use windows::core::*;
use windows::Win32::System::Registry::*;

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
/// Tempat Task Manager mencatat app startup yang dimatikan pengguna.
const APPROVED: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
const NAME: PCWSTR = w!("Leo");
/// Nama entri dari sebelum aplikasi ini bernama Leo; dibersihkan agar tidak jalan dobel.
const LEGACY_NAME: PCWSTR = w!("Amora");

fn exe_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

fn read_bytes(key: PCWSTR) -> Option<Vec<u8>> {
    unsafe {
        let mut len = 0u32;
        RegGetValueW(HKEY_CURRENT_USER, key, NAME, RRF_RT_ANY, None, None, Some(&mut len)).ok().ok()?;
        let mut buf = vec![0u8; len as usize];
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            NAME,
            RRF_RT_ANY,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut len),
        )
        .ok()
        .ok()?;
        buf.truncate(len as usize);
        Some(buf)
    }
}

/// Aktif bila ada di Run, menunjuk ke exe ini, dan tidak dimatikan lewat Task Manager.
pub fn is_enabled() -> bool {
    let Some(raw) = read_bytes(RUN) else { return false };
    let wide: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let value = String::from_utf16_lossy(&wide).trim_end_matches('\0').to_string();
    if Some(value.to_lowercase()) != exe_command().map(|s| s.to_lowercase()) {
        return false;
    }
    // Byte pertama ganjil (mis. 03) = dimatikan pengguna di Task Manager.
    !read_bytes(APPROVED).is_some_and(|b| b.first().is_some_and(|x| x & 1 == 1))
}

pub fn set(enabled: bool) {
    unsafe {
        if enabled {
            let Some(cmd) = exe_command() else { return };
            let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN,
                NAME,
                REG_SZ.0,
                Some(wide.as_ptr() as *const c_void),
                (wide.len() * 2) as u32,
            );
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, LEGACY_NAME);
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED, LEGACY_NAME);
            // Hapus tanda "dimatikan" lama supaya pilihan baru ini yang berlaku.
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED, NAME);
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME);
        }
    }
}
