//! OSD dan notifikasi singkat di notch tertutup: volume, kecerahan, Caps Lock,
//! bahasa keyboard, charger/baterai, jaringan, dan perangkat audio.

use std::time::{Duration, Instant};

use super::{Event, Module, Poll};
use crate::gfx::{Align, Gfx, Rect, DIM, FAINT, WHITE};

const SHOW_LEVEL: Duration = Duration::from_millis(1600);
const SHOW_NOTICE: Duration = Duration::from_millis(2600);

#[derive(Clone, PartialEq)]
enum Kind {
    Volume,
    Brightness,
    Notice { icon: &'static str, text: String, detail: String },
}

pub struct Osd {
    kind: Kind,
    level: u32,
    muted: bool,
    until: Option<Instant>,
}

impl Osd {
    pub fn new() -> Self {
        Self { kind: Kind::Volume, level: 0, muted: false, until: None }
    }

    fn glyph(&self) -> char {
        // Segoe MDL2 Assets
        match self.kind {
            Kind::Brightness => '\u{E706}',
            _ if self.muted || self.level == 0 => '\u{E74F}',
            _ if self.level < 34 => '\u{E993}',
            _ if self.level < 67 => '\u{E994}',
            _ => '\u{E995}',
        }
    }
}

impl Module for Osd {
    fn active(&self) -> bool {
        self.until.is_some_and(|t| Instant::now() < t)
    }

    fn urgent(&self) -> bool {
        self.active()
    }

    fn compact_size(&self) -> (f32, f32) {
        match self.kind {
            Kind::Notice { .. } => (320.0, 32.0),
            _ => (280.0, 32.0),
        }
    }

    fn expanded_size(&self) -> (f32, f32) {
        self.compact_size()
    }

    fn draw_compact(&mut self, g: &mut Gfx, r: Rect) {
        if let Kind::Notice { icon, text, detail } = &self.kind {
            g.text(icon, Rect::new(r.x + 10.0, r.y, 26.0, r.h), 14.0, 400, WHITE, Align::Center);
            g.text(text, Rect::new(r.x + 42.0, r.y, r.w - 42.0 - 90.0, r.h), 12.5, 600, WHITE, Align::Left);
            g.text(detail, Rect::new(r.x + r.w - 104.0, r.y, 90.0, r.h), 12.0, 400, DIM, Align::Right);
            return;
        }
        let cy = r.y + r.h / 2.0;
        let dim = self.kind == Kind::Volume && self.muted;
        g.icon(self.glyph(), Rect::new(r.x + 10.0, r.y, 26.0, r.h), 14.0, if dim { DIM } else { WHITE });
        let bar = Rect::new(r.x + 44.0, cy - 2.5, r.w - 44.0 - 52.0, 5.0);
        g.fill_rrect(bar, 2.5, FAINT);
        let k = self.level.min(100) as f32 / 100.0;
        g.fill_rrect(Rect::new(bar.x, bar.y, bar.w * k, bar.h), 2.5, if dim { DIM } else { WHITE });
        let label = if dim { "Mute".to_string() } else { self.level.to_string() };
        g.text(&label, Rect::new(r.x, r.y, r.w - 14.0, r.h), 12.5, 600, WHITE, Align::Right);
    }

    fn draw_expanded(&mut self, _g: &mut Gfx, _r: Rect) {}

    fn on_event(&mut self, e: &Event) {
        let show = match e {
            Event::Volume { level, muted } => {
                (self.kind, self.level, self.muted) = (Kind::Volume, *level, *muted);
                SHOW_LEVEL
            }
            Event::Brightness(level) => {
                (self.kind, self.level) = (Kind::Brightness, *level);
                SHOW_LEVEL
            }
            Event::Notice { icon, text, detail } => {
                self.kind = Kind::Notice { icon, text: text.clone(), detail: detail.clone() };
                SHOW_NOTICE
            }
            _ => return,
        };
        self.until = Some(Instant::now() + show);
    }

    fn poll(&mut self) -> Poll {
        // Sudah habis waktunya: kembalikan notch ke isi sebelumnya.
        if self.until.is_some_and(|t| Instant::now() >= t) {
            self.until = None;
            return Poll::Redraw;
        }
        Poll::Idle
    }

    fn poll_in(&self) -> Option<u32> {
        // Bangun saat OSD harus hilang supaya notch kembali ke isi sebelumnya.
        self.until
            .filter(|t| Instant::now() < *t)
            .map(|t| t.saturating_duration_since(Instant::now()).as_millis() as u32 + 10)
    }
}
