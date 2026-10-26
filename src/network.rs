//! Status internet dan Wi-Fi, dari event NetworkStatusChanged milik Windows.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Networking::Connectivity::*;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Ada perubahan jaringan; baca `current()` setelah ditunda sebentar (event sering beruntun).
pub const WM_APP_NET: u32 = WM_APP + 8;

static TARGET: AtomicIsize = AtomicIsize::new(0);

#[derive(Clone, PartialEq, Debug)]
pub struct NetState {
    pub online: bool,
    /// Nama Wi-Fi; kosong untuk kabel, atau bila Windows tidak mengizinkan membacanya.
    pub ssid: String,
}

pub fn current() -> NetState {
    let Ok(p) = NetworkInformation::GetInternetConnectionProfile() else {
        return NetState { online: false, ssid: String::new() };
    };
    let online = p.GetNetworkConnectivityLevel().is_ok_and(|l| l == NetworkConnectivityLevel::InternetAccess);
    let ssid = if p.IsWlanConnectionProfile().unwrap_or(false) {
        p.WlanConnectionProfileDetails()
            .and_then(|d| d.GetConnectedSsid())
            .map(|s| s.to_string_lossy())
            .unwrap_or_default()
    } else {
        String::new()
    };
    NetState { online, ssid }
}

pub fn watch(hwnd: HWND) {
    TARGET.store(hwnd.0 as isize, Ordering::Relaxed);
    let handler = NetworkStatusChangedEventHandler::new(|_| {
        unsafe {
            let _ = PostMessageW(HWND(TARGET.load(Ordering::Relaxed) as _), WM_APP_NET, WPARAM(0), LPARAM(0));
        }
        Ok(())
    });
    let _ = NetworkInformation::NetworkStatusChanged(&handler);
}
