//! Pop-up earbud: mendeteksi case earbud TWS yang dibuka di dekat laptop lewat iklan BLE,
//! lalu membaca persentase baterainya dari Windows.
//!
//! - Case terbuka: `BluetoothLEAdvertisementWatcher` mendengarkan iklan dengan service UUID
//!   0xFD92 (Qualcomm; dipakai antara lain oleh KZ Vxs). Iklannya hanya burst singkat dan
//!   alamatnya acak, jadi keputusan "terbuka" ada di `case_detect` (sinyal terkuat, bukan per alamat).
//! - Baterai: iklan itu tidak membawa data baterai. Angkanya diambil dari properti perangkat
//!   Windows (yang sama dengan persentase di Setelan > Bluetooth), hanya ada selama earbud
//!   tersambung ke PC ini. Satu angka saja, bukan kiri/kanan/case terpisah.
//!
//! Callback BLE berjalan di thread pool WinRT, jadi hasilnya dikirim ke jendela lewat PostMessage.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{w, GUID};
use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher, BluetoothLEScanningMode,
};
use windows::Foundation::TypedEventHandler;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDevicePropertyW,
    SetupDiGetDeviceRegistryPropertyW, DIGCF_ALLCLASSES, DIGCF_PRESENT, HDEVINFO, SPDRP_FRIENDLYNAME,
    SETUP_DI_REGISTRY_PROPERTY, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::{DEVPROPKEY, DEVPROPTYPE, DEVPROP_TYPE_BYTE};
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::case_detect::CaseDetector;
use crate::settings::EarbudsCfg;

/// wParam = 0: case terbuka. wParam = 1: angka baterai baru (baca `percent()`).
pub const WM_APP_EARBUDS: u32 = WM_APP + 12;

/// Service UUID 0xFD92 dalam bentuk 128-bit (basis UUID Bluetooth).
const SERVICE_UUID: GUID = GUID::from_u128(0x0000fd92_0000_1000_8000_00805f9b34fb);

/// Properti baterai perangkat Bluetooth di Windows: {104EA319-6EE2-4701-BD47-8DDBF425BBE5}, 2.
const BATTERY_KEY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

/// Setelah case terbuka, baterai dicoba dibaca tiap `POLL_EVERY` selama `POLL_TRIES` kali
/// (earbud mungkin baru tersambung beberapa detik kemudian, dan angkanya bisa bergeser).
const POLL_TRIES: u32 = 8;
const POLL_EVERY: Duration = Duration::from_secs(2);

/// Baterai terakhir yang terbaca (0..100); -1 = belum pernah terbaca.
static BATTERY: AtomicI32 = AtomicI32::new(-1);
static POLLING: AtomicBool = AtomicBool::new(false);

pub fn percent() -> Option<u32> {
    let v = BATTERY.load(Ordering::Relaxed);
    (v >= 0).then_some(v as u32)
}

fn post(hwnd: isize, wp: usize) {
    unsafe {
        let _ = PostMessageW(HWND(hwnd as _), WM_APP_EARBUDS, WPARAM(wp), LPARAM(0));
    }
}

/// Pemantau iklan BLE; berhenti saat dibuang.
pub struct Watch {
    watcher: BluetoothLEAdvertisementWatcher,
}

impl Watch {
    /// `None` bila Bluetooth tidak tersedia atau scan gagal dimulai.
    pub fn start(hwnd: HWND, cfg: &EarbudsCfg) -> Option<Self> {
        match Self::try_start(hwnd, cfg) {
            Ok(w) => Some(w),
            Err(e) => {
                crate::crashlog::write(&format!("Pop-up earbud: scan BLE gagal dimulai ({e})"));
                None
            }
        }
    }

    fn try_start(hwnd: HWND, cfg: &EarbudsCfg) -> windows::core::Result<Self> {
        let watcher = BluetoothLEAdvertisementWatcher::new()?;
        // Mode aktif = yang terbukti menangkap iklan earbud ini di Windows.
        watcher.SetScanningMode(BluetoothLEScanningMode::Active)?;
        watcher.AdvertisementFilter()?.Advertisement()?.ServiceUuids()?.Append(SERVICE_UUID)?;

        let detector = Arc::new(Mutex::new(CaseDetector::new(cfg.open_rssi)));
        let hwnd = hwnd.0 as isize;
        let name = cfg.device_match.clone();
        let handler = TypedEventHandler::<
            BluetoothLEAdvertisementWatcher,
            BluetoothLEAdvertisementReceivedEventArgs,
        >::new(move |_, args| {
            let Some(a) = args.as_ref() else { return Ok(()) };
            let rssi = a.RawSignalStrengthInDBm().unwrap_or(-127);
            let addr = a.BluetoothAddress().unwrap_or(0);
            let opened = match detector.lock() {
                Ok(mut d) => d.ingest(addr, rssi, Instant::now()),
                Err(_) => false,
            };
            if opened {
                post(hwnd, 0);
                poll_battery(hwnd, name.clone());
            }
            Ok(())
        });
        watcher.Received(&handler)?;
        watcher.Start()?;
        Ok(Self { watcher })
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.watcher.Stop();
    }
}

/// Baca baterai di thread sendiri (enumerasi perangkat bisa memakan waktu) dan beri tahu
/// jendela setiap kali angkanya baru atau berubah.
fn poll_battery(hwnd: isize, name: String) {
    if POLLING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(move || {
        for i in 0..POLL_TRIES {
            if i > 0 {
                std::thread::sleep(POLL_EVERY);
            }
            if let Some(p) = read_battery(&name) {
                if BATTERY.swap(p as i32, Ordering::Relaxed) != p as i32 {
                    post(hwnd, 1);
                }
            }
        }
        POLLING.store(false, Ordering::Relaxed);
    });
}

struct DevInfoSet(HDEVINFO);

impl Drop for DevInfoSet {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// String registri perangkat (UTF-16) atau kosong bila tidak ada.
unsafe fn device_string(set: HDEVINFO, info: &SP_DEVINFO_DATA, prop: SETUP_DI_REGISTRY_PROPERTY) -> String {
    let mut buf = [0u8; 512];
    if SetupDiGetDeviceRegistryPropertyW(set, info, prop, None, Some(&mut buf), None).is_err() {
        return String::new();
    }
    let wide: Vec<u16> =
        buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&wide)
}

/// Persentase baterai perangkat Bluetooth yang namanya memuat `name_part` (tanpa membedakan
/// huruf besar/kecil), mis. "vxs" untuk simpul "VXS Hands-Free AG". `None` bila tidak ada.
pub fn read_battery(name_part: &str) -> Option<u32> {
    let needle = name_part.to_lowercase();
    unsafe {
        let set = DevInfoSet(SetupDiGetClassDevsW(None, w!("BTHENUM"), None, DIGCF_PRESENT | DIGCF_ALLCLASSES).ok()?);
        let mut index = 0;
        loop {
            let mut info = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInfo(set.0, index, &mut info).is_err() {
                return None;
            }
            index += 1;
            if !device_string(set.0, &info, SPDRP_FRIENDLYNAME).to_lowercase().contains(&needle) {
                continue;
            }
            // Tidak semua simpul punya properti baterai; lanjut ke simpul berikutnya.
            let mut kind = DEVPROPTYPE::default();
            let mut value = [0u8; 4];
            if SetupDiGetDevicePropertyW(set.0, &info, &BATTERY_KEY, &mut kind, Some(&mut value), None, 0).is_ok()
                && kind == DEVPROP_TYPE_BYTE
                && value[0] <= 100
            {
                return Some(value[0] as u32);
            }
        }
    }
}
