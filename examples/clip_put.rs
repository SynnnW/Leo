//! Diagnostik: isi clipboard dengan potongan HTML + teks, seperti yang dilakukan browser
//! saat menyalin (OpenClipboard/SetClipboardData). Untuk menguji "Rapikan tabel".
//! Jalankan: cargo run --example clip_put -- <fragmen.html> <teks.txt>

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_UNICODETEXT;

fn cf_html(fragment: &str) -> Vec<u8> {
    let pre = "<html><body><!--StartFragment-->";
    let post = "<!--EndFragment--></body></html>";
    let header_len = format!("Version:0.9\r\nStartHTML:{0:010}\r\nEndHTML:{0:010}\r\nStartFragment:{0:010}\r\nEndFragment:{0:010}\r\n", 0).len();
    let sf = header_len + pre.len();
    let ef = sf + fragment.len();
    let eh = ef + post.len();
    format!("Version:0.9\r\nStartHTML:{header_len:010}\r\nEndHTML:{eh:010}\r\nStartFragment:{sf:010}\r\nEndFragment:{ef:010}\r\n{pre}{fragment}{post}\0").into_bytes()
}

unsafe fn put(fmt: u32, bytes: &[u8]) {
    let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).unwrap();
    let p = GlobalLock(h) as *mut u8;
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    let _ = GlobalUnlock(h);
    SetClipboardData(fmt, HANDLE(h.0)).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let frag = std::fs::read_to_string(&args[1]).unwrap();
    let text = std::fs::read_to_string(&args[2]).unwrap();
    let text = text.trim_start_matches('\u{feff}');
    unsafe {
        for _ in 0..20 {
            if OpenClipboard(None).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        EmptyClipboard().unwrap();
        put(RegisterClipboardFormatW(w!("HTML Format")), &cf_html(frag.trim_start_matches('\u{feff}')));
        let wide: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
        put(CF_UNICODETEXT.0 as u32, &wide);
        CloseClipboard().unwrap();
    }
    println!("clipboard diisi");
}
