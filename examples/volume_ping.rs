//! Diagnostik OSD: geser volume 1% lalu kembalikan ke semula (Windows hanya mengirim
//! notifikasi bila nilainya benar-benar berubah). OSD Amora harus muncul.
//! Jalankan: cargo run --example volume_ping

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

fn main() -> windows::core::Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let ep: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None)?;
        let level = ep.GetMasterVolumeLevelScalar()?;
        let nudged = if level > 0.5 { level - 0.01 } else { level + 0.01 };
        ep.SetMasterVolumeLevelScalar(nudged, std::ptr::null())?;
        std::thread::sleep(std::time::Duration::from_millis(150));
        ep.SetMasterVolumeLevelScalar(level, std::ptr::null())?;
        println!("volume kembali {:.0}%, mute: {}", level * 100.0, ep.GetMute()?.as_bool());
    }
    Ok(())
}
