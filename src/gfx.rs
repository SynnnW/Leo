//! Lapisan gambar: Direct2D (software) ke DIB, lalu UpdateLayeredWindow.
//! Piksel transparan otomatis tembus klik, jadi area di sekitar notch tidak menghalangi.

use std::collections::HashMap;
use std::ffi::c_void;

use windows::core::*;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
    pub fn inflate(&self, d: f32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.h }
    }
}

#[derive(Clone, Copy)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

pub const WHITE: Color = Color(1.0, 1.0, 1.0, 1.0);
pub const BLACK: Color = Color(0.0, 0.0, 0.0, 1.0);
pub const DIM: Color = Color(1.0, 1.0, 1.0, 0.55);
pub const FAINT: Color = Color(1.0, 1.0, 1.0, 0.18);

/// Jenis format teks yang di-cache.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Font {
    /// Satu baris, dipotong elipsis.
    Line,
    /// Ikon Segoe MDL2 Assets.
    Icon,
    /// Boleh turun ke baris berikutnya.
    Wrap,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct Gfx {
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    pub wic: IWICImagingFactory,
    rt: ID2D1DCRenderTarget,
    mem_dc: HDC,
    dib: HBITMAP,
    old: HGDIOBJ,
    pub px_w: i32,
    pub px_h: i32,
    pub scale: f32,
    /// Opasitas global; dikalikan ke semua warna dan gambar.
    pub alpha: f32,
    /// Naik setiap render target dibuat ulang; bitmap lama harus dibuang.
    pub generation: u64,
    formats: HashMap<(u32, u16, Align, Font), IDWriteTextFormat>,
}

impl Gfx {
    pub fn new(px_w: i32, px_h: i32, scale: f32) -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let rt = Self::make_rt(&d2d, scale)?;
            let mem_dc = CreateCompatibleDC(None);
            let (dib, old) = Self::make_dib(mem_dc, px_w, px_h)?;
            Ok(Self {
                d2d,
                dw,
                wic,
                rt,
                mem_dc,
                dib,
                old,
                px_w,
                px_h,
                scale,
                alpha: 1.0,
                generation: 0,
                formats: HashMap::new(),
            })
        }
    }

    fn make_rt(d2d: &ID2D1Factory, scale: f32) -> Result<ID2D1DCRenderTarget> {
        // Software: kanvasnya kecil dan hanya digambar saat bergerak, jadi tidak perlu
        // memuat driver GPU (yang menambah puluhan MB RAM).
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        unsafe {
            let rt = d2d.CreateDCRenderTarget(&props)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            Ok(rt)
        }
    }

    unsafe fn make_dib(dc: HDC, w: i32, h: i32) -> Result<(HBITMAP, HGDIOBJ)> {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, dib);
        Ok((dib, old))
    }

    pub fn resize(&mut self, px_w: i32, px_h: i32, scale: f32) -> Result<()> {
        unsafe {
            SelectObject(self.mem_dc, self.old);
            let _ = DeleteObject(self.dib);
            let (dib, old) = Self::make_dib(self.mem_dc, px_w, px_h)?;
            self.dib = dib;
            self.old = old;
        }
        self.px_w = px_w;
        self.px_h = px_h;
        if scale != self.scale {
            self.scale = scale;
            self.rt = Self::make_rt(&self.d2d, scale)?;
            self.generation += 1;
        }
        Ok(())
    }

    pub fn begin(&mut self) -> Result<()> {
        let r = RECT { left: 0, top: 0, right: self.px_w, bottom: self.px_h };
        unsafe {
            self.rt.BindDC(self.mem_dc, &r)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        }
        self.alpha = 1.0;
        Ok(())
    }

    /// Selesai menggambar dan tampilkan ke jendela pada posisi layar `pos` (piksel).
    pub fn end(&mut self, hwnd: HWND, pos: POINT) -> Result<()> {
        unsafe {
            if let Err(e) = self.rt.EndDraw(None, None) {
                if e.code() == D2DERR_RECREATE_TARGET {
                    self.rt = Self::make_rt(&self.d2d, self.scale)?;
                    self.generation += 1;
                }
                return Err(e);
            }
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let size = SIZE { cx: self.px_w, cy: self.px_h };
            let src = POINT { x: 0, y: 0 };
            UpdateLayeredWindow(
                hwnd,
                None,
                Some(&pos),
                Some(&size),
                self.mem_dc,
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
    }

    fn color(&self, c: Color) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: c.3 * self.alpha }
    }

    fn brush(&self, c: Color) -> Option<ID2D1SolidColorBrush> {
        unsafe { self.rt.CreateSolidColorBrush(&self.color(c), None).ok() }
    }

    pub fn fill_rect(&self, r: Rect, c: Color) {
        if let Some(b) = self.brush(c) {
            unsafe { self.rt.FillRectangle(&r.d2d(), &b) };
        }
    }

    pub fn fill_rrect(&self, r: Rect, radius: f32, c: Color) {
        if let Some(b) = self.brush(c) {
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
            unsafe { self.rt.FillRoundedRectangle(&rr, &b) };
        }
    }

    pub fn polygon(&self, pts: &[(f32, f32)], c: Color) {
        let Some(b) = self.brush(c) else { return };
        unsafe {
            let Ok(geo) = self.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(D2D_POINT_2F { x: pts[0].0, y: pts[0].1 }, D2D1_FIGURE_BEGIN_FILLED);
            for p in &pts[1..] {
                sink.AddLine(D2D_POINT_2F { x: p.0, y: p.1 });
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_ok() {
                self.rt.FillGeometry(&geo, &b, None);
            }
        }
    }

    /// "Telinga" cekung di sudut atas notch: kotak `r`×`r` dikurangi seperempat lingkaran,
    /// menempel di tepi atas pada `x` (tepi kiri notch bila `left`, tepi kanan bila tidak).
    pub fn ear(&self, x: f32, r: f32, left: bool, c: Color) {
        let Some(b) = self.brush(c) else { return };
        let d = if left { -1.0 } else { 1.0 };
        unsafe {
            let Ok(geo) = self.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(D2D_POINT_2F { x: x + d * r, y: 0.0 }, D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(D2D_POINT_2F { x, y: 0.0 });
            sink.AddLine(D2D_POINT_2F { x, y: r });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: D2D_POINT_2F { x: x + d * r, y: 0.0 },
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: if left { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_CLOCKWISE },
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_ok() {
                self.rt.FillGeometry(&geo, &b, None);
            }
        }
    }

    fn format(&mut self, size: f32, weight: u16, align: Align, font: Font) -> Option<IDWriteTextFormat> {
        let key = ((size * 10.0) as u32, weight, align, font);
        if let Some(f) = self.formats.get(&key) {
            return Some(f.clone());
        }
        unsafe {
            let f = self
                .dw
                .CreateTextFormat(
                    if font == Font::Icon { w!("Segoe MDL2 Assets") } else { w!("Segoe UI") },
                    None,
                    DWRITE_FONT_WEIGHT(weight as i32),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("id-ID"),
                )
                .ok()?;
            let _ = f.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let _ = f.SetWordWrapping(if font == Font::Wrap {
                DWRITE_WORD_WRAPPING_WRAP
            } else {
                DWRITE_WORD_WRAPPING_NO_WRAP
            });
            if let Ok(sign) = self.dw.CreateEllipsisTrimmingSign(&f) {
                let trim = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                let _ = f.SetTrimming(&trim, &sign);
            }
            self.formats.insert(key, f.clone());
            Some(f)
        }
    }

    /// Teks satu baris; terlalu panjang dipotong dengan elipsis.
    pub fn text(&mut self, s: &str, r: Rect, size: f32, weight: u16, c: Color, align: Align) {
        self.draw_str(s, r, size, weight, c, align, Font::Line);
    }

    /// Teks yang boleh turun ke baris berikutnya (sisa yang tidak muat dipotong elipsis).
    pub fn text_wrapped(&mut self, s: &str, r: Rect, size: f32, weight: u16, c: Color, align: Align) {
        self.draw_str(s, r, size, weight, c, align, Font::Wrap);
    }

    /// Berapa baris yang dibutuhkan `s` bila dibungkus selebar `width`.
    pub fn line_count(&mut self, s: &str, width: f32, size: f32, weight: u16) -> u32 {
        let Some(f) = self.format(size, weight, Align::Left, Font::Wrap) else { return 1 };
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&wide, &f, width, 10_000.0) else { return 1 };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_err() {
                return 1;
            }
            m.lineCount.max(1)
        }
    }

    /// Lebar teks satu baris (DIP), untuk tahu apakah perlu digeser (marquee).
    pub fn text_width(&mut self, s: &str, size: f32, weight: u16) -> f32 {
        let Some(f) = self.format(size, weight, Align::Left, Font::Line) else { return 0.0 };
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&wide, &f, 10_000.0, 100.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_err() {
                return 0.0;
            }
            m.width
        }
    }

    /// Ikon dari font Segoe MDL2 Assets, mis. '\u{E767}' (speaker).
    pub fn icon(&mut self, glyph: char, r: Rect, size: f32, c: Color) {
        self.draw_str(&glyph.to_string(), r, size, 400, c, Align::Center, Font::Icon);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_str(&mut self, s: &str, r: Rect, size: f32, weight: u16, c: Color, align: Align, font: Font) {
        let Some(f) = self.format(size, weight, align, font) else { return };
        let Some(b) = self.brush(c) else { return };
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            self.rt.DrawText(
                &wide,
                &f,
                &r.d2d(),
                &b,
                D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// Tombol pil dengan label di tengah.
    pub fn button(&mut self, r: Rect, label: &str, bg: Color, fg: Color) {
        self.fill_rrect(r, r.h / 2.0, bg);
        self.text(label, r, 12.5, 600, fg, Align::Center);
    }

    pub fn stroke_rrect(&self, r: Rect, radius: f32, width: f32, c: Color) {
        if let Some(b) = self.brush(c) {
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
            unsafe { self.rt.DrawRoundedRectangle(&rr, &b, width, None) };
        }
    }

    pub fn circle(&self, cx: f32, cy: f32, rad: f32, c: Color) {
        self.fill_rrect(Rect::new(cx - rad, cy - rad, rad * 2.0, rad * 2.0), rad, c);
    }

    /// Lingkaran garis (mis. kotak centang bulat).
    pub fn ring(&self, cx: f32, cy: f32, rad: f32, width: f32, c: Color) {
        if let Some(b) = self.brush(c) {
            let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: rad, radiusY: rad };
            unsafe { self.rt.DrawEllipse(&e, &b, width, None) };
        }
    }

    pub fn push_clip(&self, r: Rect) {
        unsafe { self.rt.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    }

    pub fn pop_clip(&self) {
        unsafe { self.rt.PopAxisAlignedClip() };
    }

    /// Ubah bitmap WIC menjadi bitmap Direct2D milik render target saat ini.
    pub fn bitmap(&self, src: &IWICBitmap) -> Option<ID2D1Bitmap> {
        unsafe { self.rt.CreateBitmapFromWicBitmap(src, None).ok() }
    }

    /// Gambar bitmap dengan sudut membulat, direntangkan memenuhi `r`.
    pub fn image(&self, bmp: &ID2D1Bitmap, r: Rect, radius: f32) {
        unsafe {
            let size = bmp.GetSize();
            let props = D2D1_BITMAP_BRUSH_PROPERTIES {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            };
            let brush_props = D2D1_BRUSH_PROPERTIES {
                opacity: self.alpha,
                transform: Matrix3x2::identity(),
            };
            let Ok(brush) = self.rt.CreateBitmapBrush(bmp, Some(&props), Some(&brush_props)) else {
                return;
            };
            brush.SetTransform(&Matrix3x2 {
                M11: r.w / size.width.max(1.0),
                M12: 0.0,
                M21: 0.0,
                M22: r.h / size.height.max(1.0),
                M31: r.x,
                M32: r.y,
            });
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
            self.rt.FillRoundedRectangle(&rr, &brush);
        }
    }
}

/// Decode gambar (PNG/JPEG/dll.) dari memori lalu perkecil ke `max_px` supaya hemat RAM.
pub fn decode_image(wic: &IWICImagingFactory, bytes: &[u8], max_px: u32) -> Option<IWICBitmap> {
    unsafe {
        let stream = wic.CreateStream().ok()?;
        stream.InitializeFromMemory(bytes).ok()?;
        let decoder = wic
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).ok()?;
        let k = (max_px as f32 / w.max(h).max(1) as f32).min(1.0);
        let scaler = wic.CreateBitmapScaler().ok()?;
        scaler
            .Initialize(
                &frame,
                ((w as f32 * k) as u32).max(1),
                ((h as f32 * k) as u32).max(1),
                WICBitmapInterpolationModeFant,
            )
            .ok()?;
        let conv = wic.CreateFormatConverter().ok()?;
        conv.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .ok()?;
        // Salin piksel sekarang supaya `bytes` tidak perlu disimpan.
        wic.CreateBitmapFromSource(&conv, WICBitmapCacheOnLoad).ok()
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem_dc, self.old);
            let _ = DeleteObject(self.dib);
            let _ = DeleteDC(self.mem_dc);
        }
    }
}
