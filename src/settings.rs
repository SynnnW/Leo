//! Setelan pengguna, disimpan di %LOCALAPPDATA%\Amora\settings.json.
//! Semua modul memakai satu salinan bersama supaya simpan tidak saling menimpa.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

pub type SharedSettings = Rc<RefCell<Settings>>;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// Lirik dinyalakan pengguna. Mati secara default karena mengirim judul lagu ke LRCLIB.
    pub lyrics: bool,
    /// Notch minggir saat ada jendela di posisinya dan kursor mengarah ke jendela itu.
    pub auto_hide: bool,
    /// Mata muncul menggantikan jam saat PC lama tidak disentuh.
    pub mascot: bool,
    /// Notch tertutup jadi garis tipis (untuk laptop dengan bezel atas tebal).
    pub minimal: bool,
    /// Screenshot baru otomatis masuk Shelf.
    pub screenshot_to_shelf: bool,
    /// Riwayat clipboard (hanya di memori). Mati secara default demi privasi.
    pub clipboard: bool,
    /// Tabel yang disalin dari chat AI dirapikan (bergaris, lebar kolom pas) sebelum ditempel.
    pub tidy_tables: bool,
    pub pomodoro: PomodoroCfg,
    pub water: WaterCfg,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lyrics: false,
            auto_hide: true,
            mascot: true,
            minimal: false,
            screenshot_to_shelf: true,
            clipboard: false,
            tidy_tables: true,
            pomodoro: Default::default(),
            water: Default::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct PomodoroCfg {
    pub focus_min: u32,
    pub short_break_min: u32,
    pub long_break_min: u32,
    /// Istirahat panjang setelah sekian sesi fokus.
    pub rounds: u32,
}

impl Default for PomodoroCfg {
    fn default() -> Self {
        Self { focus_min: 25, short_break_min: 5, long_break_min: 15, rounds: 4 }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct WaterCfg {
    pub goal_ml: u32,
    /// Jumlah yang ditambahkan tombol "Minum".
    pub sip_ml: u32,
    pub remind: bool,
    pub interval_min: u32,
    pub start_hour: u32,
    pub end_hour: u32,
}

impl Default for WaterCfg {
    fn default() -> Self {
        Self { goal_ml: 1500, sip_ml: 250, remind: true, interval_min: 60, start_hour: 8, end_hour: 21 }
    }
}

pub fn data_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Amora"))
}

/// Baca file JSON di folder data; `None` bila belum ada atau rusak.
/// File yang rusak disisihkan jadi `<nama>.rusak` supaya tidak langsung tertimpa.
pub fn load_json<T: for<'de> Deserialize<'de>>(name: &str) -> Option<T> {
    let path = data_dir()?.join(name);
    let s = std::fs::read_to_string(&path).ok()?;
    // File yang diedit tangan (Notepad, PowerShell) bisa diawali BOM.
    let v = serde_json::from_str(s.trim_start_matches('\u{feff}')).ok();
    if v.is_none() {
        let _ = std::fs::rename(&path, path.with_extension("rusak"));
    }
    v
}

/// Tulis ke file sementara lalu ganti nama, supaya file lama tetap utuh bila
/// penulisan terputus (listrik mati, Amora dimatikan paksa).
pub fn save_json<T: Serialize>(name: &str, v: &T) {
    let Some(dir) = data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(s) = serde_json::to_string_pretty(v) {
        let tmp = dir.join(format!("{name}.tmp"));
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join(name));
        }
    }
}

impl Settings {
    /// Muat lalu tulis ulang, supaya kolom baru (dengan nilai bawaan) muncul di file
    /// dan bisa diubah pengguna.
    pub fn load_shared() -> SharedSettings {
        let s: Settings = load_json("settings.json").unwrap_or_default();
        s.save();
        Rc::new(RefCell::new(s))
    }

    pub fn save(&self) {
        save_json("settings.json", self);
    }
}
