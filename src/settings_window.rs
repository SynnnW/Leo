//! Jendela Pengaturan sendiri: terpisah dari notch, bisa dipindah, dan tidak ikut tertutup
//! saat notch menyusut. Berisi beberapa halaman (Umum, Akun) yang sama-sama `Module`,
//! hanya digambar di jendela biasa dengan bilah judul dan tab.
//!
//! Dibuka dari menu tray ("Pengaturan…"). Setiap perubahan disimpan oleh halamannya sendiri,
//! lalu jendela utama diberi tahu lewat `WM_APP_SETTINGS` supaya setelan langsung berlaku.

use std::cell::RefCell;

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::modules::account_page::AccountPage;
use crate::modules::settings_page::SettingsPage;
use crate::modules::Module;
use crate::settings::SharedSettings;
use crate::supabase;

/// Dikirim ke jendela utama setelah ada setelan yang diubah di jendela ini.
pub const WM_APP_SETTINGS: u32 = WM_APP + 13;

const TITLE_H: f32 = 44.0;
const TAB_H: f32 = 22.0;
const TAB_W: f32 = 84.0;
const TIMER_REFRESH: usize = 1;
const BG: Color = Color(0.085, 0.085, 0.10, 1.0);

struct State {
    hwnd: HWND,
    main: HWND,
    g: Gfx,
    pages: Vec<(&'static str, Box<dyn Module>)>,
    tab: usize,
    tab_hits: Vec<Rect>,
    close: Rect,
    content: Rect,
    /// `supabase::version()` terakhir yang tergambar; berubah = ada data/status baru.
    seen_version: u32,
}

thread_local! {
    static WIN: RefCell<Option<State>> = const { RefCell::new(None) };
}

impl State {
    fn size_dip(&self) -> (f32, f32) {
        (self.g.px_w as f32 / self.g.scale, self.g.px_h as f32 / self.g.scale)
    }

    fn draw(&mut self) {
        let (w, h) = self.size_dip();
        if self.g.begin().is_err() {
            return;
        }
        self.seen_version = supabase::version();
        let full = Rect::new(0.0, 0.0, w, h);
        self.g.fill_rrect(full, 16.0, BG);
        self.g.stroke_rrect(full.inflate(-0.5), 16.0, 1.0, FAINT);
        self.g.text("Leo", Rect::new(20.0, 0.0, 60.0, TITLE_H), 14.0, 700, WHITE, Align::Left);
        self.g.text("Pengaturan", Rect::new(58.0, 0.0, 200.0, TITLE_H), 14.0, 400, DIM, Align::Left);
        self.g.fill_rrect(self.close, 8.0, FAINT);
        self.g.text("×", self.close, 16.0, 600, WHITE, Align::Center);

        // Tab halaman.
        self.tab_hits.clear();
        for (i, (name, _)) in self.pages.iter().enumerate() {
            let r = Rect::new(20.0 + i as f32 * (TAB_W + 6.0), TITLE_H - 8.0, TAB_W, TAB_H);
            if i == self.tab {
                self.g.fill_rrect(r, 8.0, FAINT);
            }
            self.g.text(name, r, 12.5, if i == self.tab { 600 } else { 400 }, if i == self.tab { WHITE } else { DIM }, Align::Center);
            self.tab_hits.push(r);
        }

        let content = self.content;
        let tab = self.tab;
        self.pages[tab].1.draw_expanded(&mut self.g, content);

        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut rc);
        }
        let _ = self.g.end(self.hwnd, POINT { x: rc.left, y: rc.top });
    }

    /// Titik (DIP, relatif jendela): bilah judul bisa diseret kecuali tombol tutup dan tab.
    fn hit_test(&self, x: f32, y: f32) -> u32 {
        let on_tab = self.tab_hits.iter().any(|r| r.contains(x, y));
        if (0.0..TITLE_H - 8.0).contains(&y) && !self.close.contains(x, y) && !on_tab {
            HTCAPTION
        } else {
            HTCLIENT
        }
    }
}

/// Buka jendela Pengaturan; bila sudah terbuka, bawa ke depan.
pub fn open(main: HWND, settings: SharedSettings) {
    let existing = WIN.with(|w| w.try_borrow().ok().and_then(|w| w.as_ref().map(|s| s.hwnd)));
    if let Some(h) = existing {
        unsafe {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    if let Err(e) = create(main, settings) {
        crate::crashlog::write(&format!("Jendela Pengaturan gagal dibuka ({e})"));
    }
}

fn create(main: HWND, settings: SharedSettings) -> Result<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = w!("LeoSettings");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            // Ikon exe (ID 1 dari build.rs) untuk tombol di taskbar dan Alt+Tab.
            #[allow(clippy::manual_dangling_ptr)]
            hIcon: LoadIconW(hinst, PCWSTR(1 as *const u16)).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc); // gagal bila sudah terdaftar; tidak masalah

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_APPWINDOW,
            class,
            w!("Leo - Pengaturan"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            hinst,
            None,
        )?;

        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        let pages: Vec<(&'static str, Box<dyn Module>)> = vec![
            ("Umum", Box::new(SettingsPage::new(settings.clone()))),
            ("Akun", Box::new(AccountPage::new(settings))),
        ];
        // Jendela seukuran halaman terbesar supaya tidak berubah ukuran saat pindah tab.
        let (cw, ch) = pages.iter().map(|(_, p)| p.expanded_size()).fold((0.0f32, 0.0f32), |a, s| (a.0.max(s.0), a.1.max(s.1)));
        let content_top = TITLE_H - 8.0 + TAB_H + 4.0;
        let (w_dip, h_dip) = (cw + 40.0, content_top + ch + 6.0);
        let (px_w, px_h) = ((w_dip * scale).round() as i32, (h_dip * scale).round() as i32);
        let g = Gfx::new(px_w, px_h, scale)?;

        // Tengah layar utama.
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        SetWindowPos(hwnd, None, (sw - px_w) / 2, ((sh - px_h) / 3).max(0), px_w, px_h, SWP_NOZORDER | SWP_NOACTIVATE)?;

        let state = State {
            hwnd,
            main,
            g,
            pages,
            tab: 0,
            tab_hits: Vec::new(),
            close: Rect::new(w_dip - 20.0 - 28.0, (TITLE_H - 28.0) / 2.0, 28.0, 28.0),
            content: Rect::new(0.0, content_top, w_dip, ch),
            seen_version: supabase::version(),
        };
        WIN.with(|w| *w.borrow_mut() = Some(state));
        redraw();
        // Login Google dan data makrab berjalan di latar belakang; gambar ulang saat ada kabar baru.
        SetTimer(hwnd, TIMER_REFRESH, 700, None);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

fn redraw() {
    WIN.with(|w| {
        if let Ok(mut w) = w.try_borrow_mut() {
            if let Some(s) = w.as_mut() {
                s.draw();
            }
        }
    });
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            // lParam = koordinat layar (piksel) -> DIP relatif jendela.
            let hit = WIN.with(|w| {
                let w = w.try_borrow().ok()?;
                let s = w.as_ref()?;
                let mut rc = RECT::default();
                unsafe { GetWindowRect(hwnd, &mut rc).ok()? };
                let sx = (lp.0 & 0xffff) as i16 as i32;
                let sy = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                Some(s.hit_test((sx - rc.left) as f32 / s.g.scale, (sy - rc.top) as f32 / s.g.scale))
            });
            match hit {
                Some(h) => LRESULT(h as isize),
                None => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
            }
        }
        WM_LBUTTONUP => {
            let mut changed_for: Option<HWND> = None;
            let mut close = false;
            WIN.with(|w| {
                let Ok(mut w) = w.try_borrow_mut() else { return };
                let Some(s) = w.as_mut() else { return };
                let x = (lp.0 & 0xffff) as i16 as f32 / s.g.scale;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / s.g.scale;
                if s.close.contains(x, y) {
                    close = true;
                } else if let Some(i) = s.tab_hits.iter().position(|r| r.contains(x, y)) {
                    s.tab = i;
                    s.draw();
                } else {
                    let (content, tab) = (s.content, s.tab);
                    if s.pages[tab].1.click(content, x, y) {
                        s.draw();
                        changed_for = Some(s.main);
                    }
                }
            });
            if close {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            if let Some(main) = changed_for {
                unsafe {
                    let _ = PostMessageW(main, WM_APP_SETTINGS, WPARAM(0), LPARAM(0));
                }
            }
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER_REFRESH => {
            WIN.with(|w| {
                if let Ok(mut w) = w.try_borrow_mut() {
                    if let Some(s) = w.as_mut() {
                        if supabase::version() != s.seen_version {
                            s.draw();
                        }
                    }
                }
            });
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                let _ = KillTimer(hwnd, TIMER_REFRESH);
            }
            WIN.with(|w| {
                if let Ok(mut w) = w.try_borrow_mut() {
                    *w = None;
                }
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
