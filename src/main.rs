#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
mod autostart;
mod crashlog;
mod gfx;
mod keyboard;
mod lyrics;
mod media;
mod modules;
mod network;
mod occlusion;
mod privacy;
mod screenshots;
mod settings;
mod system;
mod tables;
mod time;
mod tray;
mod uninstall;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use anim::Spring;
use gfx::{Align, Color, Gfx, Rect, BLACK, DIM, WHITE};
use modules::{
    clipboard, clipboard::Clipboard, clock::ClockModule, media::MediaModule, schedule::Schedule,
    settings_page::SettingsPage, Poll, osd::Osd, pomodoro::Pomodoro, shelf, shelf::Shelf, todo::Todo,
    water::Water, Event, Module,
};
use settings::Settings;

/// Ukuran kanvas jendela (DIP). Notch digambar di tengah atasnya; sisanya transparan.
const CANVAS_W: f32 = 760.0;
const CANVAS_H: f32 = 380.0;
/// Tinggi baris nama halaman di bawah notch yang terbuka.
const TAB_H: f32 = 30.0;
const MIN_OPEN_W: f32 = 360.0;
/// Lebar satu nama halaman di baris bawah notch yang terbuka.
const TAB_W: f32 = 62.0;
const BADGE_W: f32 = 64.0;
/// Lebar per titik indikator mic/kamera di notch tertutup.
const DOT_W: f32 = 12.0;
const MIC: Color = Color(1.0, 0.62, 0.2, 1.0);
const CAM: Color = Color(0.3, 0.85, 0.45, 1.0);
/// Tinggi garis notch dalam mode minimal.
const LINE_H: f32 = 5.0;

const WM_APP_MEDIA: u32 = WM_APP + 1;
const WM_APP_LYRICS: u32 = WM_APP + 2;
const WM_MOUSELEAVE: u32 = 0x02A3;
const TIMER_ANIM: usize = 1;
const TIMER_TICK: usize = 2;
const TIMER_POLL: usize = 3;
const TIMER_ALERT: usize = 4;
/// Saat ada jendela di posisi notch: cek posisi kursor untuk menyingkirkan notch.
const TIMER_DODGE: usize = 6;
/// Status jaringan dibaca setelah event berhenti beruntun (bisa naik-turun sebentar).
const TIMER_NET: usize = 7;
const NET_SETTLE_MS: u32 = 2000;
const LOW_BATTERY: [u32; 2] = [20, 10];
/// Proses clipboard sedikit setelah berubah, supaya aplikasi penyalin selesai menulis
/// dulu dan tidak terganggu oleh Amora yang ikut membuka clipboard.
const TIMER_CLIP: usize = 8;
const CLIP_SETTLE_MS: u32 = 150;
const DODGE_MS: u32 = 80;
/// Area (DIP) di sekitar notch: kursor di sini berarti mengarah ke tab di belakang notch.
const DODGE_MARGIN_X: f32 = EAR_R + 4.0;
const DODGE_MARGIN_Y: f32 = 4.0;
/// Kursor ditempel di tepi paling atas layar (piksel) = ingin membuka notch, bukan tab.
const TOP_EDGE_PX: i32 = 1;
/// Lama notch terbuka sendiri saat ada pengingat.
const ALERT_MS: u32 = 7000;
/// Kursor harus diam sebentar di atas notch sebelum ia terbuka, supaya tidak
/// terbuka sendiri saat kursor hanya lewat.
const TIMER_HOVER: usize = 9;
const HOVER_DELAY_MS: u32 = 150;
/// Jari-jari "telinga" cekung yang menyambungkan notch ke tepi atas layar.
const EAR_R: f32 = 8.0;

struct App {
    hwnd: HWND,
    g: Gfx,
    origin: POINT,
    modules: Vec<Box<dyn Module>>,
    /// Modul yang tampil saat notch tertutup.
    current: usize,
    /// Halaman yang tampil saat notch terbuka.
    page: usize,
    hover: bool,
    /// Kursor baru masuk; notch dibuka bila masih di sana setelah `HOVER_DELAY_MS`.
    hover_pending: bool,
    /// Notch dibuka sendiri oleh pengingat.
    alert: bool,
    /// Keyboard sedang diarahkan ke halaman (mis. mengetik tugas).
    editing: bool,
    tab_hits: Vec<(Rect, usize)>,
    /// File sedang diseret dari luar ke atas notch.
    drop_hover: bool,
    /// Titik tombol kiri ditekan (DIP), untuk mendeteksi seret keluar.
    press: Option<(f32, f32)>,
    /// File yang akan diseret keluar setelah `handle` selesai (di luar pinjaman APP).
    pending_drag: Option<PathBuf>,
    audio: Option<system::AudioWatch>,
    /// Notifikasi kecerahan pertama hanyalah nilai awal, bukan perubahan.
    brightness_ready: bool,
    /// Status terakhir untuk notifikasi: hanya perubahan yang ditampilkan.
    charger: Option<bool>,
    battery: Option<u32>,
    /// Batas baterai lemah terendah yang sudah diperingatkan sejak terakhir dicas.
    battery_warned: u32,
    net: network::NetState,
    settings: settings::SharedSettings,
    /// Sedang menerima WM_CLIPBOARDUPDATE.
    clip_listening: bool,
    /// Aplikasi yang sedang memakai mikrofon/kamera.
    in_use: privacy::InUse,
    auto_hide: bool,
    /// Ada jendela lain di posisi notch.
    covered: bool,
    /// Notch minggir karena kursor mengarah ke jendela di belakangnya.
    dodge: bool,
    w: Spring,
    h: Spring,
    last: Instant,
    animating: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Sedang menyeret file keluar dari notch; jangan terima sebagai drop ke diri sendiri.
    static SELF_DRAG: Cell<bool> = const { Cell::new(false) };
}

fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|a| {
        if let Ok(mut a) = a.try_borrow_mut() {
            if let Some(app) = a.as_mut() {
                f(app);
            }
        }
    });
}

impl App {
    fn active_index(&self) -> usize {
        self.modules.iter().position(|m| m.active()).unwrap_or(self.modules.len() - 1)
    }

    fn tabs(&self) -> Vec<usize> {
        (0..self.modules.len()).filter(|&i| self.modules[i].tab().is_some()).collect()
    }

    fn is_open(&self) -> bool {
        self.hover || self.alert || self.editing || self.drop_hover || force_open()
    }

    /// Info kecil dari modul lain untuk notch tertutup.
    fn badge(&self) -> Option<(usize, String, Color)> {
        (0..self.modules.len())
            .filter(|&i| i != self.current)
            .find_map(|i| self.modules[i].badge().map(|(s, c)| (i, s, c)))
    }

    /// Notch disembunyikan: ada jendela di belakangnya, kursor mengarah ke sana,
    /// dan tidak ada yang perlu ditampilkan.
    fn hidden(&self) -> bool {
        self.auto_hide && self.covered && self.dodge && !self.is_open() && !self.modules[self.current].urgent()
    }

    /// Kotak notch tertutup di layar (piksel).
    fn compact_screen_rect(&self, margin_x: f32, margin_y: f32) -> RECT {
        let (w, h) = self.compact_size();
        let s = self.g.scale;
        let left = self.origin.x + (((CANVAS_W - w) / 2.0 - margin_x) * s) as i32;
        RECT {
            left,
            top: self.origin.y,
            right: left + ((w + margin_x * 2.0) * s) as i32,
            bottom: self.origin.y + ((h + margin_y) * s) as i32,
        }
    }

    fn recheck_covered(&mut self) {
        let covered = self.auto_hide && occlusion::covered(self.hwnd, self.compact_screen_rect(0.0, 0.0));
        if covered == self.covered {
            return;
        }
        self.covered = covered;
        unsafe {
            if covered {
                SetTimer(self.hwnd, TIMER_DODGE, DODGE_MS, None);
            } else {
                let _ = KillTimer(self.hwnd, TIMER_DODGE);
            }
        }
        self.dodge = false;
        self.check_dodge();
        self.retarget();
    }

    /// Kursor berada di area notch tapi tidak di tepi paling atas layar:
    /// pengguna mengarah ke tab/jendela di belakang notch.
    fn cursor_wants_behind(&self) -> bool {
        let p = occlusion::cursor();
        let z = self.compact_screen_rect(DODGE_MARGIN_X, DODGE_MARGIN_Y);
        let inside = p.x >= z.left && p.x < z.right && p.y >= z.top && p.y < z.bottom;
        inside && p.y > z.top + TOP_EDGE_PX
    }

    /// Selama ada jendela di belakang notch: minggir saat kursor mengarah ke sana.
    fn check_dodge(&mut self) {
        let dodge = self.covered && !self.is_open() && self.cursor_wants_behind();
        if dodge != self.dodge {
            self.dodge = dodge;
            self.retarget();
        }
    }

    /// Mode minimal: notch tertutup hanya garis tipis, kecuali ada lirik atau notifikasi.
    fn as_line(&self) -> bool {
        let m = &self.modules[self.current];
        self.settings.borrow().minimal && !self.is_open() && !m.urgent() && !m.wants_space()
    }

    fn dots(&self) -> Vec<Color> {
        let mut d = Vec::new();
        if !self.in_use.mic.is_empty() {
            d.push(MIC);
        }
        if !self.in_use.cam.is_empty() {
            d.push(CAM);
        }
        d
    }

    fn dots_w(&self) -> f32 {
        let n = self.dots().len();
        if n == 0 { 0.0 } else { 6.0 + n as f32 * DOT_W }
    }

    fn compact_size(&self) -> (f32, f32) {
        let (w, h) = self.modules[self.current].compact_size();
        if self.as_line() {
            return (w, LINE_H);
        }
        (w + if self.badge().is_some() { BADGE_W } else { 0.0 } + self.dots_w(), h)
    }

    fn open_size(&self) -> (f32, f32) {
        let (w, h) = self.modules[self.page].expanded_size();
        // Cukup lebar supaya semua nama halaman muat.
        let tabs_w = self.tabs().len() as f32 * TAB_W + 24.0;
        (w.max(MIN_OPEN_W).max(tabs_w), h + TAB_H)
    }

    fn retarget(&mut self) {
        self.current = self.active_index();
        let (w, h) = if self.is_open() {
            self.open_size()
        } else if self.hidden() {
            // Naik keluar layar.
            (self.compact_size().0, 0.0)
        } else {
            self.compact_size()
        };
        self.w.set(w);
        self.h.set(h);
        if !self.animating {
            self.animating = true;
            self.last = Instant::now();
            unsafe { SetTimer(self.hwnd, TIMER_ANIM, 15, None) };
        }
    }

    /// Jadwalkan pengecekan pengingat berikutnya dari semua modul.
    fn schedule_poll(&self) {
        let next = self.modules.iter().filter_map(|m| m.poll_in()).min();
        unsafe {
            match next {
                Some(ms) => {
                    SetTimer(self.hwnd, TIMER_POLL, ms.max(50), None);
                }
                None => {
                    let _ = KillTimer(self.hwnd, TIMER_POLL);
                }
            }
        }
    }

    fn poll(&mut self) {
        let mut changed = false;
        for i in 0..self.modules.len() {
            match self.modules[i].poll() {
                Poll::Alert if self.modules[i].tab().is_some() => {
                    self.page = i;
                    self.alert = true;
                    unsafe { SetTimer(self.hwnd, TIMER_ALERT, ALERT_MS, None) };
                    changed = true;
                }
                Poll::Idle => {}
                _ => changed = true,
            }
        }
        self.schedule_poll();
        if changed {
            self.retarget();
            self.render();
        }
    }

    /// Letakkan kanvas di tengah atas monitor utama, sesuai DPI-nya.
    fn place(&mut self) {
        unsafe {
            let scale = GetDpiForWindow(self.hwnd) as f32 / 96.0;
            let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let pw = (CANVAS_W * scale).round() as i32;
            let ph = (CANVAS_H * scale).round() as i32;
            let m = mi.rcMonitor;
            self.origin = POINT { x: m.left + (m.right - m.left - pw) / 2, y: m.top };
            if dev_instance() {
                // `AMORA_DEV_X=<piksel>` menggeser instance uji ke tempat lain (layar sempit).
                let dx = std::env::var("AMORA_DEV_X").ok().and_then(|v| v.parse().ok());
                self.origin.x += dx.unwrap_or(pw * 3 / 4);
            }
            if pw != self.g.px_w || ph != self.g.px_h || scale != self.g.scale {
                let _ = self.g.resize(pw, ph, scale);
            }
        }
    }

    fn island(&self) -> Rect {
        let (w, h) = (self.w.value, self.h.value);
        Rect::new((CANVAS_W - w) / 2.0, 0.0, w, h)
    }

    fn content(&self) -> Rect {
        let i = self.island();
        Rect::new(i.x, i.y, i.w, (i.h - TAB_H).max(0.0))
    }

    fn render(&mut self) {
        if self.g.begin().is_err() {
            return;
        }
        let island = self.island();
        // Sudut atas menempel di tepi layar, hanya sudut bawah yang membulat.
        let radius = (island.h * 0.5).min(22.0);
        self.g.fill_rrect(Rect::new(island.x, -radius, island.w, island.h + radius), radius, BLACK);
        // Telinga cekung di kiri-kanan, supaya notch tampak tumbuh dari tepi layar.
        // Mengecil bersama notch (garis tipis, bersembunyi).
        let ear = (island.h * 0.35).min(EAR_R);
        if ear >= 1.0 {
            self.g.ear(island.x, ear, true, BLACK);
            self.g.ear(island.x + island.w, ear, false, BLACK);
        }

        let (_, ch) = self.compact_size();
        let (_, eh) = self.open_size();
        let p = ((island.h - ch) / (eh - ch).abs().max(1.0)).clamp(0.0, 1.0);
        let expanded = p >= 0.5;
        let badge = self.badge();
        self.g.push_clip(island);
        self.tab_hits.clear();
        if !expanded {
            // Memudar saat notch naik bersembunyi.
            // Memudar saat menyusut jadi garis atau naik bersembunyi.
            let full_h = self.modules[self.current].compact_size().1;
            self.g.alpha = (1.0 - p * 2.0) * ((island.h - LINE_H) / (full_h - LINE_H)).clamp(0.0, 1.0);
            let mut r = island;
            // Titik mic (oranye) dan kamera (hijau) di ujung kanan, seperti di iPhone.
            let dots = self.dots();
            if !dots.is_empty() {
                let dw = self.dots_w();
                r.w -= dw;
                for (k, c) in dots.iter().enumerate() {
                    let cx = r.x + r.w + 4.0 + DOT_W * (k as f32 + 0.5);
                    self.g.circle(cx, r.y + r.h / 2.0, 3.5, *c);
                }
            }
            if let Some((_, text, color)) = &badge {
                r.w -= BADGE_W;
                let br = Rect::new(r.x + r.w - 4.0, r.y, BADGE_W - 12.0, r.h);
                self.g.text(text, br, 12.5, 600, *color, Align::Right);
            }
            self.modules[self.current].draw_compact(&mut self.g, r);
        } else {
            self.g.alpha = (p - 0.5) * 2.0;
            let content = self.content();
            self.modules[self.page].draw_expanded(&mut self.g, content);
            self.draw_tabs(Rect::new(island.x, content.y + content.h, island.w, TAB_H));
        }
        self.g.alpha = 1.0;
        self.g.pop_clip();
        let _ = self.g.end(self.hwnd, self.origin);

        let tick = if expanded {
            self.modules[self.page].tick_ms(true)
        } else {
            let a = self.modules[self.current].tick_ms(false);
            let b = badge.and_then(|(i, _, _)| self.modules[i].tick_ms(false));
            a.into_iter().chain(b).min()
        };
        unsafe {
            match tick {
                Some(ms) => {
                    SetTimer(self.hwnd, TIMER_TICK, ms, None);
                }
                None => {
                    let _ = KillTimer(self.hwnd, TIMER_TICK);
                }
            }
        }
    }

    fn draw_tabs(&mut self, r: Rect) {
        let tabs = self.tabs();
        let w = TAB_W;
        let x0 = r.x + (r.w - w * tabs.len() as f32) / 2.0;
        for (k, &i) in tabs.iter().enumerate() {
            let tr = Rect::new(x0 + k as f32 * w, r.y, w, r.h - 6.0);
            let name = self.modules[i].tab().unwrap_or("");
            let (c, wt) = if i == self.page { (WHITE, 600) } else { (DIM, 400) };
            self.g.text(name, tr, 12.0, wt, c, Align::Center);
            self.tab_hits.push((tr, i));
        }
    }

    fn set_editing(&mut self, on: bool) {
        if on == self.editing {
            return;
        }
        self.editing = on;
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let na = WS_EX_NOACTIVATE.0 as isize;
            if on {
                // Jendela notch biasanya tidak pernah aktif; saat mengetik ia perlu fokus keyboard.
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex & !na);
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(self.hwnd);
            } else {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex | na);
            }
        }
    }

    fn shelf_index(&self) -> Option<usize> {
        self.modules.iter().position(|m| m.accepts_drop())
    }

    fn set_drop_hover(&mut self, on: bool) {
        let Some(i) = self.shelf_index() else { return };
        if on {
            self.page = i;
        }
        self.drop_hover = on;
        self.modules[i].drop_hover(on);
        self.retarget();
        self.render();
    }

    fn drop_files(&mut self, paths: Vec<PathBuf>) {
        let Some(i) = self.shelf_index() else { return };
        self.modules[i].drop_files(paths);
        self.set_drop_hover(false);
        // Kursor masih di atas notch: anggap hover supaya tetap terbuka sampai ditinggal.
        self.start_hover();
        self.retarget();
        self.render();
    }

    fn start_hover(&mut self) {
        self.hover = true;
        self.track_leave();
    }

    /// Minta WM_MOUSELEAVE saat kursor meninggalkan notch.
    fn track_leave(&self) {
        let mut t = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.hwnd,
            dwHoverTime: 0,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut t);
        }
    }

    fn broadcast(&mut self, e: Event) {
        for m in &mut self.modules {
            m.on_event(&e);
        }
        self.schedule_poll();
        self.retarget();
        self.render();
    }

    fn notice(&mut self, icon: &'static str, text: impl Into<String>, detail: impl Into<String>) {
        self.broadcast(Event::Notice { icon, text: text.into(), detail: detail.into() });
    }

    fn battery_detail(&self) -> String {
        self.battery.map(|b| format!("{b}%")).unwrap_or_default()
    }

    fn on_power(&mut self, p: system::Power) {
        match p {
            system::Power::Brightness(level) => {
                if self.brightness_ready {
                    self.broadcast(Event::Brightness(level));
                }
                self.brightness_ready = true;
            }
            system::Power::Charger(on) => {
                let changed = self.charger.is_some_and(|c| c != on);
                self.charger = Some(on);
                if on {
                    self.battery_warned = 101;
                }
                // Desktop tanpa baterai tidak perlu diberi tahu soal charger.
                if changed && self.battery.is_some_and(|b| b <= 100) {
                    let d = self.battery_detail();
                    if on {
                        self.notice("🔌", "Mengisi daya", d);
                    } else {
                        self.notice("🔋", "Charger dicabut", d);
                    }
                }
            }
            system::Power::Battery(level) => {
                let first = self.battery.is_none();
                self.battery = Some(level);
                if first || self.charger != Some(false) {
                    return;
                }
                if let Some(&th) = LOW_BATTERY.iter().filter(|&&t| level <= t && self.battery_warned > t).last() {
                    self.battery_warned = th;
                    modules::chime();
                    self.notice("🔋", "Baterai lemah", format!("{level}%"));
                }
            }
        }
    }

    fn on_network(&mut self) {
        let now = network::current();
        if now == self.net {
            return;
        }
        let was_online = self.net.online;
        self.net = now.clone();
        match (was_online, now.online) {
            (_, true) if !now.ssid.is_empty() => self.notice("📶", "Terhubung", now.ssid),
            (false, true) => self.notice("🌐", "Internet tersambung", ""),
            (true, false) => self.notice("⚠️", "Internet terputus", ""),
            _ => {}
        }
    }

    /// Terapkan setelan yang butuh tindakan di luar modul, lalu beri tahu semua modul.
    fn apply_settings(&mut self) {
        let (clip, auto_hide) = {
            let s = self.settings.borrow();
            // Clipboard dipantau untuk riwayat atau untuk merapikan tabel.
            (s.clipboard || s.tidy_tables, s.auto_hide)
        };
        if clip != self.clip_listening {
            clipboard::listen(self.hwnd, clip);
            self.clip_listening = clip;
        }
        if auto_hide != self.auto_hide {
            self.auto_hide = auto_hide;
            self.recheck_covered();
        }
        // Halaman yang sedang dibuka bisa hilang (mis. Clipboard dimatikan).
        if self.modules[self.page].tab().is_none() {
            self.page = self.tabs().first().copied().unwrap_or(0);
        }
        self.broadcast(Event::SettingsChanged);
    }

    fn sync_editing(&mut self) {
        let on = self.is_open() && self.modules[self.page].editing();
        self.set_editing(on);
    }

    fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            // Ada tab di belakang notch dan kursor datang dari bawah: minggir, jangan dibuka.
            // Untuk membuka notch, tempelkan kursor ke tepi paling atas layar.
            WM_MOUSEMOVE if !self.hover && self.auto_hide && self.covered && !self.is_open() && self.cursor_wants_behind() => {
                if !self.dodge {
                    self.dodge = true;
                    self.retarget();
                }
            }
            WM_MOUSEMOVE => {
                if self.is_open() && !self.hover {
                    // Sudah terbuka (pengingat, mengetik): langsung anggap hover.
                    self.hover_pending = false;
                    self.start_hover();
                    self.retarget();
                } else if !self.hover && !self.hover_pending {
                    self.hover_pending = true;
                    self.track_leave();
                    unsafe { SetTimer(self.hwnd, TIMER_HOVER, HOVER_DELAY_MS, None) };
                }
                // Tombol kiri ditahan dan digeser: mulai seret file keluar.
                if let Some((px, py)) = self.press {
                    let s = self.g.scale;
                    let x = (lp.0 & 0xffff) as i16 as f32 / s;
                    let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / s;
                    if wp.0 & 1 == 0 {
                        self.press = None;
                    } else if (x - px).hypot(y - py) > 6.0 {
                        self.press = None;
                        if self.is_open() && !self.editing {
                            self.pending_drag = self.modules[self.page].drag_source(px, py);
                        }
                    }
                }
            }
            WM_LBUTTONDOWN => {
                let s = self.g.scale;
                let x = (lp.0 & 0xffff) as i16 as f32 / s;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / s;
                self.press = Some((x, y));
            }
            WM_MOUSELEAVE => {
                if std::mem::take(&mut self.hover_pending) {
                    unsafe {
                        let _ = KillTimer(self.hwnd, TIMER_HOVER);
                    }
                }
                self.hover = false;
                self.alert = false;
                self.retarget();
            }
            WM_LBUTTONUP => {
                self.press = None;
                let s = self.g.scale;
                let x = (lp.0 & 0xffff) as i16 as f32 / s;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / s;
                if !self.is_open() {
                    return Some(LRESULT(0));
                }
                self.alert = false;
                if let Some(&(_, i)) = self.tab_hits.iter().find(|(r, _)| r.contains(x, y)) {
                    if self.editing {
                        self.modules[self.page].stop_editing();
                    }
                    self.page = i;
                } else {
                    let content = self.content();
                    if !self.modules[self.page].click(content, x, y) {
                        return Some(LRESULT(0));
                    }
                    if self.modules[self.page].tab() == Some("Setelan") {
                        self.apply_settings();
                    }
                }
                self.sync_editing();
                self.schedule_poll();
                // Klik bisa mengubah ukuran (mis. lirik dinyalakan, tugas ditambah).
                self.retarget();
                self.render();
            }
            WM_MOUSEWHEEL if self.is_open() && !self.editing => {
                let tabs = self.tabs();
                let k = tabs.iter().position(|&i| i == self.page).unwrap_or(0);
                let up = (wp.0 >> 16) as i16 > 0;
                let n = tabs.len();
                self.page = tabs[if up { (k + n - 1) % n } else { (k + 1) % n }];
                self.retarget();
                self.render();
            }
            WM_CHAR if self.editing => {
                if let Some(c) = char::from_u32(wp.0 as u32) {
                    self.modules[self.page].key_char(c);
                }
                self.sync_editing();
                self.schedule_poll();
                self.retarget();
                self.render();
            }
            WM_ACTIVATE if self.editing && (wp.0 & 0xffff) as u32 == WA_INACTIVE => {
                // Klik di luar notch: simpan ketikan.
                self.modules[self.page].stop_editing();
                self.set_editing(false);
                self.retarget();
                self.render();
            }
            WM_MOUSEACTIVATE if !self.editing => return Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_TIMER if wp.0 == TIMER_ANIM => {
                let now = Instant::now();
                let dt = (now - self.last).as_secs_f32();
                self.last = now;
                let a = self.w.step(dt);
                let b = self.h.step(dt);
                self.render();
                if !(a || b) {
                    self.animating = false;
                    unsafe {
                        let _ = KillTimer(self.hwnd, TIMER_ANIM);
                    }
                }
            }
            WM_TIMER if wp.0 == TIMER_TICK => self.render(),
            WM_TIMER if wp.0 == TIMER_POLL => self.poll(),
            WM_TIMER if wp.0 == occlusion::TIMER_RECHECK => {
                unsafe {
                    let _ = KillTimer(self.hwnd, occlusion::TIMER_RECHECK);
                }
                self.recheck_covered();
            }
            WM_TIMER if wp.0 == TIMER_DODGE => self.check_dodge(),
            WM_TIMER if wp.0 == TIMER_HOVER => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_HOVER);
                }
                // Masih di atas notch (WM_MOUSELEAVE belum datang) dan tidak sedang minggir.
                if std::mem::take(&mut self.hover_pending) && !self.hover && !self.dodge {
                    if !self.is_open() && self.modules[self.current].tab().is_some() {
                        // Buka di halaman yang sedang tampil di notch tertutup.
                        self.page = self.current;
                    }
                    self.start_hover();
                    self.retarget();
                }
            }
            WM_TIMER if wp.0 == TIMER_ALERT => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_ALERT);
                }
                self.alert = false;
                self.retarget();
            }
            WM_APP_MEDIA => self.broadcast(Event::MediaChanged),
            WM_APP_LYRICS => self.broadcast(Event::LyricsChanged),
            system::WM_APP_VOLUME => {
                self.broadcast(Event::Volume { level: wp.0 as u32, muted: lp.0 != 0 });
            }
            system::WM_APP_AUDIO_DEVICE => {
                let device = self.audio.as_mut().and_then(|a| a.rebind());
                if let Some((name, headphones)) = device {
                    let icon = if headphones { "🎧" } else { "🔊" };
                    self.notice(icon, name, "Output audio");
                }
            }
            WM_POWERBROADCAST => {
                if let Some(p) = system::power_from(wp, lp) {
                    self.on_power(p);
                }
                return Some(LRESULT(1));
            }
            privacy::WM_APP_PRIVACY => {
                let now = privacy::current();
                let new_mic: Vec<String> = now.mic.iter().filter(|a| !self.in_use.mic.contains(a)).cloned().collect();
                let new_cam: Vec<String> = now.cam.iter().filter(|a| !self.in_use.cam.contains(a)).cloned().collect();
                self.in_use = now;
                if let Some(app) = new_cam.first() {
                    self.notice("📷", "Kamera dipakai", app.clone());
                } else if let Some(app) = new_mic.first() {
                    self.notice("🎙️", "Mikrofon dipakai", app.clone());
                } else {
                    self.retarget();
                    self.render();
                }
            }
            WM_CLIPBOARDUPDATE => unsafe {
                SetTimer(self.hwnd, TIMER_CLIP, CLIP_SETTLE_MS, None);
            },
            WM_TIMER if wp.0 == TIMER_CLIP => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_CLIP);
                }
                let tidy = self.settings.borrow().tidy_tables;
                if tidy {
                    if let Some((rows, cols)) = clipboard::tidy_table(self.hwnd) {
                        self.notice("📋", "Tabel siap ditempel", format!("{rows} × {cols}"));
                    }
                }
                self.broadcast(Event::ClipboardChanged);
            }
            screenshots::WM_APP_SCREENSHOT => {
                let paths = screenshots::take();
                let on = self.settings.borrow().screenshot_to_shelf;
                if let (true, Some(i)) = (on && !paths.is_empty(), self.shelf_index()) {
                    self.modules[i].drop_files(paths);
                    self.notice("📸", "Screenshot", "masuk Shelf");
                }
            }
            keyboard::WM_APP_CAPS => {
                self.notice("⇪", "Caps Lock", if wp.0 != 0 { "Nyala" } else { "Mati" });
            }
            keyboard::WM_APP_LANG => {
                let (name, code) = keyboard::language(lp.0);
                self.notice("🌐", name, code);
            }
            network::WM_APP_NET => unsafe {
                SetTimer(self.hwnd, TIMER_NET, NET_SETTLE_MS, None);
            },
            WM_TIMER if wp.0 == TIMER_NET => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_NET);
                }
                self.on_network();
            }
            WM_DPICHANGED | WM_DISPLAYCHANGE => {
                self.place();
                // Posisi notch di layar berubah: cek ulang jendela di belakangnya.
                self.recheck_covered();
                self.retarget();
                self.render();
            }
            shelf::WM_APP_THUMB if self.is_open() => self.render(),
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

/// Untuk pengembangan: `AMORA_FORCE_OPEN=1` membuat notch selalu terbuka.
fn dev_instance() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AMORA_DEV").is_some())
}

/// Dibaca sekali saja: `is_open()` dipanggil berkali-kali di setiap frame.
fn force_open() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AMORA_FORCE_OPEN").is_some())
}

const CMD_EXIT: usize = 1;
const CMD_AUTOSTART: usize = 2;
const CMD_ABOUT: usize = 3;
const CMD_MASCOT: usize = 4;
const CMD_MINIMAL: usize = 5;
const CMD_CLIPBOARD: usize = 6;

/// Menu yang sama untuk klik kanan notch dan klik ikon tray.
fn show_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let auto = autostart::is_enabled();
        let (mut mascot, mut minimal, mut clip) = (false, false, false);
        with_app(|a| {
            let s = a.settings.borrow();
            (mascot, minimal, clip) = (s.mascot, s.minimal, s.clipboard);
        });
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | check(auto), CMD_AUTOSTART, w!("Mulai bersama Windows"));
        let _ = AppendMenuW(menu, MF_STRING | check(mascot), CMD_MASCOT, w!("Maskot Amora"));
        let _ = AppendMenuW(menu, MF_STRING | check(minimal), CMD_MINIMAL, w!("Notch minimal"));
        let _ = AppendMenuW(menu, MF_STRING | check(clip), CMD_CLIPBOARD, w!("Riwayat clipboard"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, CMD_ABOUT, w!("Tentang Amora"));
        let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, w!("Keluar"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, None);
        let _ = DestroyMenu(menu);
        // Supaya menu tray menutup dengan benar saat klik di luar.
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        match cmd.0 as usize {
            CMD_EXIT => {
                let _ = DestroyWindow(hwnd);
            }
            CMD_AUTOSTART => autostart::set(!auto),
            CMD_MASCOT | CMD_MINIMAL | CMD_CLIPBOARD => with_app(|a| {
                {
                    let mut s = a.settings.borrow_mut();
                    match cmd.0 as usize {
                        CMD_MASCOT => s.mascot = !s.mascot,
                        CMD_MINIMAL => s.minimal = !s.minimal,
                        _ => s.clipboard = !s.clipboard,
                    }
                    s.save();
                }
                a.apply_settings();
                a.schedule_poll();
                a.retarget();
                a.render();
            }),
            CMD_ABOUT => {
                let text = HSTRING::from(format!(
                    "Amora {}
Notch hidup di tepi atas layar Windows.

Lirik dari LRCLIB (lrclib.net). Hanya judul, artis, dan durasi lagu yang dikirim.",
                    env!("CARGO_PKG_VERSION")
                ));
                MessageBoxW(hwnd, &text, w!("Tentang Amora"), MB_OK | MB_ICONINFORMATION);
            }
            _ => {}
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // Menu berjalan dengan loop pesannya sendiri, jadi jangan sambil memegang APP.
        WM_RBUTTONUP => {
            show_menu(hwnd);
            return LRESULT(0);
        }
        tray::WM_TRAY => {
            if tray::wants_menu(lp) {
                show_menu(hwnd);
            }
            return LRESULT(0);
        }
        WM_DESTROY => {
            tray::remove(hwnd);
            unsafe {
                let _ = RevokeDragDrop(hwnd);
            }
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        // Explorer restart: ikon tray harus didaftarkan ulang.
        m if tray::is_taskbar_created(m) => {
            tray::add(hwnd);
            return LRESULT(0);
        }
        _ => {}
    }
    let r = APP.with(|a| {
        let mut a = a.try_borrow_mut().ok()?;
        a.as_mut()?.handle(msg, wp, lp)
    });
    // Seret keluar menjalankan loop pesannya sendiri, jadi dilakukan setelah APP dilepas.
    let mut drag = None;
    with_app(|a| drag = a.pending_drag.take());
    if let Some(path) = drag {
        SELF_DRAG.with(|d| d.set(true));
        let dropped = shelf::drag_out(hwnd, &path);
        SELF_DRAG.with(|d| d.set(false));
        if dropped {
            with_app(|a| {
                let page = a.page;
                a.modules[page].dragged_out(&path);
                a.retarget();
                a.render();
            });
        }
    }
    r.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wp, lp) })
}

/// Menerima file yang diseret dari Explorer ke notch.
#[implement(IDropTarget)]
struct DropTarget;

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        data: Option<&IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let ok = !SELF_DRAG.with(|d| d.get()) && data.is_some_and(shelf::has_files);
        unsafe { *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        if ok {
            with_app(|a| a.set_drop_hover(true));
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let mut ok = false;
        with_app(|a| ok = a.drop_hover);
        unsafe { *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        with_app(|a| a.set_drop_hover(false));
        Ok(())
    }

    fn Drop(
        &self,
        data: Option<&IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let paths = data.map(shelf::files_from).unwrap_or_default();
        unsafe { *effect = if paths.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_COPY } };
        with_app(|a| {
            if paths.is_empty() {
                a.set_drop_hover(false);
            } else {
                a.drop_files(paths);
            }
        });
        Ok(())
    }
}

fn main() -> Result<()> {
    crashlog::install();
    // Untuk pengembangan: `cargo run -- --probe-lyrics "<judul>" "<artis>" <durasi>`
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--probe-lyrics") {
        return lyrics::probe(&args[2], &args[3], args[4].parse().unwrap_or(0.0));
    }
    // Untuk pengembangan: `amora --probe-table <file CF_HTML atau teks>` menampilkan hasil urai.
    if args.get(1).map(String::as_str) == Some("--probe-table") {
        let bytes = std::fs::read(&args[2]).unwrap_or_default();
        let rows = tables::cf_html_fragment(&bytes)
            .and_then(|f| tables::from_html(&f))
            .or_else(|| tables::from_markdown(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}')));
        match rows {
            Some(r) => {
                for row in &r {
                    println!("{} {:?}", if row.head { "J" } else { " " }, row.cells);
                }
                println!("lebar kolom %: {:?}", tables::column_widths(&r));
            }
            None => println!("bukan tabel"),
        }
        return Ok(());
    }
    // Untuk pengembangan: `amora --crash-test` memastikan panic tercatat di log.txt.
    if args.get(1).map(String::as_str) == Some("--crash-test") {
        panic!("uji log crash");
    }
    // Dipanggil dari Settings → Apps (lihat installer).
    if args.get(1).map(String::as_str) == Some("--uninstall") {
        let has = |f: &str| args.iter().any(|a| a == f);
        uninstall::run(has("--quiet"), has("--wipe"));
        return Ok(());
    }
    // Untuk pengembangan: `amora --probe-system` mencetak status yang dipakai notifikasi.
    if args.get(1).map(String::as_str) == Some("--probe-system") {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        println!("jaringan: {:?}", network::current());
        println!("output audio: {:?}", system::default_output());
        println!("bahasa keyboard: {:?}", keyboard::language(keyboard::current_layout()));
        return Ok(());
    }
    // Untuk pengembangan: `amora --autostart on|off|status`
    if args.get(1).map(String::as_str) == Some("--autostart") {
        match args.get(2).map(String::as_str) {
            Some("on") => autostart::set(true),
            Some("off") => autostart::set(false),
            _ => {}
        }
        println!("autostart: {}", autostart::is_enabled());
        return Ok(());
    }
    unsafe {
        // Satu instance saja.
        // `AMORA_DEV=1`: instance uji yang boleh jalan berdampingan (digeser ke kanan).
        let _mutex = CreateMutexW(None, true, w!("Local\\AmoraNotch"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS && !dev_instance() {
            return Ok(());
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // OLE (bukan COM biasa) supaya drag & drop file bisa dipakai.
        OleInitialize(None)?;

        let hinst = GetModuleHandleW(None)?;
        let class = w!("AmoraNotch");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Amora"),
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
        let g = Gfx::new((CANVAS_W * scale) as i32, (CANVAS_H * scale) as i32, scale)?;
        let (shared, handle) = media::spawn(hwnd, WM_APP_MEDIA);
        let lyrics = lyrics::LyricsService::spawn(hwnd, WM_APP_LYRICS);
        let settings = Settings::load_shared();
        // Urutan = prioritas untuk notch tertutup, dan urutan halaman.
        let modules: Vec<Box<dyn Module>> = vec![
            Box::new(Osd::new()),
            Box::new(MediaModule::new(shared, handle, g.wic.clone(), lyrics, settings.clone())),
            Box::new(Pomodoro::new(settings.clone())),
            Box::new(Water::new(settings.clone())),
            Box::new(Todo::new()),
            Box::new(Schedule::new()),
            Box::new(Shelf::new(g.wic.clone(), hwnd)),
            Box::new(Clipboard::new(hwnd, settings.clone())),
            Box::new(SettingsPage::new(settings.clone())),
            Box::new(ClockModule::new(settings.clone())),
        ];

        let mut app = App {
            hwnd,
            g,
            origin: POINT::default(),
            modules,
            current: 0,
            page: 0,
            hover: false,
            hover_pending: false,
            alert: false,
            editing: false,
            tab_hits: Vec::new(),
            drop_hover: false,
            press: None,
            pending_drag: None,
            audio: system::AudioWatch::new(hwnd),
            brightness_ready: false,
            charger: None,
            battery: None,
            battery_warned: 101,
            net: network::current(),
            settings: settings.clone(),
            clip_listening: settings.borrow().clipboard || settings.borrow().tidy_tables,
            in_use: privacy::InUse::default(),
            auto_hide: settings.borrow().auto_hide,
            covered: false,
            dodge: false,
            w: Spring::new(0.0),
            h: Spring::new(0.0),
            last: Instant::now(),
            animating: false,
        };
        app.current = app.active_index();
        if let Ok(name) = std::env::var("AMORA_PAGE") {
            if let Some(i) = app.modules.iter().position(|m| m.tab() == Some(name.as_str())) {
                app.page = i;
            }
        }
        let (w, h) = if app.is_open() { app.open_size() } else { app.compact_size() };
        app.w = Spring::new(w);
        app.h = Spring::new(h);
        app.schedule_poll();
        app.place();
        app.render();
        APP.with(|a| *a.borrow_mut() = Some(app));
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        tray::add(hwnd);
        system::watch_power(hwnd);
        keyboard::spawn(hwnd);
        network::watch(hwnd);
        privacy::watch(hwnd);
        with_app(|a| a.in_use = privacy::current());
        screenshots::watch(hwnd);
        {
            let s = settings.borrow();
            clipboard::listen(hwnd, s.clipboard || s.tidy_tables);
        }
        occlusion::install(hwnd);
        with_app(|a| a.recheck_covered());
        let target: IDropTarget = DropTarget.into();
        let _ = RegisterDragDrop(hwnd, &target);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        APP.with(|a| a.borrow_mut().take());
    }
    Ok(())
}
