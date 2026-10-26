//! Minum: catat air yang diminum hari ini dan ingatkan selama jam aktif.

use serde::{Deserialize, Serialize};

use super::{chime, Module, Poll};
use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::settings::{load_json, save_json, SharedSettings};
use crate::time::{day, minute_of_day, now_ms, MIN};

const BLUE: Color = Color(0.36, 0.72, 1.0, 1.0);
const FILE: &str = "water.json";

#[derive(Serialize, Deserialize, Default)]
struct Log {
    day: i64,
    /// (menit dalam hari, ml)
    entries: Vec<(u32, u32)>,
}

pub struct Water {
    settings: SharedSettings,
    log: Log,
    /// Menit pengingat terakhir hari ini.
    last_alert: Option<u32>,
    alerting: bool,
}

struct Layout {
    amount: Rect,
    label: Rect,
    bar: Rect,
    drink: Rect,
    undo: Rect,
    chart: Rect,
}

fn layout(r: Rect) -> Layout {
    Layout {
        amount: Rect::new(r.x + 24.0, r.y + 14.0, 190.0, 34.0),
        label: Rect::new(r.x + 24.0, r.y + 48.0, 190.0, 18.0),
        bar: Rect::new(r.x + 24.0, r.y + 74.0, 180.0, 6.0),
        drink: Rect::new(r.x + 24.0, r.y + 88.0, 104.0, 28.0),
        undo: Rect::new(r.x + 134.0, r.y + 88.0, 70.0, 28.0),
        chart: Rect::new(r.x + 228.0, r.y + 18.0, r.w - 228.0 - 20.0, 96.0),
    }
}

fn liters(ml: u32) -> String {
    format!("{},{}", ml / 1000, (ml % 1000) / 100)
}

impl Water {
    pub fn new(settings: SharedSettings) -> Self {
        let mut log: Log = load_json(FILE).unwrap_or_default();
        if log.day != day(now_ms()) {
            log = Log { day: day(now_ms()), entries: Vec::new() };
        }
        // Hitung pengingat pertama sejak Amora dibuka, bukan langsung berbunyi saat startup.
        let last_alert = Some(minute_of_day(now_ms()));
        Self { settings, log, last_alert, alerting: false }
    }

    fn rollover(&mut self) {
        let today = day(now_ms());
        if self.log.day != today {
            self.log = Log { day: today, entries: Vec::new() };
            self.last_alert = None;
            save_json(FILE, &self.log);
        }
    }

    fn total(&self) -> u32 {
        self.log.entries.iter().map(|e| e.1).sum()
    }

    /// Menit kapan pengingat berikutnya jatuh, atau `None` bila tidak perlu lagi hari ini.
    fn next_due(&self) -> Option<u32> {
        let s = self.settings.borrow();
        let w = &s.water;
        if !w.remind || self.total() >= w.goal_ml {
            return None;
        }
        let start = w.start_hour * 60;
        let since = self.log.entries.last().map(|e| e.0).unwrap_or(start).max(start);
        let since = self.last_alert.map_or(since, |a| a.max(since));
        let due = since + w.interval_min.max(5);
        (due < w.end_hour * 60).then_some(due)
    }
}

impl Module for Water {
    fn tab(&self) -> Option<&'static str> {
        Some("Minum")
    }

    fn expanded_size(&self) -> (f32, f32) {
        (420.0, 128.0)
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        self.rollover();
        let l = layout(r);
        let (goal, sip, start_h, end_h) = {
            let s = self.settings.borrow();
            (s.water.goal_ml, s.water.sip_ml, s.water.start_hour, s.water.end_hour)
        };
        let total = self.total();
        g.text(&format!("{} / {} L", liters(total), liters(goal)), l.amount, 24.0, 600, WHITE, Align::Left);
        let pct = (total * 100 / goal.max(1)).min(999);
        let label = if self.alerting {
            "Waktunya minum 💧".to_string()
        } else if total >= goal {
            format!("{pct}% · target tercapai")
        } else {
            format!("{pct}% · target hari ini")
        };
        g.text(&label, l.label, 12.5, 400, if self.alerting { BLUE } else { DIM }, Align::Left);
        g.fill_rrect(l.bar, 3.0, FAINT);
        let k = (total as f32 / goal.max(1) as f32).min(1.0);
        g.fill_rrect(Rect::new(l.bar.x, l.bar.y, l.bar.w * k, l.bar.h), 3.0, BLUE);
        g.button(l.drink, &format!("+{sip} ml"), BLUE, Color(0.0, 0.0, 0.0, 1.0));
        if !self.log.entries.is_empty() {
            g.button(l.undo, "Batal", FAINT, WHITE);
        }

        // Grafik per jam selama jam aktif.
        let hours = (start_h..end_h.max(start_h + 1)).collect::<Vec<_>>();
        let mut per_hour = vec![0u32; hours.len()];
        for (m, ml) in &self.log.entries {
            if let Some(i) = hours.iter().position(|h| *h == m / 60) {
                per_hour[i] += ml;
            }
        }
        let max = per_hour.iter().copied().max().unwrap_or(0).max(sip * 2);
        let c = l.chart;
        let label_h = 14.0;
        let slot = c.w / hours.len() as f32;
        let bw = (slot * 0.6).max(2.0);
        let now_h = minute_of_day(now_ms()) / 60;
        for (i, h) in hours.iter().enumerate() {
            let x = c.x + i as f32 * slot + (slot - bw) / 2.0;
            let bh = (c.h - label_h) * per_hour[i] as f32 / max as f32;
            let base = c.y + c.h - label_h;
            g.fill_rrect(Rect::new(x, base - 2.0, bw, 2.0), 1.0, FAINT);
            if bh > 0.0 {
                g.fill_rrect(Rect::new(x, base - bh, bw, bh), (bw / 2.0).min(3.0), BLUE);
            }
            // Label tiap 3 jam, plus jam sekarang; jangan berdempetan dengan jam sekarang.
            if *h == now_h || (h % 3 == 0 && h.abs_diff(now_h) > 1) {
                let col = if *h == now_h { WHITE } else { DIM };
                g.text(&format!("{h:02}"), Rect::new(x - 8.0, base + 1.0, bw + 16.0, label_h), 10.0, 400, col, Align::Center);
            }
        }
    }

    fn click(&mut self, r: Rect, x: f32, y: f32) -> bool {
        self.rollover();
        let l = layout(r);
        if l.drink.contains(x, y) {
            let sip = self.settings.borrow().water.sip_ml;
            self.log.entries.push((minute_of_day(now_ms()), sip));
        } else if l.undo.contains(x, y) && !self.log.entries.is_empty() {
            self.log.entries.pop();
        } else {
            return false;
        }
        self.alerting = false;
        save_json(FILE, &self.log);
        true
    }

    fn poll_in(&self) -> Option<u32> {
        let now = minute_of_day(now_ms());
        let ms_left_in_minute = (MIN - now_ms() % MIN) as u32;
        match self.next_due() {
            Some(due) if due > now => Some((due - now - 1) * 60_000 + ms_left_in_minute),
            Some(_) => Some(1000),
            // Cek lagi nanti (mis. ganti hari atau setelan diubah).
            None => Some(30 * 60_000),
        }
    }

    fn poll(&mut self) -> Poll {
        self.rollover();
        let now = minute_of_day(now_ms());
        let start = self.settings.borrow().water.start_hour * 60;
        match self.next_due() {
            Some(due) if now >= due && now >= start => {
                self.last_alert = Some(now);
                self.alerting = true;
                chime();
                Poll::Alert
            }
            _ => Poll::Idle,
        }
    }
}
