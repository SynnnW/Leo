//! Halaman Setelan: semua pilihan yang sebelumnya harus diubah di settings.json.
//! Perubahan langsung disimpan dan langsung berlaku.

use super::Module;
use crate::autostart;
use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::settings::{Settings, SharedSettings};

const ACCENT: Color = Color(0.35, 0.85, 0.55, 1.0);
const ROW: f32 = 26.0;
const COL_GAP: f32 = 24.0;

#[derive(Clone, Copy)]
enum Toggle {
    Autostart,
    Lyrics,
    AutoHide,
    Mascot,
    Minimal,
    Screenshot,
    Clipboard,
    TidyTables,
    WaterRemind,
}

#[derive(Clone, Copy)]
enum Step {
    Focus,
    ShortBreak,
    LongBreak,
    WaterGoal,
    WaterEvery,
    WaterStart,
    WaterEnd,
}

enum Hit {
    Toggle(Toggle),
    Step(Step, i32),
}

const LEFT: [Toggle; 9] = [
    Toggle::Autostart,
    Toggle::Lyrics,
    Toggle::AutoHide,
    Toggle::Mascot,
    Toggle::Minimal,
    Toggle::Screenshot,
    Toggle::Clipboard,
    Toggle::TidyTables,
    Toggle::WaterRemind,
];
const RIGHT: [Step; 7] = [
    Step::Focus,
    Step::ShortBreak,
    Step::LongBreak,
    Step::WaterGoal,
    Step::WaterEvery,
    Step::WaterStart,
    Step::WaterEnd,
];

fn toggle_label(t: Toggle) -> &'static str {
    match t {
        Toggle::Autostart => "Mulai bersama Windows",
        Toggle::Lyrics => "Lirik lagu",
        Toggle::AutoHide => "Minggir untuk tab di belakang",
        Toggle::Mascot => "Maskot",
        Toggle::Minimal => "Notch minimal",
        Toggle::Screenshot => "Screenshot ke Shelf",
        Toggle::Clipboard => "Riwayat clipboard",
        Toggle::TidyTables => "Rapikan tabel yang disalin",
        Toggle::WaterRemind => "Pengingat minum",
    }
}

fn toggle_get(s: &Settings, t: Toggle) -> bool {
    match t {
        Toggle::Autostart => autostart::is_enabled(),
        Toggle::Lyrics => s.lyrics,
        Toggle::AutoHide => s.auto_hide,
        Toggle::Mascot => s.mascot,
        Toggle::Minimal => s.minimal,
        Toggle::Screenshot => s.screenshot_to_shelf,
        Toggle::Clipboard => s.clipboard,
        Toggle::TidyTables => s.tidy_tables,
        Toggle::WaterRemind => s.water.remind,
    }
}

fn toggle_flip(s: &mut Settings, t: Toggle) {
    match t {
        Toggle::Autostart => autostart::set(!autostart::is_enabled()),
        Toggle::Lyrics => s.lyrics = !s.lyrics,
        Toggle::AutoHide => s.auto_hide = !s.auto_hide,
        Toggle::Mascot => s.mascot = !s.mascot,
        Toggle::Minimal => s.minimal = !s.minimal,
        Toggle::Screenshot => s.screenshot_to_shelf = !s.screenshot_to_shelf,
        Toggle::Clipboard => s.clipboard = !s.clipboard,
        Toggle::TidyTables => s.tidy_tables = !s.tidy_tables,
        Toggle::WaterRemind => s.water.remind = !s.water.remind,
    }
}

/// (label, nilai sekarang, langkah, min, maks, satuan)
fn step_info(s: &Settings, k: Step) -> (&'static str, u32, u32, u32, u32, &'static str) {
    match k {
        Step::Focus => ("Fokus", s.pomodoro.focus_min, 5, 5, 120, "mnt"),
        Step::ShortBreak => ("Istirahat pendek", s.pomodoro.short_break_min, 1, 1, 30, "mnt"),
        Step::LongBreak => ("Istirahat panjang", s.pomodoro.long_break_min, 5, 5, 60, "mnt"),
        Step::WaterGoal => ("Target minum", s.water.goal_ml, 250, 500, 5000, "ml"),
        Step::WaterEvery => ("Ingatkan tiap", s.water.interval_min, 15, 15, 240, "mnt"),
        Step::WaterStart => ("Minum mulai jam", s.water.start_hour, 1, 0, 23, ""),
        Step::WaterEnd => ("Minum sampai jam", s.water.end_hour, 1, 1, 24, ""),
    }
}

fn step_apply(s: &mut Settings, k: Step, dir: i32) {
    let (_, v, step, min, max, _) = step_info(s, k);
    let v = (v as i64 + dir as i64 * step as i64).clamp(min as i64, max as i64) as u32;
    match k {
        Step::Focus => s.pomodoro.focus_min = v,
        Step::ShortBreak => s.pomodoro.short_break_min = v,
        Step::LongBreak => s.pomodoro.long_break_min = v,
        Step::WaterGoal => s.water.goal_ml = v,
        Step::WaterEvery => s.water.interval_min = v,
        // Jam aktif tidak boleh terbalik.
        Step::WaterStart => s.water.start_hour = v.min(s.water.end_hour.saturating_sub(1)),
        Step::WaterEnd => s.water.end_hour = v.max(s.water.start_hour + 1),
    }
}

pub struct SettingsPage {
    settings: SharedSettings,
    hits: Vec<(Rect, Hit)>,
}

impl SettingsPage {
    pub fn new(settings: SharedSettings) -> Self {
        Self { settings, hits: Vec::new() }
    }
}

impl Module for SettingsPage {
    fn tab(&self) -> Option<&'static str> {
        Some("Setelan")
    }

    fn expanded_size(&self) -> (f32, f32) {
        (560.0, 44.0 + LEFT.len().max(RIGHT.len()) as f32 * ROW + 10.0)
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        self.hits.clear();
        g.text("Setelan", Rect::new(r.x + 20.0, r.y + 10.0, 200.0, 24.0), 15.0, 600, WHITE, Align::Left);
        let s = self.settings.borrow().clone();
        let col_w = (r.w - 40.0 - COL_GAP) / 2.0;
        let (lx, rx) = (r.x + 20.0, r.x + 20.0 + col_w + COL_GAP);
        let top = r.y + 40.0;

        for (i, t) in LEFT.iter().enumerate() {
            let row = Rect::new(lx, top + i as f32 * ROW, col_w, ROW);
            let on = toggle_get(&s, *t);
            g.text(toggle_label(*t), Rect::new(row.x, row.y, row.w - 44.0, ROW), 12.5, 400, WHITE, Align::Left);
            // Sakelar kecil.
            let sw = Rect::new(row.x + row.w - 34.0, row.y + (ROW - 18.0) / 2.0, 34.0, 18.0);
            g.fill_rrect(sw, 9.0, if on { ACCENT } else { FAINT });
            let kx = if on { sw.x + sw.w - 9.0 } else { sw.x + 9.0 };
            g.circle(kx, sw.y + 9.0, 7.0, WHITE);
            self.hits.push((row, Hit::Toggle(*t)));
        }

        for (i, k) in RIGHT.iter().enumerate() {
            let row = Rect::new(rx, top + i as f32 * ROW, col_w, ROW);
            let (label, v, _, _, _, unit) = step_info(&s, *k);
            g.text(label, Rect::new(row.x, row.y, row.w - 110.0, ROW), 12.5, 400, WHITE, Align::Left);
            let minus = Rect::new(row.x + row.w - 104.0, row.y + 3.0, 22.0, ROW - 6.0);
            let plus = Rect::new(row.x + row.w - 22.0, row.y + 3.0, 22.0, ROW - 6.0);
            g.fill_rrect(minus, 6.0, FAINT);
            g.fill_rrect(plus, 6.0, FAINT);
            g.text("−", minus, 13.0, 600, WHITE, Align::Center);
            g.text("+", plus, 13.0, 600, WHITE, Align::Center);
            let value = if unit.is_empty() { format!("{v:02}.00") } else { format!("{v} {unit}") };
            let vr = Rect::new(minus.x + minus.w, row.y, plus.x - minus.x - minus.w, ROW);
            g.text(&value, vr, 12.0, 600, DIM, Align::Center);
            self.hits.push((minus, Hit::Step(*k, -1)));
            self.hits.push((plus, Hit::Step(*k, 1)));
        }
    }

    fn click(&mut self, _r: Rect, x: f32, y: f32) -> bool {
        let Some((_, hit)) = self.hits.iter().find(|(r, _)| r.contains(x, y)) else { return false };
        let mut s = self.settings.borrow_mut();
        match hit {
            Hit::Toggle(t) => toggle_flip(&mut s, *t),
            Hit::Step(k, d) => step_apply(&mut s, *k, *d),
        }
        s.save();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_clamp_and_keep_hours_ordered() {
        let mut s = Settings::default();
        step_apply(&mut s, Step::Focus, -1);
        assert_eq!(s.pomodoro.focus_min, 20);
        for _ in 0..100 {
            step_apply(&mut s, Step::Focus, 1);
        }
        assert_eq!(s.pomodoro.focus_min, 120);
        s.water.start_hour = 8;
        s.water.end_hour = 9;
        step_apply(&mut s, Step::WaterStart, 1);
        assert_eq!(s.water.start_hour, 8, "mulai tidak boleh menyusul selesai");
        step_apply(&mut s, Step::WaterEnd, -1);
        assert_eq!(s.water.end_hour, 9, "selesai tidak boleh mendahului mulai");
    }
}
