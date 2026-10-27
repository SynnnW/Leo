//! Tugas & makrab dari Supabase (database website): pengingat deadline di notch tertutup dan
//! halaman "Tugas" berisi progres makrab serta deadline terdekat. Datanya diambil di latar
//! belakang oleh `crate::supabase` (login lewat Pengaturan -> Akun).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{Event, Module, Poll};
use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::supabase::{self, Urgency};

const GOOD: Color = Color(0.35, 0.85, 0.55, 1.0);
const RED: Color = Color(1.0, 0.42, 0.42, 1.0);
const ORANGE: Color = Color(1.0, 0.70, 0.30, 1.0);
const YELLOW: Color = Color(1.0, 0.90, 0.45, 1.0);
const SHOW: Duration = Duration::from_secs(6);
const ROWS: usize = 6;
const ROW_H: f32 = 24.0;

fn urgency_color(u: Urgency) -> Color {
    match u {
        Urgency::Overdue => RED,
        Urgency::Soon => ORANGE,
        Urgency::Today => YELLOW,
        Urgency::Later => DIM,
    }
}

/// Pengingat deadline di notch tertutup. Muncul beberapa detik saat sebuah tugas memasuki
/// 24 jam, 3 jam, 1 jam, atau terlambat; tiap tingkat hanya sekali per tugas.
pub struct DeadlineAlert {
    seen: HashMap<String, u8>,
    shown: Option<(String, String, Urgency)>,
    until: Option<Instant>,
}

impl DeadlineAlert {
    pub fn new() -> Self {
        Self { seen: HashMap::new(), shown: None, until: None }
    }

    /// Periksa data terbaru; `true` bila ada pengingat baru yang harus ditampilkan.
    fn evaluate(&mut self) -> bool {
        let Some(snap) = supabase::snapshot() else { return false };
        let now = supabase::now_unix();
        let alerts = supabase::new_alerts(&snap.tasks, now, &mut self.seen);
        // Bila beberapa sekaligus, tampilkan yang paling mendesak (tingkat tertinggi, lalu paling awal).
        let Some(&(i, _)) = alerts.iter().max_by_key(|&&(i, level)| (level, std::cmp::Reverse(i))) else { return false };
        let t = &snap.tasks[i];
        let (when, urgency) = supabase::relative(t.deadline, now);
        self.shown = Some((t.judul.clone(), when, urgency));
        self.until = Some(Instant::now() + SHOW);
        true
    }
}

impl Module for DeadlineAlert {
    fn active(&self) -> bool {
        self.until.is_some_and(|t| Instant::now() < t)
    }

    fn urgent(&self) -> bool {
        self.active()
    }

    fn compact_size(&self) -> (f32, f32) {
        (340.0, 32.0)
    }

    fn expanded_size(&self) -> (f32, f32) {
        self.compact_size()
    }

    fn draw_compact(&mut self, g: &mut Gfx, r: Rect) {
        let Some((title, when, urgency)) = &self.shown else { return };
        g.text("⏰", Rect::new(r.x + 10.0, r.y, 26.0, r.h), 14.0, 400, WHITE, Align::Center);
        g.text(title, Rect::new(r.x + 42.0, r.y, r.w - 42.0 - 100.0, r.h), 12.5, 600, WHITE, Align::Left);
        g.text(when, Rect::new(r.x + r.w - 14.0 - 90.0, r.y, 90.0, r.h), 12.0, 600, urgency_color(*urgency), Align::Right);
    }

    fn draw_expanded(&mut self, _g: &mut Gfx, _r: Rect) {}

    fn on_event(&mut self, e: &Event) {
        if matches!(e, Event::Makrab) {
            self.evaluate();
        }
    }

    fn poll(&mut self) -> Poll {
        let expired = self.until.is_some_and(|t| Instant::now() >= t);
        if expired {
            self.until = None;
        }
        // Waktu terus berjalan: tugas bisa masuk tingkat peringatan baru di antara dua pembaruan data.
        if self.evaluate() || expired {
            Poll::Redraw
        } else {
            Poll::Idle
        }
    }

    fn poll_in(&self) -> Option<u32> {
        let until_hide = self
            .until
            .filter(|t| Instant::now() < *t)
            .map(|t| t.saturating_duration_since(Instant::now()).as_millis() as u32 + 10);
        match until_hide {
            Some(ms) => Some(ms.min(60_000)),
            // Tanpa data (belum masuk / fitur tidak dipakai) tidak perlu bangun tiap menit: CPU tetap 0.
            None => supabase::snapshot().map(|_| 60_000),
        }
    }
}

/// Halaman "Tugas": progres makrab dan deadline terdekat.
pub struct DeadlinePage;

impl DeadlinePage {
    pub fn new() -> Self {
        Self
    }
}

impl Module for DeadlinePage {
    fn tab(&self) -> Option<&'static str> {
        Some("Tugas")
    }

    fn expanded_size(&self) -> (f32, f32) {
        (480.0, 44.0 + 30.0 + ROWS as f32 * ROW_H + 14.0)
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        g.text("Tugas & Makrab", Rect::new(r.x + 20.0, r.y + 10.0, 240.0, 24.0), 15.0, 600, WHITE, Align::Left);
        let note = |g: &mut Gfx, msg: &str| {
            g.text_wrapped(msg, Rect::new(r.x + 20.0, r.y + 48.0, r.w - 40.0, 80.0), 12.5, 400, DIM, Align::Left);
        };

        let Some(snap) = supabase::snapshot() else {
            let err = supabase::last_error();
            let msg = if !err.is_empty() {
                format!("Gagal memuat: {err}")
            } else {
                "Belum ada data. Buka menu tray → Pengaturan → Akun: isi anon_key (sekali) lalu masuk dengan Google.".to_string()
            };
            note(g, &msg);
            return;
        };

        let ago = ((supabase::now_unix() - snap.fetched_at) / 60).max(0);
        let info = if ago == 0 { format!("{} · baru saja", snap.email) } else { format!("{} · {ago} mnt lalu", snap.email) };
        g.text(&info, Rect::new(r.x + r.w - 20.0 - 260.0, r.y + 12.0, 260.0, 20.0), 11.0, 400, DIM, Align::Right);

        // Progres makrab.
        let top = r.y + 40.0;
        let label = if snap.makrab_total == 0 {
            "Makrab: belum ada tugas".to_string()
        } else {
            format!("Makrab {}/{}", snap.makrab_done, snap.makrab_total)
        };
        g.text(&label, Rect::new(r.x + 20.0, top, 150.0, 22.0), 12.5, 600, WHITE, Align::Left);
        let bar = Rect::new(r.x + 175.0, top + 8.0, 150.0, 6.0);
        g.fill_rrect(bar, 3.0, FAINT);
        if snap.makrab_total > 0 {
            let k = snap.makrab_done as f32 / snap.makrab_total as f32;
            g.fill_rrect(Rect::new(bar.x, bar.y, bar.w * k, bar.h), 3.0, GOOD);
        }
        if snap.review_pending > 0 {
            let text = format!("{} menunggu review", snap.review_pending);
            g.text(&text, Rect::new(bar.x + bar.w + 12.0, top, r.x + r.w - 20.0 - (bar.x + bar.w + 12.0), 22.0), 12.0, 600, ORANGE, Align::Left);
        }

        // Deadline terdekat.
        let now = supabase::now_unix();
        if snap.tasks.is_empty() {
            note(g, "Tidak ada tugas berdeadline yang belum selesai.");
            return;
        }
        for (i, t) in snap.tasks.iter().take(ROWS).enumerate() {
            let y = top + 32.0 + i as f32 * ROW_H;
            let (when, urgency) = supabase::relative(t.deadline, now);
            g.circle(r.x + 26.0, y + ROW_H / 2.0, 3.5, urgency_color(urgency));
            g.text(&t.judul, Rect::new(r.x + 40.0, y, r.w - 40.0 - 20.0 - 110.0, ROW_H), 12.5, 400, WHITE, Align::Left);
            g.text(&when, Rect::new(r.x + r.w - 20.0 - 100.0, y, 100.0, ROW_H), 12.0, 600, urgency_color(urgency), Align::Right);
        }
        let more = snap.tasks.len().saturating_sub(ROWS);
        if more > 0 {
            let y = top + 32.0 + ROWS as f32 * ROW_H;
            g.text(&format!("+{more} lainnya"), Rect::new(r.x + 40.0, y, 200.0, 18.0), 11.0, 400, DIM, Align::Left);
        }
    }
}
