//! Shelf: seret file ke notch untuk disimpan sementara, lalu seret keluar ke chat,
//! email, atau folder lain. Yang disimpan hanya lokasi file, bukan salinannya.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use windows::core::HSTRING;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::Graphics::Gdi::{DeleteObject, HPALETTE};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::{ReleaseStgMedium, CF_HDROP};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_SHOWNORMAL, WM_APP};

use super::Module;
use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::settings::{load_json, save_json};

const FILE: &str = "shelf.json";
const ACCENT: Color = Color(0.35, 0.72, 1.0, 1.0);
const MAX_TILES: usize = 5;
const TILE_W: f32 = 76.0;
/// Thumbnail selesai dibuat di thread latar; gambar ulang bila Shelf sedang terlihat.
pub const WM_APP_THUMB: u32 = WM_APP + 11;

/// Piksel PBGRA hasil thumbnail, dikirim dari thread latar.
struct Pixels {
    w: u32,
    h: u32,
    data: Vec<u8>,
}

enum Thumb {
    /// Belum diminta.
    None,
    /// Sedang dibuat di thread latar.
    Loading,
    /// `None` bila Explorer tidak punya thumbnail untuk file ini.
    Ready(Option<IWICBitmap>),
}

struct Item {
    path: PathBuf,
    name: String,
    size: String,
    /// Thumbnail dimuat saat pertama kali digambar.
    thumb_src: Thumb,
    thumb: Option<(u64, ID2D1Bitmap)>,
}

enum Hit {
    Open(usize),
    Remove(usize),
    Clear,
}

pub struct Shelf {
    wic: IWICImagingFactory,
    hwnd: HWND,
    thumbs_tx: Sender<(PathBuf, Option<Pixels>)>,
    thumbs_rx: Receiver<(PathBuf, Option<Pixels>)>,
    items: Vec<Item>,
    /// File sedang diseret di atas notch.
    hover_drop: bool,
    hits: Vec<(Rect, Hit)>,
}

fn size_label(p: &Path) -> String {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => "Folder".into(),
        Ok(m) => {
            let b = m.len() as f64;
            if b >= 1e9 {
                format!("{:.1} GB", b / 1e9).replace('.', ",")
            } else if b >= 1e6 {
                format!("{:.1} MB", b / 1e6).replace('.', ",")
            } else {
                format!("{} KB", (b / 1e3).ceil() as u64)
            }
        }
        Err(_) => "Hilang".into(),
    }
}

fn item(path: PathBuf) -> Item {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let size = size_label(&path);
    Item { path, name, size, thumb_src: Thumb::None, thumb: None }
}

/// Thumbnail dari Explorer (foto/video asli, atau ikon jenis file).
/// Bisa lambat (video, file di jaringan), jadi dijalankan di thread latar.
fn thumbnail(wic: &IWICImagingFactory, path: &Path) -> Option<Pixels> {
    unsafe {
        let f: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let hbmp = f.GetImage(SIZE { cx: 96, cy: 96 }, SIIGBF_RESIZETOFIT).ok()?;
        let src = wic.CreateBitmapFromHBITMAP(hbmp, HPALETTE::default(), WICBitmapUsePremultipliedAlpha);
        let _ = DeleteObject(hbmp);
        let src = src.ok()?;
        let conv = wic.CreateFormatConverter().ok()?;
        conv.Initialize(&src, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        conv.GetSize(&mut w, &mut h).ok()?;
        let mut data = vec![0u8; (w * h * 4) as usize];
        conv.CopyPixels(std::ptr::null(), w * 4, &mut data).ok()?;
        Some(Pixels { w, h, data })
    }
}

/// Buat thumbnail `paths` satu per satu di thread latar, kirim hasilnya ke `tx`.
fn load_thumbs(paths: Vec<PathBuf>, tx: Sender<(PathBuf, Option<Pixels>)>, hwnd: HWND) {
    let hwnd_raw = hwnd.0 as isize;
    std::thread::spawn(move || unsafe {
        // Shell (IShellItemImageFactory) butuh apartemen STA.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let wic: Option<IWICImagingFactory> = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok();
        for p in paths {
            let px = wic.as_ref().and_then(|w| thumbnail(w, &p));
            if tx.send((p, px)).is_err() {
                break;
            }
            let _ = PostMessageW(HWND(hwnd_raw as _), WM_APP_THUMB, WPARAM(0), LPARAM(0));
        }
        drop(wic);
        CoUninitialize();
    });
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

/// Data yang diseret berisi file (bukan teks/gambar dari browser).
pub fn has_files(data: &IDataObject) -> bool {
    unsafe { data.QueryGetData(&hdrop_format()).is_ok() }
}

pub fn files_from(data: &IDataObject) -> Vec<PathBuf> {
    let mut out = Vec::new();
    unsafe {
        let Ok(mut medium) = data.GetData(&hdrop_format()) else { return out };
        let hdrop = HDROP(medium.u.hGlobal.0);
        let n = DragQueryFileW(hdrop, u32::MAX, None);
        for i in 0..n {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(hdrop, i, Some(&mut buf));
            out.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
        }
        ReleaseStgMedium(&mut medium);
    }
    out
}

/// Mulai menyeret file keluar dari notch (dengan gambar seret bawaan Windows).
/// Hanya salin/tautan, tidak pernah memindahkan file aslinya.
/// `true` bila file benar-benar dilepas di suatu tempat (bukan dibatalkan).
pub fn drag_out(hwnd: HWND, path: &Path) -> bool {
    unsafe {
        let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path.as_os_str()), None) else {
            return false;
        };
        let Ok(data) = item.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else { return false };
        use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_NONE};
        SHDoDragDrop(hwnd, &data, None, DROPEFFECT_COPY | DROPEFFECT_LINK).is_ok_and(|e| e != DROPEFFECT_NONE)
    }
}

impl Shelf {
    pub fn new(wic: IWICImagingFactory, hwnd: HWND) -> Self {
        let paths: Vec<PathBuf> = load_json(FILE).unwrap_or_default();
        let items = paths.into_iter().filter(|p| p.exists()).map(item).collect();
        let (thumbs_tx, thumbs_rx) = channel();
        Self { wic, hwnd, thumbs_tx, thumbs_rx, items, hover_drop: false, hits: Vec::new() }
    }

    /// Ambil thumbnail yang sudah jadi, dan minta yang belum untuk ubin yang terlihat.
    fn update_thumbs(&mut self) {
        while let Ok((path, px)) = self.thumbs_rx.try_recv() {
            let src = px.and_then(|p| unsafe {
                self.wic
                    .CreateBitmapFromMemory(p.w, p.h, &GUID_WICPixelFormat32bppPBGRA, p.w * 4, &p.data)
                    .ok()
            });
            if let Some(it) = self.items.iter_mut().find(|i| i.path == path) {
                it.thumb_src = Thumb::Ready(src);
                it.thumb = None;
            }
        }
        let mut want = Vec::new();
        for i in self.visible() {
            let it = &mut self.items[i];
            if matches!(it.thumb_src, Thumb::None) {
                it.thumb_src = Thumb::Loading;
                want.push(it.path.clone());
            }
        }
        if !want.is_empty() {
            load_thumbs(want, self.thumbs_tx.clone(), self.hwnd);
        }
    }

    fn save(&self) {
        let paths: Vec<&PathBuf> = self.items.iter().map(|i| &i.path).collect();
        save_json(FILE, &paths);
    }

    /// Yang terbaru di depan.
    fn visible(&self) -> Vec<usize> {
        (0..self.items.len()).rev().take(MAX_TILES).collect()
    }

    fn hit(&self, x: f32, y: f32) -> Option<&Hit> {
        self.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, h)| h)
    }
}

impl Module for Shelf {
    fn tab(&self) -> Option<&'static str> {
        Some("Shelf")
    }

    fn expanded_size(&self) -> (f32, f32) {
        (420.0, 136.0)
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        self.hits.clear();
        let title = if self.items.len() > MAX_TILES {
            format!("Shelf · {} file", self.items.len())
        } else {
            "Shelf".to_string()
        };
        g.text(&title, Rect::new(r.x + 20.0, r.y + 10.0, 200.0, 24.0), 15.0, 600, WHITE, Align::Left);
        let zone = Rect::new(r.x + 16.0, r.y + 40.0, r.w - 32.0, r.h - 50.0);

        if self.items.is_empty() || self.hover_drop {
            let (c, text) = if self.hover_drop {
                (ACCENT, "Lepas untuk menambahkan")
            } else {
                (FAINT, "Seret file ke sini")
            };
            if self.hover_drop && !self.items.is_empty() {
                g.fill_rrect(zone, 12.0, Color(0.0, 0.0, 0.0, 1.0));
            }
            g.stroke_rrect(zone, 12.0, 1.5, c);
            let tc = if self.hover_drop { ACCENT } else { DIM };
            g.text(text, zone, 13.0, 600, tc, Align::Center);
            return;
        }

        let clear = Rect::new(r.x + r.w - 20.0 - 80.0, r.y + 10.0, 80.0, 24.0);
        g.text("Bersihkan", clear, 12.0, 400, DIM, Align::Right);
        self.hits.push((clear, Hit::Clear));

        self.update_thumbs();
        for (k, i) in self.visible().into_iter().enumerate() {
            let x = zone.x + k as f32 * TILE_W;
            let tile = Rect::new(x, zone.y, TILE_W - 6.0, zone.h);
            let it = &mut self.items[i];
            if let Thumb::Ready(Some(src)) = &it.thumb_src {
                if it.thumb.as_ref().map(|t| t.0) != Some(g.generation) {
                    it.thumb = g.bitmap(src).map(|b| (g.generation, b));
                }
            }
            let tr = Rect::new(x + (tile.w - 48.0) / 2.0, tile.y + 2.0, 48.0, 48.0);
            match &it.thumb {
                Some((_, bmp)) => g.image(bmp, tr, 8.0),
                None => g.fill_rrect(tr, 8.0, FAINT),
            }
            g.text(&it.name, Rect::new(tile.x, tile.y + 52.0, tile.w, 16.0), 11.5, 400, WHITE, Align::Center);
            g.text(&it.size, Rect::new(tile.x, tile.y + 67.0, tile.w, 14.0), 10.5, 400, DIM, Align::Center);

            let rm = Rect::new(tile.x + tile.w - 16.0, tile.y - 4.0, 16.0, 16.0);
            g.circle(rm.x + 8.0, rm.y + 8.0, 7.0, Color(0.25, 0.25, 0.25, 1.0));
            g.text("×", rm, 12.0, 600, WHITE, Align::Center);
            self.hits.push((rm, Hit::Remove(i)));
            self.hits.push((tile, Hit::Open(i)));
        }
    }

    fn click(&mut self, _r: Rect, x: f32, y: f32) -> bool {
        match self.hit(x, y) {
            Some(Hit::Open(i)) => {
                let path = HSTRING::from(self.items[*i].path.as_os_str());
                unsafe {
                    ShellExecuteW(None, &HSTRING::from("open"), &path, None, None, SW_SHOWNORMAL);
                }
                false
            }
            Some(Hit::Remove(i)) => {
                self.items.remove(*i);
                self.save();
                true
            }
            Some(Hit::Clear) => {
                self.items.clear();
                self.save();
                true
            }
            None => false,
        }
    }

    fn accepts_drop(&self) -> bool {
        true
    }

    fn drop_hover(&mut self, on: bool) {
        self.hover_drop = on;
    }

    fn drop_files(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            if !self.items.iter().any(|i| i.path == p) {
                self.items.push(item(p));
            }
        }
        self.save();
    }

    /// Sudah dipakai di tempat lain: lepas dari shelf (file aslinya tetap ada).
    fn dragged_out(&mut self, path: &Path) {
        self.items.retain(|i| i.path != path);
        self.save();
    }

    fn drag_source(&self, x: f32, y: f32) -> Option<PathBuf> {
        match self.hit(x, y) {
            Some(Hit::Open(i)) => Some(self.items[*i].path.clone()),
            _ => None,
        }
    }
}
