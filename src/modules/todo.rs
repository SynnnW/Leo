//! To-do dengan pengingat. Ketik tugas langsung di notch; akhiri dengan "@14:30"
//! (atau "@9") untuk pengingat jam itu. Kalau jamnya sudah lewat, untuk besok.

use serde::{Deserialize, Serialize};

use super::{chime, Module, Poll};
use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::settings::{load_json, save_json};
use crate::time::{day, hhmm, now_ms, DAY, MIN};

const FILE: &str = "todo.json";
const ACCENT: Color = Color(0.35, 0.85, 0.55, 1.0);
const LATE: Color = Color(1.0, 0.45, 0.4, 1.0);
const MAX_ROWS: usize = 5;
const ROW: f32 = 26.0;

#[derive(Serialize, Deserialize, Clone)]
struct Task {
    text: String,
    done: bool,
    /// Waktu lokal (ms) pengingat.
    due: Option<i64>,
    notified: bool,
    /// Hari saat dicentang; tugas selesai hilang keesokan harinya.
    done_day: Option<i64>,
}

enum Hit {
    Toggle(usize),
    Delete(usize),
    Add,
}

pub struct Todo {
    tasks: Vec<Task>,
    editing: Option<String>,
    hits: Vec<(Rect, Hit)>,
    /// Tugas yang baru saja jatuh tempo (disorot).
    flash: Option<usize>,
}

/// Pisahkan "teks @14:30" → ("teks", Some(menit)). Menerima @14:30, @14.30, @9.
pub fn parse_due(input: &str) -> (String, Option<u32>) {
    let s = input.trim();
    if let Some(i) = s.rfind('@') {
        let t = s[i + 1..].trim();
        let (h, m) = match t.split_once([':', '.']) {
            Some((h, m)) => (h.parse::<u32>().ok(), m.parse::<u32>().ok()),
            None => (t.parse::<u32>().ok(), Some(0)),
        };
        if let (Some(h), Some(m)) = (h, m) {
            if h < 24 && m < 60 {
                return (s[..i].trim().to_string(), Some(h * 60 + m));
            }
        }
    }
    (s.to_string(), None)
}

/// Menit-dalam-hari → waktu absolut berikutnya (hari ini, atau besok kalau sudah lewat).
fn next_at(minute: u32, now: i64) -> i64 {
    let today = day(now) * DAY + minute as i64 * MIN;
    if today > now { today } else { today + DAY }
}

impl Todo {
    pub fn new() -> Self {
        let mut tasks: Vec<Task> = load_json(FILE).unwrap_or_default();
        let today = day(now_ms());
        tasks.retain(|t| !t.done || t.done_day == Some(today));
        Self { tasks, editing: None, hits: Vec::new(), flash: None }
    }

    fn save(&self) {
        save_json(FILE, &self.tasks);
    }

    /// Belum selesai dulu (yang paling dekat jatuh tempo di atas), lalu yang sudah selesai.
    fn order(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.tasks.len()).collect();
        idx.sort_by_key(|&i| {
            let t = &self.tasks[i];
            (t.done, t.due.unwrap_or(i64::MAX), i)
        });
        idx
    }

    fn rows(&self) -> usize {
        self.tasks.len().min(MAX_ROWS)
    }

    fn commit(&mut self) {
        let Some(text) = self.editing.take() else { return };
        let (text, minute) = parse_due(&text);
        if text.is_empty() {
            return;
        }
        let due = minute.map(|m| next_at(m, now_ms()));
        self.tasks.push(Task { text, done: false, due, notified: false, done_day: None });
        self.save();
    }
}

impl Module for Todo {
    fn tab(&self) -> Option<&'static str> {
        Some("To-do")
    }

    fn expanded_size(&self) -> (f32, f32) {
        let extra = if self.tasks.len() > MAX_ROWS { 18.0 } else { 0.0 };
        (420.0, 44.0 + self.rows() as f32 * ROW + 36.0 + extra)
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        self.hits.clear();
        let done = self.tasks.iter().filter(|t| t.done).count();
        g.text("To-do", Rect::new(r.x + 20.0, r.y + 10.0, 120.0, 24.0), 15.0, 600, WHITE, Align::Left);
        if !self.tasks.is_empty() {
            let s = format!("{done}/{} selesai", self.tasks.len());
            g.text(&s, Rect::new(r.x, r.y + 10.0, r.w - 20.0, 24.0), 12.0, 400, DIM, Align::Right);
        }

        let now = now_ms();
        let mut y = r.y + 40.0;
        let order = self.order();
        for &i in order.iter().take(MAX_ROWS) {
            let t = &self.tasks[i];
            let row = Rect::new(r.x + 16.0, y, r.w - 32.0, ROW);
            if self.flash == Some(i) {
                g.fill_rrect(row, 8.0, Color(1.0, 0.45, 0.4, 0.18));
            }
            let (cx, cy) = (row.x + 12.0, row.y + ROW / 2.0);
            if t.done {
                g.circle(cx, cy, 7.0, ACCENT);
            } else {
                g.ring(cx, cy, 6.5, 1.5, DIM);
            }
            let tc = if t.done { Color(1.0, 1.0, 1.0, 0.35) } else { WHITE };
            g.text(&t.text, Rect::new(row.x + 28.0, row.y, row.w - 28.0 - 92.0, ROW), 13.0, 400, tc, Align::Left);
            if let Some(due) = t.due.filter(|_| !t.done) {
                let late = due <= now;
                let label = if day(due) == day(now) { hhmm(due) } else { format!("besok {}", hhmm(due)) };
                let dr = Rect::new(row.x, row.y, row.w - 26.0, ROW);
                g.text(&label, dr, 11.5, 400, if late { LATE } else { DIM }, Align::Right);
            }
            let del = Rect::new(row.x + row.w - 22.0, row.y, 22.0, ROW);
            g.text("×", del, 15.0, 400, Color(1.0, 1.0, 1.0, 0.35), Align::Center);
            self.hits.push((Rect::new(row.x, row.y, row.w - 120.0, ROW), Hit::Toggle(i)));
            self.hits.push((del, Hit::Delete(i)));
            y += ROW;
        }
        if self.tasks.len() > MAX_ROWS {
            let more = format!("+{} lainnya", self.tasks.len() - MAX_ROWS);
            g.text(&more, Rect::new(r.x + 44.0, y, 200.0, 18.0), 11.5, 400, DIM, Align::Left);
            y += 18.0;
        }

        let add = Rect::new(r.x + 16.0, y + 4.0, r.w - 32.0, 28.0);
        match &self.editing {
            Some(text) => {
                g.fill_rrect(add, 8.0, FAINT);
                let inner = Rect::new(add.x + 12.0, add.y, add.w - 24.0, add.h);
                if text.is_empty() {
                    g.text("Tulis tugas… tambahkan @14:30 untuk pengingat", inner, 12.5, 400, DIM, Align::Left);
                } else {
                    g.text(&format!("{text}▏"), inner, 13.0, 400, WHITE, Align::Left);
                }
            }
            None => {
                g.text("+  Tambah tugas", Rect::new(add.x + 6.0, add.y, add.w, add.h), 12.5, 600, DIM, Align::Left);
            }
        }
        self.hits.push((add, Hit::Add));
    }

    fn click(&mut self, _r: Rect, x: f32, y: f32) -> bool {
        let Some(pos) = self.hits.iter().position(|(r, _)| r.contains(x, y)) else {
            // Klik di tempat lain menyelesaikan ketikan.
            if self.editing.is_some() {
                self.commit();
                return true;
            }
            return false;
        };
        self.flash = None;
        match self.hits[pos].1 {
            Hit::Toggle(i) => {
                let t = &mut self.tasks[i];
                t.done = !t.done;
                t.done_day = t.done.then(|| day(now_ms()));
                self.save();
            }
            Hit::Delete(i) => {
                self.tasks.remove(i);
                self.save();
            }
            Hit::Add => {
                if self.editing.is_none() {
                    self.editing = Some(String::new());
                }
            }
        }
        true
    }

    fn editing(&self) -> bool {
        self.editing.is_some()
    }

    fn key_char(&mut self, c: char) {
        let Some(text) = &mut self.editing else { return };
        match c {
            '\r' => self.commit(),
            '\u{1b}' => self.editing = None,
            '\u{8}' => {
                text.pop();
            }
            c if !c.is_control() && text.chars().count() < 120 => text.push(c),
            _ => {}
        }
    }

    fn stop_editing(&mut self) {
        self.commit();
    }

    fn poll_in(&self) -> Option<u32> {
        let now = now_ms();
        self.tasks
            .iter()
            .filter(|t| !t.done && !t.notified)
            .filter_map(|t| t.due)
            .map(|d| (d - now).clamp(0, 6 * 3600 * 1000) as u32 + 5)
            .min()
    }

    fn poll(&mut self) -> Poll {
        let now = now_ms();
        let hit = self.tasks.iter().position(|t| !t.done && !t.notified && t.due.is_some_and(|d| d <= now));
        if let Some(i) = hit {
            self.tasks[i].notified = true;
            self.flash = Some(i);
            self.save();
            chime();
            return Poll::Alert;
        }
        Poll::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_parsing() {
        assert_eq!(parse_due("Balas Dina @14:30"), ("Balas Dina".into(), Some(14 * 60 + 30)));
        assert_eq!(parse_due("Kumpul laporan @9"), ("Kumpul laporan".into(), Some(9 * 60)));
        assert_eq!(parse_due("Rapat @08.15"), ("Rapat".into(), Some(8 * 60 + 15)));
        assert_eq!(parse_due("Email ke a@b.com"), ("Email ke a@b.com".into(), None));
        assert_eq!(parse_due("Tanpa jam"), ("Tanpa jam".into(), None));
        assert_eq!(parse_due("Salah @25:00"), ("Salah @25:00".into(), None));
    }

    #[test]
    fn next_time_rolls_to_tomorrow() {
        let now = 100 * DAY + 15 * 60 * MIN; // hari ke-100, 15:00
        assert_eq!(next_at(16 * 60, now), 100 * DAY + 16 * 60 * MIN);
        assert_eq!(next_at(9 * 60, now), 101 * DAY + 9 * 60 * MIN);
    }
}
