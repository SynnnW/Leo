//! Event sistem untuk OSD: volume (callback dari Windows, termasuk saat perangkat
//! audio default berganti) dan kecerahan layar (notifikasi power setting).

use std::ffi::c_void;

use windows::core::{implement, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Media::Audio::Endpoints::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Power::*;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING, GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS,
};
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::Win32::UI::WindowsAndMessaging::*;

/// wParam = volume 0..100, lParam = 1 bila mute.
pub const WM_APP_VOLUME: u32 = WM_APP + 4;
/// Perangkat audio default berganti (mis. headphone dicolok): daftarkan ulang.
pub const WM_APP_AUDIO_DEVICE: u32 = WM_APP + 5;

/// Saat Leo sendiri mengubah volume (sleep timer), jangan tampilkan OSD.
static QUIET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_quiet(on: bool) {
    QUIET.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn default_volume() -> Option<IAudioEndpointVolume> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?.Activate(CLSCTX_ALL, None).ok()
    }
}

/// Volume master perangkat output default, 0..1.
pub fn master_volume() -> Option<f32> {
    unsafe { default_volume()?.GetMasterVolumeLevelScalar().ok() }
}

pub fn set_master_volume(v: f32) {
    if let Some(ep) = default_volume() {
        unsafe {
            let _ = ep.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null());
        }
    }
}

fn post(hwnd: isize, msg: u32, wp: usize, lp: isize) {
    unsafe {
        let _ = PostMessageW(HWND(hwnd as _), msg, WPARAM(wp), LPARAM(lp));
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback {
    hwnd: isize,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        if QUIET.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        if let Some(d) = unsafe { data.as_ref() } {
            let level = (d.fMasterVolume * 100.0).round() as usize;
            post(self.hwnd, WM_APP_VOLUME, level, d.bMuted.as_bool() as isize);
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceCallback {
    hwnd: isize,
}

impl IMMNotificationClient_Impl for DeviceCallback_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            post(self.hwnd, WM_APP_AUDIO_DEVICE, 0, 0);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// Menjaga callback tetap terdaftar selama objek ini hidup.
pub struct AudioWatch {
    enumerator: IMMDeviceEnumerator,
    devices: IMMNotificationClient,
    volume: Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)>,
    hwnd: isize,
}

impl AudioWatch {
    pub fn new(hwnd: HWND) -> Option<Self> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let hwnd = hwnd.0 as isize;
            let devices: IMMNotificationClient = DeviceCallback { hwnd }.into();
            enumerator.RegisterEndpointNotificationCallback(&devices).ok()?;
            let mut w = Self { enumerator, devices, volume: None, hwnd };
            w.rebind();
            Some(w)
        }
    }

    /// Pasang callback volume ke perangkat output default saat ini.
    /// Mengembalikan nama perangkat itu dan apakah ia headphone/headset.
    pub fn rebind(&mut self) -> Option<(String, bool)> {
        unsafe {
            if let Some((ep, cb)) = self.volume.take() {
                let _ = ep.UnregisterControlChangeNotify(&cb);
            }
            let dev = self.enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            if let Ok(ep) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                let cb: IAudioEndpointVolumeCallback = VolumeCallback { hwnd: self.hwnd }.into();
                if ep.RegisterControlChangeNotify(&cb).is_ok() {
                    self.volume = Some((ep, cb));
                }
            }
            device_info(&dev)
        }
    }
}

/// Nama perangkat output default saat ini (untuk diagnostik).
pub fn default_output() -> Option<(String, bool)> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        device_info(&en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?)
    }
}

fn device_info(dev: &IMMDevice) -> Option<(String, bool)> {
    unsafe {
        let store = dev.OpenPropertyStore(STGM_READ).ok()?;
        let name = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let p = PropVariantToStringAlloc(&name).ok()?;
        let name = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const c_void));
        // EndpointFormFactor: 3 = Headphones, 5 = Headset.
        let form = store
            .GetValue(&PKEY_AudioEndpoint_FormFactor)
            .ok()
            .and_then(|v| u32::try_from(&v).ok())
            .unwrap_or(0);
        Some((name, form == 3 || form == 5))
    }
}

impl Drop for AudioWatch {
    fn drop(&mut self) {
        unsafe {
            if let Some((ep, cb)) = self.volume.take() {
                let _ = ep.UnregisterControlChangeNotify(&cb);
            }
            let _ = self.enumerator.UnregisterEndpointNotificationCallback(&self.devices);
        }
    }
}

pub enum Power {
    /// Kecerahan layar laptop, 0..100.
    Brightness(u32),
    /// `true` = charger tercolok.
    Charger(bool),
    /// Sisa baterai, 0..100.
    Battery(u32),
}

/// Minta WM_POWERBROADCAST saat kecerahan, sumber daya, atau sisa baterai berubah.
/// Windows langsung mengirim nilai saat ini sekali untuk tiap jenis setelah didaftarkan.
pub fn watch_power(hwnd: HWND) {
    for guid in [GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS, GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING] {
        unsafe {
            let _ = RegisterPowerSettingNotification(HANDLE(hwnd.0), &guid, DEVICE_NOTIFY_WINDOW_HANDLE);
        }
    }
}

pub fn power_from(wp: WPARAM, lp: LPARAM) -> Option<Power> {
    if wp.0 as u32 != PBT_POWERSETTINGCHANGE || lp.0 == 0 {
        return None;
    }
    unsafe {
        let s = &*(lp.0 as *const POWERBROADCAST_SETTING);
        if s.DataLength < 4 {
            return None;
        }
        let v = std::ptr::read_unaligned(s.Data.as_ptr() as *const u32);
        match s.PowerSetting {
            g if g == GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS => Some(Power::Brightness(v)),
            // 0 = AC (charger), 1 = baterai, 2 = UPS
            g if g == GUID_ACDC_POWER_SOURCE => Some(Power::Charger(v == 0)),
            g if g == GUID_BATTERY_PERCENTAGE_REMAINING => Some(Power::Battery(v)),
            _ => None,
        }
    }
}
