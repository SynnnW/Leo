//! `amora.exe --uninstall`, dipanggil dari Settings → Apps. Kebalikan dari installer:
//! hapus autostart, shortcut, entri uninstall, (opsional) data pengguna, lalu folder program.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const UNINSTALL_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Amora");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn ask(text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    unsafe { MessageBoxW(None, &HSTRING::from(text), w!("Amora"), flags) }
}

fn install_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs").join("Amora"))
}

fn close_running() {
    unsafe {
        for _ in 0..30 {
            let Ok(h) = FindWindowW(w!("AmoraNotch"), None) else { return };
            if h.is_invalid() {
                return;
            }
            let _ = PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0));
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// `quiet`: tanpa pertanyaan (untuk pengujian/otomatisasi); `wipe` lalu menentukan data dihapus.
pub fn run(quiet: bool, wipe_data: bool) {
    if !quiet && ask("Hapus Amora dari komputer ini?", MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    let wipe = if quiet { wipe_data } else { ask(
        "Hapus juga data Anda?\n\nTugas, jadwal kuliah, catatan minum, isi Shelf, cache lirik, dan setelan.\nPilih \"No\" untuk menyimpannya (berguna bila nanti memasang Amora lagi).",
        MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
    ) == IDYES };

    close_running();
    crate::autostart::set(false);
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let _ = std::fs::remove_file(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\Amora.lnk"));
    }
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, UNINSTALL_KEY);
    }
    if wipe {
        if let Some(d) = crate::settings::data_dir() {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    // Exe yang sedang berjalan tidak bisa menghapus dirinya sendiri: serahkan ke cmd
    // yang menunggu sebentar. Hanya bila memang berjalan dari folder terpasang.
    let here = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    if let (Some(here), Some(dir)) = (here, install_dir()) {
        if here.to_string_lossy().eq_ignore_ascii_case(&dir.to_string_lossy()) {
            let _ = std::process::Command::new("cmd")
                .raw_arg(format!("/c ping 127.0.0.1 -n 3 >nul & rmdir /s /q \"{}\"", dir.display()))
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }
    }
    if !quiet {
        ask("Amora sudah dihapus. Terima kasih sudah mencobanya.", MB_OK | MB_ICONINFORMATION);
    }
}
