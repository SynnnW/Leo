//! Jam: tampil saat tidak ada yang lain. Saat PC lama tidak disentuh, jam minggir dan
//! sepasang mata (maskot) muncul, berkedip, lalu tertidur. Saat pengguna kembali, mata
//! menyapa sebentar lalu jam kembali.

use std::time::{Duration, Instant};

use windows::Win32::System::SystemInformation::{GetLocalTime, GetTickCount};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

use super::{Module, Poll};
use crate::gfx::{Align, Color, Gfx, Rect, DIM, WHITE};
use crate::settings::SharedSettings;

const HARI: [&str; 7] = ["Minggu", "Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu"];
const BULAN: [&str; 12] = [
    "Januari", "Februari", "Maret", "April", "Mei", "Juni", "Juli", "Agustus", "September",
    "Oktober", "November", "Desember",
];

/// Diam selama ini → mata muncul.
const IDLE_AFTER: Duration = Duration::from_secs(60);
/// Mata terbuka selama ini → tertidur.
const SLEEP_AFTER: Duration = Duration::from_secs(120);
const HELLO: Duration = Duration::from_millis(1600);
/// Selama mata tampil, cek apakah pengguna kembali.
const RETURN_CHECK_MS: u32 = 400;
const BLINK_EVERY_MS: u128 = 4200;
const BLINK_MS: u128 = 180;

enum Mascot {
    /// Jam biasa.
    Off,
    Eyes { since: Instant },
    Asleep,
    Hello { until: Instant },
}

pub struct ClockModule {
    settings: SharedSettings,
    mascot: Mascot,
}

/// Lama tidak ada input keyboard/mouse.
fn idle() -> Duration {
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if !GetLastInputInfo(&mut li).as_bool() {
            return Duration::ZERO;
        }
        Duration::from_millis(GetTickCount().wrapping_sub(li.dwTime) as u64)
    }
}

fn chevron(g: &Gfx, x: f32, y: f32, c: Color) {
    g.polygon(&[(x - 6.0, y + 3.0), (x, y - 3.0), (x + 6.0, y + 3.0), (x + 4.2, y + 4.6), (x, y + 0.8), (x - 4.2, y + 4.6)], c);
}

impl ClockModule {
    pub fn new(settings: SharedSettings) -> Self {
        // Untuk pengembangan: `AMORA_MASCOT=mata|tidur|halo` memaksa keadaan maskot.
        let mascot = match std::env::var("AMORA_MASCOT").as_deref() {
            Ok("mata") => Mascot::Eyes { since: Instant::now() },
            Ok("tidur") => Mascot::Asleep,
            Ok("halo") => Mascot::Hello { until: Instant::now() + Duration::from_secs(3600) },
            _ => Mascot::Off,
        };
        Self { settings, mascot }
    }

    fn enabled(&self) -> bool {
        self.settings.borrow().mascot
    }

    fn draw_eyes(&self, g: &mut Gfx, r: Rect) {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        for dx in [-13.0, 13.0] {
            let x = cx + dx;
            match &self.mascot {
                Mascot::Eyes { since } => {
                    let phase = since.elapsed().as_millis() % BLINK_EVERY_MS;
                    let open = if phase < BLINK_MS {
                        ((phase as f32 - BLINK_MS as f32 / 2.0).abs() / (BLINK_MS as f32 / 2.0)).max(0.15)
                    } else {
                        1.0
                    };
                    let h = 13.0 * open;
                    g.fill_rrect(Rect::new(x - 4.5, cy - h / 2.0, 9.0, h), 4.5_f32.min(h / 2.0), WHITE);
                }
                Mascot::Asleep => g.fill_rrect(Rect::new(x - 5.0, cy + 1.0, 10.0, 2.5), 1.2, DIM),
                Mascot::Hello { .. } => chevron(g, x, cy, WHITE),
                Mascot::Off => {}
            }
        }
        if matches!(self.mascot, Mascot::Asleep) {
            g.text("z", Rect::new(cx + 26.0, r.y + 2.0, 12.0, r.h / 2.0), 10.0, 600, DIM, Align::Center);
        }
    }
}

impl Module for ClockModule {
    fn active(&self) -> bool {
        true
    }

    fn compact_size(&self) -> (f32, f32) {
        (180.0, 32.0)
    }

    fn expanded_size(&self) -> (f32, f32) {
        (280.0, 92.0)
    }

    fn draw_compact(&mut self, g: &mut Gfx, r: Rect) {
        if !matches!(self.mascot, Mascot::Off) && self.enabled() {
            self.draw_eyes(g, r);
            return;
        }
        let t = unsafe { GetLocalTime() };
        let s = format!("{:02}:{:02}", t.wHour, t.wMinute);
        g.text(&s, r, 13.0, 600, WHITE, Align::Center);
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        let t = unsafe { GetLocalTime() };
        let time = format!("{:02}:{:02}", t.wHour, t.wMinute);
        let date = format!(
            "{}, {} {} {}",
            HARI[t.wDayOfWeek as usize % 7],
            t.wDay,
            BULAN[(t.wMonth as usize).saturating_sub(1) % 12],
            t.wYear
        );
        g.text(&time, Rect::new(r.x, r.y + 12.0, r.w, 40.0), 30.0, 600, WHITE, Align::Center);
        g.text(&date, Rect::new(r.x, r.y + 54.0, r.w, 22.0), 13.0, 400, DIM, Align::Center);
    }

    fn tick_ms(&self, expanded: bool) -> Option<u32> {
        if !expanded && self.enabled() {
            if let Mascot::Eyes { since } = &self.mascot {
                // Gambar hanya selama kedipan; di luar itu tidur sampai kedipan berikutnya.
                let phase = since.elapsed().as_millis() % BLINK_EVERY_MS;
                return Some(if phase < BLINK_MS { 16 } else { (BLINK_EVERY_MS - phase) as u32 });
            }
            if !matches!(self.mascot, Mascot::Off) {
                return None;
            }
        }
        // Bangun tepat saat menit berganti, bukan tiap detik.
        let t = unsafe { GetLocalTime() };
        Some(60_000 - (t.wSecond as u32 * 1000 + t.wMilliseconds as u32) + 20)
    }

    fn poll_in(&self) -> Option<u32> {
        if !self.enabled() {
            return None;
        }
        Some(match &self.mascot {
            Mascot::Off => IDLE_AFTER.saturating_sub(idle()).as_millis() as u32 + 50,
            Mascot::Hello { until } => until.saturating_duration_since(Instant::now()).as_millis() as u32 + 10,
            _ => RETURN_CHECK_MS,
        })
    }

    fn poll(&mut self) -> Poll {
        if std::env::var_os("AMORA_MASCOT").is_some() {
            return Poll::Idle;
        }
        if !self.enabled() {
            let was_on = !matches!(self.mascot, Mascot::Off);
            self.mascot = Mascot::Off;
            return if was_on { Poll::Redraw } else { Poll::Idle };
        }
        let before = std::mem::discriminant(&self.mascot);
        let back = idle() < Duration::from_secs(1);
        self.mascot = match std::mem::replace(&mut self.mascot, Mascot::Off) {
            Mascot::Off if idle() >= IDLE_AFTER => Mascot::Eyes { since: Instant::now() },
            Mascot::Eyes { .. } | Mascot::Asleep if back => Mascot::Hello { until: Instant::now() + HELLO },
            Mascot::Eyes { since } if since.elapsed() >= SLEEP_AFTER => Mascot::Asleep,
            Mascot::Hello { until } if Instant::now() < until => Mascot::Hello { until },
            Mascot::Hello { .. } => Mascot::Off,
            other => other,
        };
        // Cek "sudah kembali?" berjalan beberapa kali per detik; gambar ulang hanya bila berubah.
        if std::mem::discriminant(&self.mascot) == before { Poll::Idle } else { Poll::Redraw }
    }
}
