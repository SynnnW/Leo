//! Riwayat clipboard: 5 teks terakhir, hanya di memori (tidak pernah disimpan ke disk).
//! Mati secara default. Teks dari password manager (yang menandai clipboard-nya
//! "jangan dipantau") dilewati.

use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_UNICODETEXT;

use super::{Event, Module};
use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::settings::SharedSettings;

const MAX_ITEMS: usize = 5;
const ROW: f32 = 26.0;
const COPIED_FOR: Duration = Duration::from_millis(1200);
const ACCENT: Color = Color(0.35, 0.85, 0.55, 1.0);

struct Item {
    text: String,
    at: Instant,
}

enum Hit {
    Copy(usize),
    Clear,
}

pub struct Clipboard {
    hwnd: HWND,
    settings: SharedSettings,
    items: Vec<Item>,
    hits: Vec<(Rect, Hit)>,
    /// Baris yang baru disalin ulang, untuk tanda "Disalin".
    copied: Option<(usize, Instant)>,
    /// Perubahan clipboard berikutnya berasal dari kita sendiri.
    skip_next: bool,
}

fn open(hwnd: HWND) -> bool {
    // Aplikasi lain mungkin sedang memegang clipboard; coba beberapa kali.
    for _ in 0..4 {
        if unsafe { OpenClipboard(hwnd) }.is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    false
}

/// Password manager menandai isinya supaya tidak dicatat riwayat clipboard.
fn excluded() -> bool {
    unsafe {
        let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        let ignore = RegisterClipboardFormatW(w!("Clipboard Viewer Ignore"));
        let history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if IsClipboardFormatAvailable(exclude).is_ok() || IsClipboardFormatAvailable(ignore).is_ok() {
            return true;
        }
        if IsClipboardFormatAvailable(history).is_ok() {
            if let Ok(h) = GetClipboardData(history) {
                let p = GlobalLock(HGLOBAL(h.0)) as *const u32;
                let allowed = !p.is_null() && *p != 0;
                let _ = GlobalUnlock(HGLOBAL(h.0));
                return !allowed;
            }
        }
        false
    }
}

fn read_text(hwnd: HWND) -> Option<String> {
    if !open(hwnd) {
        return None;
    }
    let text = unsafe {
        let mut out = None;
        if !excluded() && IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok() {
            if let Ok(h) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
                let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
                if !p.is_null() {
                    let mut n = 0;
                    while *p.add(n) != 0 {
                        n += 1;
                    }
                    out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                }
                let _ = GlobalUnlock(HGLOBAL(h.0));
            }
        }
        let _ = CloseClipboard();
        out
    };
    text.filter(|t| !t.trim().is_empty())
}

fn write_text(hwnd: HWND, s: &str) -> bool {
    if !open(hwnd) {
        return false;
    }
    let wide: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let ok = (|| {
            EmptyClipboard().ok()?;
            let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
            let p = GlobalLock(h) as *mut u16;
            if p.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
            let _ = GlobalUnlock(h);
            SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(h.0)).ok()
        })()
        .is_some();
        let _ = CloseClipboard();
        ok
    }
}

/// Isi format clipboard `fmt` sebagai byte (clipboard sudah dibuka).
unsafe fn read_bytes(fmt: u32) -> Option<Vec<u8>> {
    if IsClipboardFormatAvailable(fmt).is_err() {
        return None;
    }
    let h = GetClipboardData(fmt).ok()?;
    let g = HGLOBAL(h.0);
    let p = GlobalLock(g) as *const u8;
    if p.is_null() {
        return None;
    }
    let bytes = std::slice::from_raw_parts(p, GlobalSize(g)).to_vec();
    let _ = GlobalUnlock(g);
    Some(bytes)
}

unsafe fn put(fmt: u32, bytes: &[u8]) -> bool {
    let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { return false };
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    let _ = GlobalUnlock(h);
    SetClipboardData(fmt, HANDLE(h.0)).is_ok()
}

/// Tabel yang perlu dirapikan dari isi clipboard (CF_HTML dan teks), `None` bila bukan
/// tabel atau sudah dirapikan.
fn table_to_tidy(html: Option<&[u8]>, text: Option<&str>) -> Option<Vec<crate::tables::Row>> {
    // Sudah dirapikan (termasuk hasil tulisan kita sendiri barusan). Tanpa ini teks
    // Markdown-nya akan dirapikan lagi, ditulis lagi, dan begitu seterusnya.
    // `data-amora`: penanda lama dari sebelum nama aplikasi menjadi Leo.
    let own = |h: &[u8]| h.windows(8).any(|w| w == b"data-leo") || h.windows(10).any(|w| w == b"data-amora");
    if html.is_some_and(own) {
        return None;
    }
    html.and_then(crate::tables::cf_html_fragment)
        .and_then(|f| crate::tables::from_html(&f))
        .or_else(|| text.and_then(crate::tables::from_markdown))
}

/// Bila yang baru disalin adalah tabel (HTML dari blok/tombol Copy, atau Markdown),
/// ganti versi HTML-nya dengan tabel rapi bergaris. Teks biasanya tidak diubah, jadi
/// menempel ke editor teks tetap menghasilkan teks yang sama.
/// Mengembalikan (jumlah baris, jumlah kolom) bila tabel dirapikan.
pub fn tidy_table(hwnd: HWND) -> Option<(usize, usize)> {
    if !open(hwnd) {
        return None;
    }
    let html_fmt = unsafe { RegisterClipboardFormatW(w!("HTML Format")) };
    let (html, text) = unsafe {
        let skip = excluded();
        let html = if skip { None } else { read_bytes(html_fmt) };
        let text = if skip { None } else { read_bytes(CF_UNICODETEXT.0 as u32) };
        let _ = CloseClipboard();
        (html, text)
    };
    // Teks UTF-16 diakhiri NUL.
    let text = text.map(|b| {
        let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        String::from_utf16_lossy(&w)
    });
    let rows = table_to_tidy(html.as_deref(), text.as_deref())?;
    let cf = crate::tables::cf_html(&crate::tables::to_html(&rows));

    if !open(hwnd) {
        return None;
    }
    let ok = unsafe {
        let mut ok = EmptyClipboard().is_ok();
        let mut cf0 = cf;
        cf0.push(0);
        ok &= put(html_fmt, &cf0);
        if let Some(t) = &text {
            let wide: Vec<u8> = t.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
            ok &= put(CF_UNICODETEXT.0 as u32, &wide);
        }
        let _ = CloseClipboard();
        ok
    };
    ok.then(|| (rows.len(), rows[0].cells.len()))
}

/// Mulai/berhenti menerima WM_CLIPBOARDUPDATE.
pub fn listen(hwnd: HWND, on: bool) {
    unsafe {
        if on {
            let _ = AddClipboardFormatListener(hwnd);
        } else {
            let _ = RemoveClipboardFormatListener(hwnd);
        }
    }
}

fn ago(t: Instant) -> String {
    let s = t.elapsed().as_secs();
    match s {
        0..=59 => "baru saja".into(),
        60..=3599 => format!("{} mnt", s / 60),
        _ => format!("{} jam", s / 3600),
    }
}

impl Clipboard {
    pub fn new(hwnd: HWND, settings: SharedSettings) -> Self {
        Self { hwnd, settings, items: Vec::new(), hits: Vec::new(), copied: None, skip_next: false }
    }

    fn enabled(&self) -> bool {
        self.settings.borrow().clipboard
    }
}

impl Module for Clipboard {
    fn tab(&self) -> Option<&'static str> {
        self.enabled().then_some("Clipboard")
    }

    fn expanded_size(&self) -> (f32, f32) {
        (420.0, 44.0 + self.items.len().max(1) as f32 * ROW + 12.0)
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        self.hits.clear();
        g.text("Clipboard", Rect::new(r.x + 20.0, r.y + 10.0, 200.0, 24.0), 15.0, 600, WHITE, Align::Left);
        if self.items.is_empty() {
            let msg = "Salin teks apa saja; 5 terakhir muncul di sini";
            g.text(msg, Rect::new(r.x + 20.0, r.y + 40.0, r.w - 40.0, ROW), 12.5, 400, DIM, Align::Left);
            return;
        }
        let clear = Rect::new(r.x + r.w - 20.0 - 90.0, r.y + 10.0, 90.0, 24.0);
        g.text("Bersihkan", clear, 12.0, 400, DIM, Align::Right);
        self.hits.push((clear, Hit::Clear));

        let copied = self.copied.filter(|(_, t)| t.elapsed() < COPIED_FOR).map(|(i, _)| i);
        for (i, it) in self.items.iter().enumerate() {
            let row = Rect::new(r.x + 16.0, r.y + 40.0 + i as f32 * ROW, r.w - 32.0, ROW);
            if copied == Some(i) {
                g.fill_rrect(row, 8.0, FAINT);
            }
            let line = it.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
            g.text(line, Rect::new(row.x + 8.0, row.y, row.w - 8.0 - 80.0, ROW), 13.0, 400, WHITE, Align::Left);
            let (label, c) = if copied == Some(i) { ("Disalin".to_string(), ACCENT) } else { (ago(it.at), DIM) };
            g.text(&label, Rect::new(row.x, row.y, row.w - 8.0, ROW), 11.5, 400, c, Align::Right);
            self.hits.push((row, Hit::Copy(i)));
        }
    }

    fn click(&mut self, _r: Rect, x: f32, y: f32) -> bool {
        let Some(pos) = self.hits.iter().position(|(r, _)| r.contains(x, y)) else { return false };
        match self.hits[pos].1 {
            Hit::Copy(i) => {
                let text = self.items[i].text.clone();
                self.skip_next = true;
                if write_text(self.hwnd, &text) {
                    self.copied = Some((i, Instant::now()));
                } else {
                    self.skip_next = false;
                }
            }
            Hit::Clear => self.items.clear(),
        }
        true
    }

    fn on_event(&mut self, e: &Event) {
        if !matches!(e, Event::ClipboardChanged | Event::SettingsChanged) {
            return;
        }
        if !self.enabled() {
            self.items.clear();
            return;
        }
        if matches!(e, Event::SettingsChanged) {
            return;
        }
        if std::mem::take(&mut self.skip_next) {
            return;
        }
        let Some(text) = read_text(self.hwnd) else { return };
        self.items.retain(|i| i.text != text);
        self.items.insert(0, Item { text, at: Instant::now() });
        self.items.truncate(MAX_ITEMS);
        self.copied = None;
    }

    fn tick_ms(&self, expanded: bool) -> Option<u32> {
        // Hilangkan tanda "Disalin" tepat waktu.
        let (_, t) = self.copied?;
        (expanded && t.elapsed() < COPIED_FOR).then(|| (COPIED_FOR - t.elapsed()).as_millis() as u32 + 10)
    }
}

#[cfg(test)]
mod tests {
    use super::table_to_tidy;
    use crate::tables::{cf_html, to_html};

    #[test]
    fn tidy_once_not_forever() {
        let md = "| A | B |
|---|---|
| 1 | 2 |
| 3 | 4 |";
        let rows = table_to_tidy(None, Some(md)).expect("Markdown dirapikan");
        // Hasil tulisan Leo sendiri (HTML rapi + teks Markdown yang sama) tidak dirapikan lagi.
        let ours = cf_html(&to_html(&rows));
        assert!(table_to_tidy(Some(&ours), Some(md)).is_none());
    }
}
