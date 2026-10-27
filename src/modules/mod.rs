//! Setiap fitur notch adalah satu modul. Menambah fitur = menambah satu file di sini
//! lalu mendaftarkannya di `main.rs`.

pub mod clipboard;
pub mod account_page;
pub mod clock;
pub mod deadline;
pub mod earbuds;
pub mod media;
pub mod osd;
pub mod pomodoro;
pub mod schedule;
pub mod settings_page;
pub mod shelf;
pub mod todo;
pub mod water;

use std::path::{Path, PathBuf};

use crate::gfx::{Color, Gfx, Rect};

/// Hasil `Module::poll`.
#[derive(PartialEq)]
pub enum Poll {
    /// Tidak ada yang berubah; jangan gambar ulang.
    Idle,
    /// Tampilan berubah.
    Redraw,
    /// Minta perhatian: notch dibuka ke halaman modul ini.
    Alert,
}

pub enum Event {
    MediaChanged,
    LyricsChanged,
    Volume { level: u32, muted: bool },
    Brightness(u32),
    /// Notifikasi singkat, mis. Caps Lock atau charger dicolok.
    Notice { icon: &'static str, text: String, detail: String },
    ClipboardChanged,
    /// Setelan diubah (halaman Setelan atau menu tray).
    SettingsChanged,
    /// Case earbud terbuka (`opened`) atau angka baterainya baru terbaca.
    Earbuds { opened: bool, percent: Option<u32> },
    /// Data makrab/deadline dari Supabase diperbarui.
    Makrab,
}

pub trait Module {
    /// Nama halaman di notch yang terbuka. `None` = bukan halaman (mis. jam).
    fn tab(&self) -> Option<&'static str> {
        None
    }

    /// Ingin menempati notch tertutup. Modul aktif pertama (urutan pendaftaran) yang menang.
    fn active(&self) -> bool {
        false
    }

    /// Harus terlihat walau notch sedang bersembunyi (mis. OSD volume).
    fn urgent(&self) -> bool {
        false
    }

    /// Punya isi yang tetap ditampilkan walau mode notch minimal (mis. baris lirik).
    fn wants_space(&self) -> bool {
        false
    }

    /// Info kecil di ujung kanan notch tertutup saat modul lain yang tampil (mis. sisa waktu fokus).
    fn badge(&self) -> Option<(String, Color)> {
        None
    }

    /// Ukuran notch dalam DIP saat tertutup dan saat dibuka.
    fn compact_size(&self) -> (f32, f32) {
        (200.0, 32.0)
    }
    fn expanded_size(&self) -> (f32, f32);

    /// `r` adalah kotak notch saat ini (sedang dianimasikan).
    fn draw_compact(&mut self, _g: &mut Gfx, _r: Rect) {}
    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect);

    /// Klik di dalam halaman. Kembalikan `true` bila perlu digambar ulang.
    fn click(&mut self, _r: Rect, _x: f32, _y: f32) -> bool {
        false
    }

    fn on_event(&mut self, _e: &Event) {}

    /// Kapan perlu digambar ulang selama terlihat (ms). `None` = tidak perlu, CPU tetap 0.
    fn tick_ms(&self, _expanded: bool) -> Option<u32> {
        None
    }

    /// Kapan `poll` perlu dipanggil lagi (ms), untuk pengingat di latar belakang.
    fn poll_in(&self) -> Option<u32> {
        None
    }

    /// Cek pengingat atau perubahan keadaan di latar belakang.
    fn poll(&mut self) -> Poll {
        Poll::Idle
    }

    /// Sedang menerima ketikan (keyboard diarahkan ke modul ini).
    fn editing(&self) -> bool {
        false
    }
    fn key_char(&mut self, _c: char) {}
    fn stop_editing(&mut self) {}

    /// Menerima file yang diseret ke notch.
    fn accepts_drop(&self) -> bool {
        false
    }
    fn drop_hover(&mut self, _on: bool) {}
    fn drop_files(&mut self, _paths: Vec<PathBuf>) {}
    /// File yang mulai diseret keluar dari titik (x, y) di halaman ini.
    fn drag_source(&self, _x: f32, _y: f32) -> Option<PathBuf> {
        None
    }
    /// File dari `drag_source` berhasil dilepas di aplikasi lain.
    fn dragged_out(&mut self, _path: &Path) {}
}

/// Bunyi singkat untuk pengingat.
pub fn chime() {
    use windows::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;
    unsafe {
        let _ = MessageBeep(MB_ICONASTERISK);
    }
}
