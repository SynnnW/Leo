//! Fokus (Pomodoro): fokus → istirahat pendek, dan istirahat panjang setiap beberapa sesi.

use std::time::{Duration, Instant};

use super::{chime, Module, Poll};
use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::settings::SharedSettings;
use crate::time::mmss;

const FOCUS: Color = Color(1.0, 0.42, 0.36, 1.0);
const REST: Color = Color(0.35, 0.78, 1.0, 1.0);

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Focus,
    Short,
    Long,
}

enum Run {
    Idle,
    Running { end: Instant },
    Paused { left: Duration },
}

pub struct Pomodoro {
    settings: SharedSettings,
    phase: Phase,
    run: Run,
    /// Sesi fokus yang selesai dalam putaran ini.
    done: u32,
    /// Pesan singkat setelah fase berganti.
    note: Option<&'static str>,
}

struct Layout {
    time: Rect,
    label: Rect,
    bar: Rect,
    main: Rect,
    reset: Rect,
    skip: Rect,
}

fn layout(r: Rect) -> Layout {
    let bx = r.x + r.w - 24.0 - 120.0;
    Layout {
        time: Rect::new(r.x + 24.0, r.y + 14.0, 200.0, 48.0),
        label: Rect::new(r.x + 24.0, r.y + 62.0, 220.0, 20.0),
        bar: Rect::new(r.x + 24.0, r.y + 100.0, r.w - 48.0, 4.0),
        main: Rect::new(bx, r.y + 18.0, 120.0, 34.0),
        reset: Rect::new(bx, r.y + 60.0, 56.0, 26.0),
        skip: Rect::new(bx + 64.0, r.y + 60.0, 56.0, 26.0),
    }
}

impl Pomodoro {
    pub fn new(settings: SharedSettings) -> Self {
        Self { settings, phase: Phase::Focus, run: Run::Idle, done: 0, note: None }
    }

    fn len(&self, p: Phase) -> Duration {
        let s = self.settings.borrow();
        let min = match p {
            Phase::Focus => s.pomodoro.focus_min,
            Phase::Short => s.pomodoro.short_break_min,
            Phase::Long => s.pomodoro.long_break_min,
        };
        Duration::from_secs(min.max(1) as u64 * 60)
    }

    fn left(&self) -> Duration {
        match self.run {
            Run::Idle => self.len(self.phase),
            Run::Running { end } => end.saturating_duration_since(Instant::now()),
            Run::Paused { left } => left,
        }
    }

    fn color(&self) -> Color {
        if self.phase == Phase::Focus { FOCUS } else { REST }
    }

    fn label(&self) -> String {
        let rounds = self.settings.borrow().pomodoro.rounds.max(1);
        match self.phase {
            Phase::Focus => format!("Fokus · sesi {} dari {}", self.done % rounds + 1, rounds),
            Phase::Short => "Istirahat pendek".into(),
            Phase::Long => "Istirahat panjang".into(),
        }
    }

    /// Fase selesai: fokus lanjut otomatis ke istirahat; istirahat kembali ke fokus dan menunggu.
    fn advance(&mut self, by_timer: bool) {
        if self.phase == Phase::Focus {
            self.done += 1;
            let rounds = self.settings.borrow().pomodoro.rounds.max(1);
            self.phase = if self.done % rounds == 0 { Phase::Long } else { Phase::Short };
            self.run = Run::Running { end: Instant::now() + self.len(self.phase) };
            self.note = by_timer.then_some("Fokus selesai. Waktunya istirahat!");
        } else {
            self.phase = Phase::Focus;
            self.run = Run::Idle;
            self.note = by_timer.then_some("Istirahat selesai. Siap fokus lagi?");
        }
    }
}

impl Module for Pomodoro {
    fn tab(&self) -> Option<&'static str> {
        Some("Fokus")
    }

    fn active(&self) -> bool {
        !matches!(self.run, Run::Idle)
    }

    fn badge(&self) -> Option<(String, Color)> {
        self.active().then(|| (mmss(self.left().as_secs()), self.color()))
    }

    fn expanded_size(&self) -> (f32, f32) {
        (420.0, 124.0)
    }

    fn draw_compact(&mut self, g: &mut Gfx, r: Rect) {
        let name = if self.phase == Phase::Focus { "Fokus" } else { "Istirahat" };
        g.circle(r.x + 18.0, r.y + r.h / 2.0, 4.0, self.color());
        g.text(name, Rect::new(r.x + 30.0, r.y, 90.0, r.h), 12.5, 400, DIM, Align::Left);
        let t = mmss(self.left().as_secs());
        g.text(&t, Rect::new(r.x, r.y, r.w - 16.0, r.h), 13.0, 600, WHITE, Align::Right);
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        let l = layout(r);
        let color = self.color();
        g.text(&mmss(self.left().as_secs()), l.time, 38.0, 600, WHITE, Align::Left);
        let label = match self.note {
            Some(n) => n.to_string(),
            None => self.label(),
        };
        g.text(&label, l.label, 12.5, 400, if self.note.is_some() { color } else { DIM }, Align::Left);

        let total = self.len(self.phase).as_secs_f32();
        let k = 1.0 - self.left().as_secs_f32() / total.max(1.0);
        g.fill_rrect(l.bar, 2.0, FAINT);
        g.fill_rrect(Rect::new(l.bar.x, l.bar.y, l.bar.w * k.clamp(0.0, 1.0), l.bar.h), 2.0, color);

        let main = match self.run {
            Run::Running { .. } => "Jeda",
            Run::Paused { .. } => "Lanjut",
            Run::Idle => "Mulai",
        };
        g.button(l.main, main, color, Color(0.0, 0.0, 0.0, 1.0));
        g.button(l.reset, "Ulang", FAINT, WHITE);
        g.button(l.skip, "Lewati", FAINT, WHITE);
    }

    fn click(&mut self, r: Rect, x: f32, y: f32) -> bool {
        let l = layout(r);
        self.note = None;
        if l.main.contains(x, y) {
            self.run = match self.run {
                Run::Running { end } => Run::Paused { left: end.saturating_duration_since(Instant::now()) },
                Run::Paused { left } => Run::Running { end: Instant::now() + left },
                Run::Idle => Run::Running { end: Instant::now() + self.len(self.phase) },
            };
        } else if l.reset.contains(x, y) {
            self.phase = Phase::Focus;
            self.run = Run::Idle;
            self.done = 0;
        } else if l.skip.contains(x, y) {
            self.advance(false);
        } else {
            return false;
        }
        true
    }

    fn tick_ms(&self, _expanded: bool) -> Option<u32> {
        // Bangun tepat saat angka detik berganti.
        match self.run {
            Run::Running { end } => {
                let ms = end.saturating_duration_since(Instant::now()).as_millis() as u32;
                Some(ms % 1000 + 5)
            }
            _ => None,
        }
    }

    fn poll_in(&self) -> Option<u32> {
        match self.run {
            Run::Running { end } => Some(end.saturating_duration_since(Instant::now()).as_millis() as u32 + 5),
            _ => None,
        }
    }

    fn poll(&mut self) -> Poll {
        if let Run::Running { end } = self.run {
            if Instant::now() >= end {
                self.advance(true);
                chime();
                return Poll::Alert;
            }
        }
        Poll::Idle
    }
}
