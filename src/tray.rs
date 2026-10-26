//! Ikon di system tray. Ikonnya digambar di kode (pil putih + titik hijau),
//! jadi tidak perlu file .ico terpisah.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_TRAY: u32 = WM_APP + 3;
const ID: u32 = 1;

/// Pesan yang dikirim Explorer setelah taskbar dibuat ulang (mis. Explorer restart).
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub fn is_taskbar_created(msg: u32) -> bool {
    let id = TASKBAR_CREATED.load(Ordering::Relaxed);
    id != 0 && msg == id
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ID,
        ..Default::default()
    }
}

pub fn add(hwnd: HWND) {
    unsafe {
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        let mut nid = data(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = make_icon().unwrap_or_default();
        let tip: Vec<u16> = "Leo".encode_utf16().collect();
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
    }
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data(hwnd));
    }
}

/// Dengan NOTIFYICON_VERSION_4, jenis event ada di LOWORD(lParam).
pub fn wants_menu(lp: LPARAM) -> bool {
    let ev = (lp.0 & 0xffff) as u32;
    // NIN_KEYSELECT = NIN_SELECT | NINF_KEY (tidak diekspor oleh crate windows).
    ev == WM_CONTEXTMENU || ev == NIN_SELECT || ev == NIN_SELECT | 1
}

/// Cakupan anti-alias untuk kotak bulat (jarak bertanda), 0..1.
fn coverage(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hw - r);
    let qy = (py - cy).abs() - (hh - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let d = outside + qx.max(qy).min(0.0) - r;
    (0.5 - d).clamp(0.0, 1.0)
}

fn make_icon() -> Option<HICON> {
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        let s = size as f32;
        let mut px = vec![0u32; (size * size) as usize];
        for y in 0..size {
            for x in 0..size {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                // Pil putih (notch) di tengah.
                let pill = coverage(fx, fy, s / 2.0, s / 2.0, s * 0.46, s * 0.24, s * 0.24);
                // Titik hijau di kanan pil.
                let dot = coverage(fx, fy, s * 0.68, s / 2.0, s * 0.1, s * 0.1, s * 0.1);
                let (r, g, b) = if dot > 0.0 {
                    let k = dot;
                    (
                        (255.0 * (1.0 - k) + 89.0 * k) as u32,
                        (255.0 * (1.0 - k) + 217.0 * k) as u32,
                        (255.0 * (1.0 - k) + 140.0 * k) as u32,
                    )
                } else {
                    (255, 255, 255)
                };
                let a = (pill * 255.0) as u32;
                px[(y * size + x) as usize] = (a << 24) | (r << 16) | (g << 8) | b;
            }
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
        let mask_bits = vec![0u8; ((size + 15) / 16 * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr() as *const c_void));
        let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color);
        let _ = DeleteObject(mask);
        icon
    }
}
