//! Pop-up earbud di notch tertutup: muncul sebentar saat case earbud dibuka dekat laptop,
//! lengkap dengan baterai (dari Windows) bila terbaca. Deteksinya ada di `crate::earbuds`.

use std::time::{Duration, Instant};

use super::{Event, Module, Poll};
use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::settings::SharedSettings;

const SHOW: Duration = Duration::from_millis(5000);
const GOOD: Color = Color(0.35, 0.85, 0.55, 1.0);
const LOW: Color = Color(1.0, 0.42, 0.42, 1.0);
/// Di bawah ini bar baterai berwarna merah.
const LOW_PERCENT: u32 = 20;

pub struct EarbudsPopup {
    settings: SharedSettings,
    percent: Option<u32>,
    until: Option<Instant>,
}

impl EarbudsPopup {
    pub fn new(settings: SharedSettings) -> Self {
        Self { settings, percent: None, until: None }
    }
}

impl Module for EarbudsPopup {
    fn active(&self) -> bool {
        self.until.is_some_and(|t| Instant::now() < t)
    }

    fn urgent(&self) -> bool {
        self.active()
    }

    fn compact_size(&self) -> (f32, f32) {
        (320.0, 32.0)
    }

    fn expanded_size(&self) -> (f32, f32) {
        self.compact_size()
    }

    fn draw_compact(&mut self, g: &mut Gfx, r: Rect) {
        let label = self.settings.borrow().earbuds.label.clone();
        let cy = r.y + r.h / 2.0;
        g.text("🎧", Rect::new(r.x + 10.0, r.y, 26.0, r.h), 14.0, 400, WHITE, Align::Center);
        g.text(&label, Rect::new(r.x + 42.0, r.y, r.w - 42.0 - 130.0, r.h), 12.5, 600, WHITE, Align::Left);

        match self.percent {
            Some(p) => {
                let text = Rect::new(r.x + r.w - 14.0 - 44.0, r.y, 44.0, r.h);
                g.text(&format!("{p}%"), text, 12.5, 600, WHITE, Align::Right);
                let bar = Rect::new(text.x - 8.0 - 46.0, cy - 3.0, 46.0, 6.0);
                g.fill_rrect(bar, 3.0, FAINT);
                let k = p.min(100) as f32 / 100.0;
                g.fill_rrect(Rect::new(bar.x, bar.y, bar.w * k, bar.h), 3.0, if p <= LOW_PERCENT { LOW } else { GOOD });
            }
            None => {
                g.text("Case terbuka", Rect::new(r.x + r.w - 14.0 - 110.0, r.y, 110.0, r.h), 12.0, 400, DIM, Align::Right);
            }
        }
    }

    fn draw_expanded(&mut self, _g: &mut Gfx, _r: Rect) {}

    fn on_event(&mut self, e: &Event) {
        let Event::Earbuds { opened, percent } = e else { return };
        if let Some(p) = percent {
            self.percent = Some(*p);
        }
        // Angka baterai yang datang belakangan hanya memperbarui isi, tidak memunculkan pop-up baru.
        if *opened {
            self.until = Some(Instant::now() + SHOW);
        }
    }

    fn poll(&mut self) -> Poll {
        // Waktunya habis: kembalikan notch ke isi sebelumnya.
        if self.until.is_some_and(|t| Instant::now() >= t) {
            self.until = None;
            return Poll::Redraw;
        }
        Poll::Idle
    }

    fn poll_in(&self) -> Option<u32> {
        self.until
            .filter(|t| Instant::now() < *t)
            .map(|t| t.saturating_duration_since(Instant::now()).as_millis() as u32 + 10)
    }
}
