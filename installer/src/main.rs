//! Installer Leo: satu exe kecil yang membawa leo.exe di dalamnya.
//! Memasang ke %LOCALAPPDATA%\Programs\Leo untuk pengguna ini saja (tanpa hak admin),
//! membuat shortcut Start Menu, mendaftar di Settings → Apps, lalu menjalankan Leo.

#![windows_subsystem = "windows"]

use std::path::{Path, PathBuf};
use std::time::Duration;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Disalin oleh build.rs dari leo.exe yang dibangun tools/build-installer.ps1.
const LEO_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/leo.exe"));
const VERSION: &str = env!("CARGO_PKG_VERSION");
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Leo";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn ask(text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    unsafe { MessageBoxW(None, &HSTRING::from(text), w!("Leo"), flags) }
}

fn install_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs").join("Leo"))
}

fn shortcut_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join(r"Microsoft\Windows\Start Menu\Programs\Leo.lnk"))
}

/// Tutup Leo yang sedang berjalan supaya exe-nya bisa ditimpa.
fn close_running() {
    unsafe {
        for _ in 0..30 {
            let Ok(h) = FindWindowW(w!("LeoNotch"), None) else { return };
            if h.is_invalid() {
                return;
            }
            let _ = PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0));
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn write_exe(path: &Path) -> std::io::Result<()> {
    // Exe lama mungkin baru saja ditutup dan masih terkunci sebentar.
    let mut last = Ok(());
    for _ in 0..20 {
        last = std::fs::write(path, LEO_EXE);
        if last.is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    last
}

fn make_shortcut(exe: &Path, lnk: &Path) -> Result<()> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let exe_w = HSTRING::from(exe.as_os_str());
        link.SetPath(&exe_w)?;
        link.SetIconLocation(&exe_w, 0)?;
        link.SetDescription(w!("Notch hidup di tepi atas layar"))?;
        if let Some(dir) = exe.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        let file: IPersistFile = link.cast()?;
        file.Save(&HSTRING::from(lnk.as_os_str()), TRUE)
    }
}

fn set_string(key: HKEY, name: &str, value: &str) {
    let v = wide(value);
    let n = wide(name);
    unsafe {
        let _ = RegSetValueExW(
            key,
            PCWSTR(n.as_ptr()),
            0,
            REG_SZ,
            Some(std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2)),
        );
    }
}

fn set_dword(key: HKEY, name: &str, value: u32) {
    let n = wide(name);
    unsafe {
        let _ = RegSetValueExW(key, PCWSTR(n.as_ptr()), 0, REG_DWORD, Some(&value.to_le_bytes()));
    }
}

/// Daftarkan di Settings → Apps supaya bisa di-uninstall dari sana.
fn register_uninstall(dir: &Path, exe: &Path) {
    let path = wide(UNINSTALL_KEY);
    let mut key = HKEY::default();
    unsafe {
        if RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), &mut key).is_err() {
            return;
        }
    }
    let exe_s = exe.display().to_string();
    set_string(key, "DisplayName", "Leo");
    set_string(key, "DisplayVersion", VERSION);
    set_string(key, "Publisher", "Leo");
    set_string(key, "DisplayIcon", &format!("{exe_s},0"));
    set_string(key, "InstallLocation", &dir.display().to_string());
    set_string(key, "UninstallString", &format!("\"{exe_s}\" --uninstall"));
    set_dword(key, "NoModify", 1);
    set_dword(key, "NoRepair", 1);
    set_dword(key, "EstimatedSize", (LEO_EXE.len() / 1024) as u32);
    unsafe {
        let _ = RegCloseKey(key);
    }
}

/// Mulai bersama Windows, menunjuk ke lokasi terpasang.
fn enable_autostart(exe: &Path) {
    let path = wide(RUN_KEY);
    let mut key = HKEY::default();
    unsafe {
        if RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), &mut key).is_ok() {
            set_string(key, "Leo", &format!("\"{}\"", exe.display()));
            let _ = RegCloseKey(key);
        }
    }
}

fn main() {
    // `/S`: pasang tanpa pertanyaan dan tanpa menjalankan Leo (otomatisasi/pengujian).
    let silent = std::env::args().any(|a| a.eq_ignore_ascii_case("/S"));
    let Some(dir) = install_dir() else { return };
    let exe = dir.join("leo.exe");
    let first_time = !exe.exists();
    let question = if first_time {
        format!(
            "Pasang Leo {VERSION}?\n\nLeo dipasang untuk akun Anda saja di:\n{}\n\nTanpa hak admin, dan bisa dihapus kapan saja dari Settings → Apps.",
            dir.display()
        )
    } else {
        format!("Perbarui Leo ke versi {VERSION}?\n\nSetelan, tugas, dan jadwal Anda tetap tersimpan.")
    };
    if !silent && ask(&question, MB_OKCANCEL | MB_ICONINFORMATION) != IDOK {
        return;
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    if !silent {
        close_running();
    }
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| write_exe(&exe)) {
        if !silent {
            ask(&format!("Leo gagal dipasang:\n{e}"), MB_OK | MB_ICONERROR);
        }
        std::process::exit(1);
    }
    if let Some(lnk) = shortcut_path() {
        let _ = make_shortcut(&exe, &lnk);
    }
    register_uninstall(&dir, &exe);
    if first_time {
        enable_autostart(&exe);
    }

    if silent {
        return;
    }
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(exe.as_os_str()), None, None, SW_SHOWNORMAL);
    }
    if first_time {
        ask(
            "Leo sudah terpasang dan berjalan di tengah atas layar.\n\nArahkan kursor ke notch untuk membukanya. Menu ada di ikon Leo di tray (pojok kanan bawah).",
            MB_OK | MB_ICONINFORMATION,
        );
    }
}
