//! Jadwal kuliah mingguan. Ketik di notch, mis. "Senin 08:00-09:40 Basis Data @R301"
//! (hari boleh dilewati = hari ini; "@ruang" boleh dilewati). Notch mengingatkan
//! 10 menit sebelum kelas, dan menampilkan hitung mundur di notch tertutup.

use serde::{Deserialize, Serialize};

use super::{chime, Module, Poll};
use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::settings::{load_json, save_json};
use crate::time::{day, minute_of_day, now_ms, weekday, MIN};

const FILE: &str = "schedule.json";
const ACCENT: Color = Color(0.55, 0.7, 1.0, 1.0);
const NOW_BG: Color = Color(0.55, 0.7, 1.0, 0.16);
const ROW: f32 = 26.0;
const MAX_ROWS: usize = 6;
/// Pengingat sekian menit sebelum kelas mulai.
const REMIND_MIN: u16 = 10;
/// Hitung mundur di notch tertutup mulai sekian menit sebelum kelas.
const BADGE_MIN: u16 = 30;

const HARI: [&str; 7] = ["Minggu", "Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu"];
const HARI_PENDEK: [&str; 7] = ["Min", "Sen", "Sel", "Rab", "Kam", "Jum", "Sab"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Class {
    /// 0 = Minggu … 6 = Sabtu
    pub day: u8,
    /// Menit sejak tengah malam.
    pub start: u16,
    pub end: u16,
    pub name: String,
    pub room: String,
}

fn hm(m: u16) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

fn parse_time(s: &str) -> Option<u16> {
    let (h, m) = s.split_once([':', '.'])?;
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

fn parse_day(word: &str) -> Option<u8> {
    // "sen", "senin", "Jum", "jumat," … minimal 3 huruf.
    let w = word.to_lowercase();
    let w = w.trim_end_matches(',');
    if w.chars().count() < 3 {
        return None;
    }
    const DAYS: [&str; 7] = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
    HARI.iter()
        .position(|h| h.to_lowercase().starts_with(w))
        .or_else(|| DAYS.iter().position(|d| d.starts_with(w)))
        .map(|i| i as u8)
}

/// "Senin 08:00-09:40 Basis Data @R301" → Class. `today` dipakai bila hari tidak ditulis.
pub fn parse_class(input: &str, today: u8) -> Option<Class> {
    let mut rest = input.trim();
    let mut day = today;
    if let Some((first, after)) = rest.split_once(' ') {
        if let Some(d) = parse_day(first) {
            day = d;
            rest = after.trim();
        }
    }
    let (times, after) = rest.split_once(' ')?;
    let (a, b) = times.split_once(['-', '–'])?;
    let (start, end) = (parse_time(a)?, parse_time(b)?);
    if end <= start {
        return None;
    }
    let (name, room) = match after.rsplit_once('@') {
        Some((n, r)) => (n.trim(), r.trim()),
        None => (after.trim(), ""),
    };
    if name.is_empty() {
        return None;
    }
    Some(Class { day, start, end, name: name.into(), room: room.into() })
}

enum Hit {
    Delete(usize),
    Add,
    ToggleView,
}

pub struct Schedule {
    classes: Vec<Class>,
    /// Tampilkan semua hari, bukan hanya hari ini.
    all: bool,
    editing: Option<String>,
    /// Input terakhir tidak bisa dibaca.
    error: bool,
    hits: Vec<(Rect, Hit)>,
    /// (hari, indeks kelas) yang sudah diingatkan.
    reminded: Vec<(i64, usize)>,
}

impl Schedule {
    pub fn new() -> Self {
        let classes = load_json(FILE).unwrap_or_default();
        Self { classes, all: false, editing: None, error: false, hits: Vec::new(), reminded: Vec::new() }
    }

    fn save(&self) {
        save_json(FILE, &self.classes);
    }

    /// Indeks kelas yang ditampilkan, urut hari lalu jam.
    fn visible(&self) -> Vec<usize> {
        let today = weekday(now_ms());
        let mut idx: Vec<usize> = (0..self.classes.len()).filter(|&i| self.all || self.classes[i].day == today).collect();
        // Urutkan mulai hari ini, supaya "Semua" dimulai dari hari ini.
        idx.sort_by_key(|&i| {
            let c = &self.classes[i];
            ((c.day + 7 - today) % 7, c.start)
        });
        idx
    }

    /// Kelas hari ini berikutnya yang belum mulai: (indeks, menit sampai mulai).
    fn next_today(&self) -> Option<(usize, u16)> {
        let now = now_ms();
        let (wd, m) = (weekday(now), minute_of_day(now) as u16);
        self.classes
            .iter()
            .enumerate()
            .filter(|(_, c)| c.day == wd && c.start > m)
            .min_by_key(|(_, c)| c.start)
            .map(|(i, c)| (i, c.start - m))
    }

    fn rows(&self) -> usize {
        self.visible().len().clamp(1, MAX_ROWS)
    }

    fn commit(&mut self) {
        let Some(text) = self.editing.take() else { return };
        if text.trim().is_empty() {
            return;
        }
        match parse_class(&text, weekday(now_ms())) {
            Some(c) => {
                self.classes.push(c);
                self.error = false;
                self.save();
            }
            None => {
                // Biarkan pengguna memperbaiki ketikannya.
                self.error = true;
                self.editing = Some(text);
            }
        }
    }
}

impl Module for Schedule {
    fn tab(&self) -> Option<&'static str> {
        Some("Jadwal")
    }

    fn badge(&self) -> Option<(String, Color)> {
        let (_, mins) = self.next_today()?;
        (mins <= BADGE_MIN).then(|| (format!("▸ {mins}m"), ACCENT))
    }

    fn expanded_size(&self) -> (f32, f32) {
        let more = if self.visible().len() > MAX_ROWS { 18.0 } else { 0.0 };
        (420.0, 44.0 + self.rows() as f32 * ROW + more + 36.0)
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        self.hits.clear();
        let now = now_ms();
        let (wd, m) = (weekday(now), minute_of_day(now) as u16);
        let title = if self.all { "Jadwal · Semua".to_string() } else { format!("Jadwal · {}", HARI[wd as usize]) };
        g.text(&title, Rect::new(r.x + 20.0, r.y + 10.0, 240.0, 24.0), 15.0, 600, WHITE, Align::Left);
        let toggle = Rect::new(r.x + r.w - 20.0 - 90.0, r.y + 10.0, 90.0, 24.0);
        g.text(if self.all { "Hari ini" } else { "Semua" }, toggle, 12.0, 400, DIM, Align::Right);
        self.hits.push((toggle, Hit::ToggleView));

        let vis = self.visible();
        let mut y = r.y + 40.0;
        if vis.is_empty() {
            let msg = if self.all { "Belum ada jadwal" } else { "Tidak ada kelas hari ini" };
            g.text(msg, Rect::new(r.x + 20.0, y, r.w - 40.0, ROW), 12.5, 400, DIM, Align::Left);
            y += ROW;
        }
        for &i in vis.iter().take(MAX_ROWS) {
            let c = &self.classes[i];
            let row = Rect::new(r.x + 16.0, y, r.w - 32.0, ROW);
            let live = c.day == wd && c.start <= m && m < c.end;
            let past = c.day == wd && c.end <= m && !self.all;
            if live {
                g.fill_rrect(row, 8.0, NOW_BG);
            }
            let tc = if past { Color(1.0, 1.0, 1.0, 0.35) } else { WHITE };
            let time = if self.all {
                format!("{} {}", HARI_PENDEK[c.day as usize], hm(c.start))
            } else {
                format!("{}–{}", hm(c.start), hm(c.end))
            };
            g.text(&time, Rect::new(row.x + 8.0, row.y, 96.0, ROW), 12.0, 600, if live { ACCENT } else { DIM }, Align::Left);
            g.text(&c.name, Rect::new(row.x + 108.0, row.y, row.w - 108.0 - 96.0, ROW), 13.0, 400, tc, Align::Left);
            g.text(&c.room, Rect::new(row.x, row.y, row.w - 28.0, ROW), 11.5, 400, DIM, Align::Right);
            let del = Rect::new(row.x + row.w - 22.0, row.y, 22.0, ROW);
            g.text("×", del, 15.0, 400, Color(1.0, 1.0, 1.0, 0.35), Align::Center);
            self.hits.push((del, Hit::Delete(i)));
            y += ROW;
        }
        if vis.len() > MAX_ROWS {
            let more = format!("+{} lainnya", vis.len() - MAX_ROWS);
            g.text(&more, Rect::new(r.x + 24.0, y, 200.0, 18.0), 11.5, 400, DIM, Align::Left);
            y += 18.0;
        }

        let add = Rect::new(r.x + 16.0, y + 4.0, r.w - 32.0, 28.0);
        match &self.editing {
            Some(text) => {
                g.fill_rrect(add, 8.0, if self.error { Color(1.0, 0.4, 0.35, 0.25) } else { FAINT });
                let inner = Rect::new(add.x + 12.0, add.y, add.w - 24.0, add.h);
                if text.is_empty() {
                    g.text("Senin 08:00-09:40 Basis Data @R301", inner, 12.5, 400, DIM, Align::Left);
                } else {
                    g.text(&format!("{text}▏"), inner, 13.0, 400, WHITE, Align::Left);
                }
            }
            None => g.text("+  Tambah kelas", Rect::new(add.x + 6.0, add.y, add.w, add.h), 12.5, 600, DIM, Align::Left),
        }
        self.hits.push((add, Hit::Add));
    }

    fn click(&mut self, _r: Rect, x: f32, y: f32) -> bool {
        let Some(pos) = self.hits.iter().position(|(r, _)| r.contains(x, y)) else {
            if self.editing.is_some() {
                self.commit();
                return true;
            }
            return false;
        };
        match self.hits[pos].1 {
            Hit::Delete(i) => {
                self.classes.remove(i);
                self.reminded.clear();
                self.save();
            }
            Hit::Add => {
                if self.editing.is_none() {
                    self.editing = Some(String::new());
                    self.error = false;
                }
            }
            Hit::ToggleView => self.all = !self.all,
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
            '\u{1b}' => {
                self.editing = None;
                self.error = false;
            }
            '\u{8}' => {
                text.pop();
            }
            c if !c.is_control() && text.chars().count() < 100 => text.push(c),
            _ => {}
        }
    }

    fn stop_editing(&mut self) {
        self.commit();
        // Klik di luar notch: buang ketikan yang tidak valid.
        self.editing = None;
        self.error = false;
    }

    fn tick_ms(&self, _expanded: bool) -> Option<u32> {
        // Hitung mundur per menit selama ada kelas yang segera mulai.
        self.next_today().filter(|(_, m)| *m <= BADGE_MIN).map(|_| (MIN - now_ms() % MIN) as u32 + 20)
    }

    fn poll_in(&self) -> Option<u32> {
        let now = now_ms();
        let to_next_minute = (MIN - now % MIN) as u32 + 20;
        match self.next_today() {
            // Tepat saat menit pengingat tiba, atau saat badge perlu mulai tampil.
            Some((_, m)) if m > BADGE_MIN => Some((m - BADGE_MIN - 1) as u32 * 60_000 + to_next_minute),
            Some(_) => Some(to_next_minute),
            // Tidak ada kelas lagi hari ini: cek lagi setelah tengah malam.
            None => Some((crate::time::DAY - now % crate::time::DAY) as u32 + 1000),
        }
    }

    fn poll(&mut self) -> Poll {
        let Some((i, mins)) = self.next_today() else { return Poll::Idle };
        let key = (day(now_ms()), i);
        if mins <= REMIND_MIN && !self.reminded.contains(&key) {
            self.reminded.push(key);
            self.all = false;
            chime();
            return Poll::Alert;
        }
        // Badge berubah tiap menit.
        if mins <= BADGE_MIN { Poll::Redraw } else { Poll::Idle }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(day: u8, s: &str, e: &str, name: &str, room: &str) -> Class {
        Class { day, start: parse_time(s).unwrap(), end: parse_time(e).unwrap(), name: name.into(), room: room.into() }
    }

    #[test]
    fn full_line() {
        assert_eq!(parse_class("Senin 08:00-09:40 Basis Data @R301", 5), Some(c(1, "08:00", "09:40", "Basis Data", "R301")));
    }

    #[test]
    fn short_day_dots_and_no_room() {
        assert_eq!(parse_class("kam 13.00-15.30 Jaringan Komputer", 1), Some(c(4, "13:00", "15:30", "Jaringan Komputer", "")));
    }

    #[test]
    fn no_day_means_today() {
        assert_eq!(parse_class("10:00-11:40 Statistika @Lab 2", 3), Some(c(3, "10:00", "11:40", "Statistika", "Lab 2")));
    }

    #[test]
    fn english_day_names() {
        assert_eq!(parse_class("Mon 08:00-09:40 Databases @R301", 5), Some(c(1, "08:00", "09:40", "Databases", "R301")));
        assert_eq!(parse_class("thu 13.00-15.30 Networks", 1), Some(c(4, "13:00", "15:30", "Networks", "")));
    }

    #[test]
    fn invalid() {
        assert_eq!(parse_class("Senin Basis Data", 1), None);
        assert_eq!(parse_class("Senin 10:00-09:00 Mundur", 1), None);
        assert_eq!(parse_class("Senin 08:00-09:40", 1), None);
    }

    #[test]
    fn weekday_math() {
        // 1 Januari 1601 = Senin.
        assert_eq!(crate::time::weekday(0), 1);
        assert_eq!(crate::time::weekday(6 * crate::time::DAY), 0);
    }
}
