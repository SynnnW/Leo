//! Setelan pengguna, disimpan di %LOCALAPPDATA%\Leo\settings.json.
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
    /// Skala tampilan notch dalam persen (70..=150), di atas skala DPI Windows.
    pub ui_scale: u32,
    pub earbuds: EarbudsCfg,
    pub ai: AiCfg,
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
            ui_scale: 100,
            earbuds: Default::default(),
            ai: Default::default(),
            pomodoro: Default::default(),
            water: Default::default(),
        }
    }
}

/// Chat AI lewat jendela browser (tanpa token API). Tiap profil = satu set login sendiri.
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AiCfg {
    /// Nama profil, mis. ["Utama", "Kedua"]: ganti akun tanpa logout. Maksimal 9 yang ditampilkan di menu.
    pub profiles: Vec<String>,
}

impl Default for AiCfg {
    fn default() -> Self {
        Self { profiles: vec!["Utama".into(), "Kedua".into()] }
    }
}

/// Pop-up saat case earbud TWS dibuka di dekat laptop (dideteksi lewat iklan BLE).
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct EarbudsCfg {
    /// Mati secara default: memindai iklan Bluetooth terus-menerus.
    pub enabled: bool,
    /// Nama yang tampil di pop-up.
    pub label: String,
    /// Sebagian nama perangkat di Windows untuk membaca baterai (tanpa membedakan huruf besar/kecil).
    pub device_match: String,
    /// Case dianggap terbuka bila sinyal lebih kuat dari ini (dBm). Berlaku setelah Leo dijalankan ulang.
    pub open_rssi: i32,
}

impl Default for EarbudsCfg {
    fn default() -> Self {
        Self { enabled: false, label: "KZ Vxs".into(), device_match: "vxs".into(), open_rssi: -50 }
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

impl Settings {
    /// Pengali ukuran notch dari `ui_scale`, dibatasi 0.7..=1.5 walau settings.json diisi aneh.
    pub fn ui_scale_factor(&self) -> f32 {
        self.ui_scale.clamp(70, 150) as f32 / 100.0
    }
}

pub fn data_dir() -> Option<PathBuf> {
    static MIGRATE: std::sync::Once = std::sync::Once::new();
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    let dir = base.join("Leo");
    MIGRATE.call_once(|| migrate_legacy_dir(&base.join("Amora"), &dir));
    Some(dir)
}

/// Aplikasi ini dulu bernama Amora. Bila folder data Leo belum ada tapi folder Amora ada,
/// salin isinya (setelan, tugas, jadwal, cache lirik) supaya tidak mulai dari nol. Folder lama dibiarkan.
fn migrate_legacy_dir(old: &std::path::Path, new: &std::path::Path) {
    fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
    if !new.exists() && old.is_dir() {
        let _ = copy_dir(old, new);
    }
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
/// penulisan terputus (listrik mati, Leo dimatikan paksa).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_file_without_earbuds_keeps_working() {
        // settings.json dari versi sebelum pop-up earbud tidak punya kolom "earbuds".
        let s: Settings = serde_json::from_str(r#"{ "lyrics": true, "minimal": true }"#).unwrap();
        assert!(s.lyrics && s.minimal);
        assert!(!s.earbuds.enabled, "pop-up earbud mati secara default");
        assert_eq!(s.earbuds.device_match, "vxs");
        assert_eq!(s.earbuds.open_rssi, -50);
    }

    #[test]
    fn ui_scale_is_clamped_and_defaults_to_100() {
        let mut s = Settings::default();
        assert_eq!(s.ui_scale_factor(), 1.0);
        s.ui_scale = 10;
        assert_eq!(s.ui_scale_factor(), 0.7);
        s.ui_scale = 900;
        assert_eq!(s.ui_scale_factor(), 1.5);
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.ui_scale, 100);
        assert_eq!(old.ai.profiles, vec!["Utama", "Kedua"]);
    }

    #[test]
    fn legacy_amora_folder_is_copied_once() {
        let root = std::env::temp_dir().join(format!("leo-mig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old = root.join("Amora");
        std::fs::create_dir_all(old.join("lyrics")).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        std::fs::write(old.join("lyrics").join("a.json"), "x").unwrap();
        let new = root.join("Leo");

        migrate_legacy_dir(&old, &new);
        assert!(new.join("settings.json").exists() && new.join("lyrics").join("a.json").exists());
        assert!(old.join("settings.json").exists(), "folder lama dibiarkan");

        // Folder Leo sudah ada: isinya tidak ditimpa.
        std::fs::write(new.join("settings.json"), "baru").unwrap();
        migrate_legacy_dir(&old, &new);
        assert_eq!(std::fs::read_to_string(new.join("settings.json")).unwrap(), "baru");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn earbuds_section_can_be_partially_overridden() {
        let s: Settings = serde_json::from_str(r#"{ "earbuds": { "enabled": true, "label": "Buds" } }"#).unwrap();
        assert!(s.earbuds.enabled);
        assert_eq!(s.earbuds.label, "Buds");
        assert_eq!(s.earbuds.device_match, "vxs", "kolom yang tidak ditulis memakai nilai bawaan");
    }
}
