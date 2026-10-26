//! Musik dari pemutar apa pun (Spotify, YouTube di browser, dll.) lewat SMTC,
//! plus lirik tersinkron di sisi kanan notch.

use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::Graphics::Imaging::{IWICBitmap, IWICImagingFactory};

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Event, Module, Poll};
use crate::gfx::{decode_image, Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::lyrics::{Line, Lyrics, LyricsService, State};
use crate::media::{MediaCmd, MediaHandle, MediaInfo, Shared};
use crate::settings::{load_json, save_json, SharedSettings};

const ACCENT: Color = Color(0.35, 0.85, 0.55, 1.0);
const PLAYER_W: f32 = 420.0;
const LYRICS_W: f32 = 300.0;
const HEIGHT: f32 = 136.0;
/// Baris lirik dimajukan sedikit supaya terasa pas dengan suara.
const LEAD: f64 = 0.15;
const ROW_H: f32 = 22.0;
const LINE_ANIM: Duration = Duration::from_millis(240);
const OFFSET_STEP: f64 = 0.5;
const OFFSETS_FILE: &str = "lyric_offsets.json";
/// Lirik di notch tertutup yang tidak muat: diam sebentar, lalu bergeser sekali sampai ujung.
const MARQUEE_PAUSE: f32 = 0.8;
/// Kecepatan geser (DIP/detik); dipercepat bila perlu supaya selesai sebelum baris berikutnya.
const MARQUEE_SPEED: f32 = 30.0;
const MARQUEE_MAX_SPEED: f32 = 140.0;
const MARQUEE_FRAME_MS: u32 = 40;

enum LyricsCtl {
    Research,
    Shift(f64),
}

pub struct MediaModule {
    shared: Shared,
    cmd: MediaHandle,
    wic: IWICImagingFactory,
    info: Option<MediaInfo>,
    thumb_id: u64,
    thumb_src: Option<IWICBitmap>,
    /// Bitmap Direct2D + generasi render target pembuatnya.
    thumb: Option<(u64, ID2D1Bitmap)>,

    settings: SharedSettings,
    lyrics: LyricsService,
    lyrics_state: Option<State>,
    /// Lagu dan durasi yang terakhir diminta liriknya.
    requested: Option<(String, f64)>,
    /// Kotak setiap baris lirik yang tampil + waktunya, untuk klik-lompat.
    line_hits: Vec<(Rect, f64)>,
    ctl_hits: Vec<(Rect, LyricsCtl)>,
    /// Geser waktu lirik per lagu (detik), diatur pengguna.
    offsets: HashMap<String, f64>,
    last_line: Option<usize>,
    line_anim: Option<Instant>,
    sleep: Option<Sleep>,
    marquee: Option<Marquee>,
    /// Lagu palsu untuk pengembangan (`AMORA_DEMO_LYRIC`); abaikan SMTC.
    demo: bool,
}

/// Baris lirik panjang yang sedang digeser di notch tertutup.
struct Marquee {
    line: usize,
    text: String,
    start: Instant,
    /// Lebar yang tidak muat (DIP).
    over: f32,
    speed: f32,
}

impl Marquee {
    fn offset(&self) -> f32 {
        let t = (self.start.elapsed().as_secs_f32() - MARQUEE_PAUSE).max(0.0);
        (t * self.speed).min(self.over)
    }

    /// Masih bergeser; setelah sampai ujung tidak perlu digambar ulang lagi.
    fn moving(&self) -> bool {
        self.start.elapsed().as_secs_f32() < MARQUEE_PAUSE + self.over / self.speed
    }
}

/// Sleep timer: musik dijeda saat `end`, volume memudar selama `FADE` terakhir.
struct Sleep {
    end: Instant,
    minutes: u32,
    /// Volume sebelum memudar, untuk dikembalikan setelah musik dijeda.
    fade_from: Option<f32>,
    /// Musik sudah dijeda; kembalikan volume pada waktu ini.
    restore_at: Option<Instant>,
}

impl Drop for MediaModule {
    /// Amora ditutup di tengah volume memudar: kembalikan volumenya.
    fn drop(&mut self) {
        if let Some(Sleep { fade_from: Some(v), .. }) = self.sleep.take() {
            crate::system::set_master_volume(v);
        }
    }
}

const SLEEP_PRESETS: [u32; 3] = [15, 30, 60];
const FADE: Duration = Duration::from_secs(10);
const FADE_STEP_MS: u32 = 400;

struct Layout {
    art: Rect,
    title: Rect,
    artist: Rect,
    prev: Rect,
    play: Rect,
    next: Rect,
    lyr: Rect,
    sleep: Rect,
    bar: Rect,
    times: Rect,
}

/// `r` adalah area pemutar (bagian kiri notch).
fn layout(r: Rect) -> Layout {
    let art = Rect::new(r.x + 16.0, r.y + 14.0, 72.0, 72.0);
    let tx = art.x + art.w + 14.0;
    let tw = r.x + r.w - 16.0 - tx;
    let by = r.y + 72.0;
    let btn = |i: f32| Rect::new(tx - 4.0 + i * 40.0, by - 15.0, 30.0, 30.0);
    Layout {
        art,
        title: Rect::new(tx, r.y + 16.0, tw, 22.0),
        artist: Rect::new(tx, r.y + 38.0, tw, 18.0),
        prev: btn(0.0),
        play: btn(1.0),
        next: btn(2.0),
        lyr: Rect::new(r.x + r.w - 16.0 - 30.0, by - 15.0, 30.0, 30.0),
        sleep: Rect::new(r.x + r.w - 16.0 - 30.0 - 84.0, by - 15.0, 80.0, 30.0),
        bar: Rect::new(r.x + 16.0, r.y + 100.0, r.w - 32.0, 4.0),
        times: Rect::new(r.x + 16.0, r.y + 108.0, r.w - 32.0, 16.0),
    }
}

fn mmss(sec: f64) -> String {
    let s = sec.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn track_key(i: &MediaInfo) -> String {
    format!("{}\u{1}{}", i.title, i.artist)
}

/// Indeks baris yang sedang dinyanyikan, `None` sebelum baris pertama.
fn current_line(lines: &[Line], pos: f64) -> Option<usize> {
    let p = pos + LEAD;
    lines.iter().rposition(|l| l.t <= p)
}

impl MediaModule {
    pub fn new(
        shared: Shared,
        cmd: MediaHandle,
        wic: IWICImagingFactory,
        lyrics: LyricsService,
        settings: SharedSettings,
    ) -> Self {
        let mut m = Self {
            shared,
            cmd,
            wic,
            info: None,
            thumb_id: 0,
            thumb_src: None,
            thumb: None,
            settings,
            lyrics,
            lyrics_state: None,
            requested: None,
            line_hits: Vec::new(),
            ctl_hits: Vec::new(),
            offsets: load_json(OFFSETS_FILE).unwrap_or_default(),
            last_line: None,
            line_anim: None,
            sleep: None,
            marquee: None,
            demo: false,
        };
        // Untuk pengembangan: `AMORA_DEMO_LYRIC="<baris>"` memutar lagu palsu dengan baris
        // lirik itu (8 detik), lalu satu baris pendek. Untuk menguji tampilan lirik tanpa musik.
        if let Ok(text) = std::env::var("AMORA_DEMO_LYRIC") {
            m.demo = true;
            m.info = Some(MediaInfo {
                title: "Lagu Uji".into(),
                artist: "Amora".into(),
                playing: true,
                position: 0.0,
                duration: 200.0,
                at: Instant::now(),
                thumb: None,
                thumb_id: 0,
            });
            let lines = vec![Line { t: 0.0, text }, Line { t: 8.0, text: "Baris pendek".into() }];
            m.lyrics_state = Some(State::Ready(Arc::new(Lyrics::Synced(lines))));
        }
        m
    }

    fn lyrics_on(&self) -> bool {
        self.settings.borrow().lyrics
    }

    fn synced(&self) -> Option<&[Line]> {
        match &self.lyrics_state {
            Some(State::Ready(l)) => match l.as_ref() {
                Lyrics::Synced(lines) => Some(lines),
                _ => None,
            },
            _ => None,
        }
    }

    /// Minta lirik bila lagu berganti, atau bila durasi baru diketahui setelah gagal.
    fn maybe_request(&mut self) {
        if !self.lyrics_on() {
            return;
        }
        let Some(info) = &self.info else { return };
        let key = track_key(info);
        let need = match &self.requested {
            None => true,
            Some((k, _)) if *k != key => true,
            Some((_, d)) => {
                *d <= 0.0
                    && info.duration > 0.0
                    && matches!(&self.lyrics_state, Some(State::Ready(l)) if matches!(**l, Lyrics::NotFound))
            }
        };
        if need {
            self.lyrics.request(&key, &info.title, &info.artist, info.duration);
            self.requested = Some((key, info.duration));
        }
    }

    fn pull_lyrics(&mut self) {
        self.lyrics_state = self
            .info
            .as_ref()
            .and_then(|i| self.lyrics.get(&track_key(i)));
    }

    fn art(&mut self, g: &mut Gfx, r: Rect, radius: f32) {
        if let Some(src) = &self.thumb_src {
            if self.thumb.as_ref().map(|t| t.0) != Some(g.generation) {
                self.thumb = g.bitmap(src).map(|b| (g.generation, b));
            }
        }
        match &self.thumb {
            Some((_, bmp)) => g.image(bmp, r, radius),
            None => {
                g.fill_rrect(r, radius, FAINT);
                g.text("♪", r, r.h * 0.45, 400, DIM, Align::Center);
            }
        }
    }

    /// Geser waktu lirik lagu ini (detik); disimpan per lagu.
    fn offset(&self) -> f64 {
        self.info.as_ref().and_then(|i| self.offsets.get(&track_key(i))).copied().unwrap_or(0.0)
    }

    fn draw_lyrics(&mut self, g: &mut Gfx, r: Rect, pos: f64) {
        self.line_hits.clear();
        self.ctl_hits.clear();
        let area = Rect::new(r.x, r.y, r.w, r.h - 24.0);
        let bar = Rect::new(r.x, r.y + r.h - 20.0, r.w, 20.0);

        let research = Rect::new(bar.x, bar.y, 96.0, bar.h);
        g.text("↻ Cari ulang", research, 11.5, 400, DIM, Align::Left);
        self.ctl_hits.push((research, LyricsCtl::Research));

        let msg = match &self.lyrics_state {
            None | Some(State::Searching) => Some("Mencari lirik…"),
            Some(State::Ready(l)) => match l.as_ref() {
                Lyrics::Synced(_) => None,
                Lyrics::Unsynced => Some("Lirik ada, tapi belum tersinkron"),
                Lyrics::Instrumental => Some("Instrumental ♪"),
                Lyrics::NotFound => Some("Lirik tidak ditemukan"),
            },
        };
        if let Some(m) = msg {
            g.text(m, area, 12.5, 400, DIM, Align::Left);
            return;
        }
        let Some(lines) = self.synced().map(|l| l.to_vec()) else { return };

        // Geser waktu: − 0,0 dtk +
        let off = self.offset();
        let plus = Rect::new(bar.x + bar.w - 20.0, bar.y, 20.0, bar.h);
        let minus = Rect::new(plus.x - 70.0, bar.y, 20.0, bar.h);
        g.fill_rrect(minus, 5.0, FAINT);
        g.fill_rrect(plus, 5.0, FAINT);
        g.text("−", minus, 12.0, 600, WHITE, Align::Center);
        g.text("+", plus, 12.0, 600, WHITE, Align::Center);
        let label = format!("{:+.1} dtk", off).replace('.', ",");
        let lc = if off == 0.0 { DIM } else { ACCENT };
        g.text(&label, Rect::new(minus.x + minus.w, bar.y, plus.x - minus.x - minus.w, bar.h), 11.0, 600, lc, Align::Center);
        self.ctl_hits.push((minus, LyricsCtl::Shift(-OFFSET_STEP)));
        self.ctl_hits.push((plus, LyricsCtl::Shift(OFFSET_STEP)));

        let cur = current_line(&lines, pos + off);
        if cur != self.last_line {
            self.last_line = cur;
            self.line_anim = Some(Instant::now());
        }
        // Baris baru naik pelan dari bawah (ease-out).
        let t = self.line_anim.map_or(1.0, |a| (a.elapsed().as_secs_f32() / LINE_ANIM.as_secs_f32()).min(1.0));
        let shift = (1.0 - t).powi(3) * ROW_H;

        let c = cur.unwrap_or(0);
        let text_of = |i: usize| if lines[i].text.is_empty() { "♪" } else { lines[i].text.as_str() };
        // Baris yang sedang dinyanyikan boleh dua baris, supaya tidak terpotong.
        let n = if cur.is_some() { g.line_count(text_of(c), area.w, 14.0, 600).min(2) } else { 1 };
        let cur_h = n as f32 * 18.0 + 6.0;
        let cur_y = area.y + area.h / 2.0 - cur_h / 2.0 + shift;

        g.push_clip(area);
        let cur_rect = Rect::new(area.x, cur_y, area.w, cur_h);
        if cur.is_some() {
            g.text_wrapped(text_of(c), cur_rect, 14.0, 600, WHITE, Align::Left);
        } else {
            g.text(text_of(c), cur_rect, 12.5, 400, DIM, Align::Left);
        }
        self.line_hits.push((cur_rect, lines[c].t));
        for d in 1..=2usize {
            let fade = if d == 1 { DIM } else { Color(1.0, 1.0, 1.0, 0.3) };
            if let Some(i) = c.checked_sub(d) {
                let rect = Rect::new(area.x, cur_y - d as f32 * ROW_H, area.w, ROW_H);
                g.text(text_of(i), rect, 12.5, 400, fade, Align::Left);
                self.line_hits.push((rect, lines[i].t));
            }
            let i = c + d;
            if i < lines.len() {
                let rect = Rect::new(area.x, cur_y + cur_h + (d - 1) as f32 * ROW_H, area.w, ROW_H);
                g.text(text_of(i), rect, 12.5, 400, fade, Align::Left);
                self.line_hits.push((rect, lines[i].t));
            }
        }
        g.pop_clip();
    }
}

fn icon_play(g: &Gfx, c: (f32, f32), playing: bool) {
    let (x, y) = c;
    if playing {
        g.fill_rrect(Rect::new(x - 7.0, y - 8.0, 5.0, 16.0), 1.5, WHITE);
        g.fill_rrect(Rect::new(x + 2.0, y - 8.0, 5.0, 16.0), 1.5, WHITE);
    } else {
        g.polygon(&[(x - 5.0, y - 9.0), (x - 5.0, y + 9.0), (x + 9.0, y)], WHITE);
    }
}

fn icon_skip(g: &Gfx, c: (f32, f32), forward: bool) {
    let (x, y) = c;
    let d = if forward { 1.0 } else { -1.0 };
    g.polygon(&[(x - 6.0 * d, y - 7.0), (x - 6.0 * d, y + 7.0), (x + 4.0 * d, y)], WHITE);
    let bx = if forward { x + 4.0 } else { x - 7.0 };
    g.fill_rect(Rect::new(bx, y - 7.0, 3.0, 14.0), WHITE);
}

/// Ikon lirik: tiga garis teks.
fn icon_lyrics(g: &Gfx, c: (f32, f32), on: bool) {
    let (x, y) = c;
    let col = if on { ACCENT } else { DIM };
    for (i, w) in [14.0, 10.0, 12.0].iter().enumerate() {
        g.fill_rrect(Rect::new(x - 7.0, y - 6.0 + i as f32 * 5.0, *w, 2.5), 1.2, col);
    }
}

fn center(r: Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

impl Module for MediaModule {
    fn tab(&self) -> Option<&'static str> {
        Some("Musik")
    }

    fn active(&self) -> bool {
        self.info.is_some()
    }

    fn wants_space(&self) -> bool {
        let Some(info) = &self.info else { return false };
        self.lyrics_on() && self.synced().is_some_and(|l| current_line(l, info.position_now() + self.offset()).is_some())
    }

    fn compact_size(&self) -> (f32, f32) {
        if self.lyrics_on() && self.synced().is_some() {
            (380.0, 32.0)
        } else {
            (240.0, 32.0)
        }
    }

    fn expanded_size(&self) -> (f32, f32) {
        if self.info.is_none() {
            (420.0, 80.0)
        } else if self.lyrics_on() {
            (PLAYER_W + LYRICS_W, HEIGHT)
        } else {
            (PLAYER_W, HEIGHT)
        }
    }

    fn draw_compact(&mut self, g: &mut Gfx, r: Rect) {
        let Some(info) = self.info.clone() else { return };
        let s = 20.0;
        self.art(g, Rect::new(r.x + 10.0, r.y + (r.h - s) / 2.0, s, s), 5.0);
        // Bar diam (bukan animasi) supaya CPU tetap 0 selama notch tertutup.
        let x = r.x + r.w - 26.0;
        let cy = r.y + r.h / 2.0;
        let color = if info.playing { ACCENT } else { DIM };
        for (i, h) in [8.0, 14.0, 10.0].iter().enumerate() {
            let h = if info.playing { *h } else { 4.0 };
            g.fill_rrect(Rect::new(x + i as f32 * 5.0, cy - h / 2.0, 3.0, h), 1.5, color);
        }
        // Baris lirik saat ini, di tengah notch yang tertutup.
        let line = self.lyrics_on().then(|| self.synced()).flatten().and_then(|lines| {
            let i = current_line(lines, info.position_now() + self.offset())?;
            let text = if lines[i].text.is_empty() { "♪".to_string() } else { lines[i].text.clone() };
            let dur = lines.get(i + 1).map_or(0.0, |n| (n.t - lines[i].t) as f32);
            Some((i, text, dur))
        });
        let Some((i, text, dur)) = line else {
            self.marquee = None;
            return;
        };
        let tr = Rect::new(r.x + 40.0, r.y, r.w - 80.0, r.h);
        let tw = g.text_width(&text, 12.5, 600);
        if tw <= tr.w {
            self.marquee = None;
            g.text(&text, tr, 12.5, 600, WHITE, Align::Center);
            return;
        }
        if self.marquee.as_ref().is_none_or(|m| m.line != i || m.text != text) {
            let over = tw - tr.w;
            // Usahakan sampai ujung sebelum baris berikutnya mulai.
            let time = dur * 0.8 - MARQUEE_PAUSE;
            let speed = if time > 0.1 { (over / time).clamp(MARQUEE_SPEED, MARQUEE_MAX_SPEED) } else { MARQUEE_SPEED };
            self.marquee = Some(Marquee { line: i, text: text.clone(), start: Instant::now(), over, speed });
        }
        let dx = self.marquee.as_ref().map_or(0.0, Marquee::offset);
        g.push_clip(tr);
        g.text(&text, Rect::new(tr.x - dx, tr.y, tw + 8.0, tr.h), 12.5, 600, WHITE, Align::Left);
        g.pop_clip();
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        let Some(info) = self.info.clone() else {
            g.text("Tidak ada musik yang diputar", r, 13.0, 400, DIM, Align::Center);
            return;
        };
        let player = Rect::new(r.x, r.y, PLAYER_W.min(r.w), r.h);
        let l = layout(player);
        self.art(g, l.art, 12.0);

        let title = if info.title.is_empty() { "Tanpa judul" } else { &info.title };
        g.text(title, l.title, 15.0, 600, WHITE, Align::Left);
        g.text(&info.artist, l.artist, 12.5, 400, DIM, Align::Left);

        icon_skip(g, center(l.prev), false);
        icon_play(g, center(l.play), info.playing);
        icon_skip(g, center(l.next), true);
        icon_lyrics(g, center(l.lyr), self.lyrics_on());
        match &self.sleep {
            Some(s) if s.restore_at.is_none() => {
                let left = s.end.saturating_duration_since(Instant::now()).as_secs();
                let t = format!("☾ {}:{:02}", left / 60, left % 60);
                g.text(&t, l.sleep, 12.0, 600, ACCENT, Align::Right);
            }
            _ => g.text("☾ Tidur", l.sleep, 12.0, 400, DIM, Align::Right),
        }

        let pos = info.position_now();
        if info.duration > 0.0 {
            let k = (pos / info.duration).clamp(0.0, 1.0) as f32;
            g.fill_rrect(l.bar, 2.0, FAINT);
            g.fill_rrect(Rect::new(l.bar.x, l.bar.y, l.bar.w * k, l.bar.h), 2.0, WHITE);
            g.text(&mmss(pos), l.times, 11.0, 400, DIM, Align::Left);
            let left = format!("-{}", mmss(info.duration - pos));
            g.text(&left, l.times, 11.0, 400, DIM, Align::Right);
        }

        if self.lyrics_on() {
            let lr = Rect::new(r.x + PLAYER_W, r.y + 8.0, LYRICS_W - 20.0, r.h - 16.0);
            self.draw_lyrics(g, lr, pos);
        } else {
            self.line_hits.clear();
        }
    }

    fn click(&mut self, r: Rect, x: f32, y: f32) -> bool {
        let Some(info) = &self.info else { return false };
        let duration = info.duration;
        let l = layout(Rect::new(r.x, r.y, PLAYER_W.min(r.w), r.h));
        if l.play.contains(x, y) {
            self.cmd.send(MediaCmd::PlayPause);
        } else if l.next.contains(x, y) {
            self.cmd.send(MediaCmd::Next);
        } else if l.prev.contains(x, y) {
            self.cmd.send(MediaCmd::Prev);
        } else if l.sleep.contains(x, y) {
            // Mati → 15 → 30 → 60 menit → mati.
            let next = match &self.sleep {
                None => Some(SLEEP_PRESETS[0]),
                Some(s) => SLEEP_PRESETS.iter().copied().find(|&m| m > s.minutes),
            };
            if let Some(Sleep { fade_from: Some(v), .. }) = self.sleep.take() {
                crate::system::set_master_volume(v);
                crate::system::set_quiet(false);
            }
            self.sleep = next.map(|m| Sleep {
                end: Instant::now() + Duration::from_secs(m as u64 * 60),
                minutes: m,
                fade_from: None,
                restore_at: None,
            });
        } else if l.lyr.contains(x, y) {
            {
                let mut s = self.settings.borrow_mut();
                s.lyrics = !s.lyrics;
                s.save();
            }
            self.maybe_request();
            self.pull_lyrics();
        } else if duration > 0.0 && l.bar.inflate(8.0).contains(x, y) {
            let k = ((x - l.bar.x) / l.bar.w).clamp(0.0, 1.0) as f64;
            self.cmd.send(MediaCmd::Seek(k * duration));
        } else if let Some((_, ctl)) = self.ctl_hits.iter().find(|(r, _)| r.contains(x, y)) {
            let Some(info) = &self.info else { return false };
            let key = track_key(info);
            match ctl {
                LyricsCtl::Research => {
                    self.lyrics.request_fresh(&key, &info.title, &info.artist, info.duration);
                    self.requested = Some((key, info.duration));
                }
                LyricsCtl::Shift(d) => {
                    let v = self.offsets.entry(key).or_insert(0.0);
                    *v = ((*v + d) * 10.0).round() / 10.0;
                    *v = v.clamp(-10.0, 10.0);
                    self.offsets.retain(|_, v| *v != 0.0);
                    save_json(OFFSETS_FILE, &self.offsets);
                }
            }
        } else if let Some((_, t)) = self.line_hits.iter().find(|(lr, _)| lr.contains(x, y)) {
            self.cmd.send(MediaCmd::Seek(*t));
        } else {
            return false;
        }
        true
    }

    fn on_event(&mut self, e: &Event) {
        if self.demo {
            return;
        }
        match e {
            Event::MediaChanged => {
                self.info = self.shared.lock().unwrap().clone();
                if let Some(info) = &self.info {
                    if info.thumb_id != self.thumb_id {
                        self.thumb_id = info.thumb_id;
                        self.thumb = None;
                        self.thumb_src = info.thumb.as_ref().and_then(|b| decode_image(&self.wic, b, 160));
                    }
                }
                self.pull_lyrics();
                self.maybe_request();
            }
            Event::LyricsChanged => self.pull_lyrics(),
            // Lirik mungkin baru dinyalakan dari halaman Setelan.
            Event::SettingsChanged => {
                self.maybe_request();
                self.pull_lyrics();
            }
            _ => {}
        }
    }

    fn poll_in(&self) -> Option<u32> {
        let s = self.sleep.as_ref()?;
        let now = Instant::now();
        Some(match s.restore_at {
            Some(t) => t.saturating_duration_since(now).as_millis() as u32 + 5,
            None => {
                let left = s.end.saturating_duration_since(now);
                if left > FADE { (left - FADE).as_millis() as u32 + 5 } else { FADE_STEP_MS }
            }
        })
    }

    fn poll(&mut self) -> Poll {
        let Some(s) = &mut self.sleep else { return Poll::Idle };
        let now = Instant::now();
        if let Some(t) = s.restore_at {
            if now >= t {
                if let Some(v) = s.fade_from {
                    crate::system::set_master_volume(v);
                }
                crate::system::set_quiet(false);
                self.sleep = None;
                return Poll::Redraw;
            }
            return Poll::Idle;
        }
        let left = s.end.saturating_duration_since(now);
        if left > FADE {
            return Poll::Idle;
        }
        if s.fade_from.is_none() {
            crate::system::set_quiet(true);
            s.fade_from = crate::system::master_volume();
        }
        let from = s.fade_from.unwrap_or(0.0);
        crate::system::set_master_volume(from * (left.as_secs_f32() / FADE.as_secs_f32()));
        if left.is_zero() {
            self.cmd.send(MediaCmd::Pause);
            // Tunggu musik benar-benar berhenti sebelum volume dikembalikan.
            s.restore_at = Some(now + Duration::from_millis(800));
        }
        Poll::Redraw
    }

    fn tick_ms(&self, expanded: bool) -> Option<u32> {
        // Selama baris lirik sedang bergeser naik.
        if expanded && self.lyrics_on() && self.line_anim.is_some_and(|a| a.elapsed() < LINE_ANIM) {
            return Some(16);
        }
        if expanded && self.sleep.is_some() {
            return Some(500);
        }
        // Lirik panjang sedang bergeser; berhenti menggambar begitu sampai ujung.
        if !expanded && self.marquee.as_ref().is_some_and(Marquee::moving) {
            return Some(MARQUEE_FRAME_MS);
        }
        let info = self.info.as_ref().filter(|i| i.playing)?;
        // Bangun tepat saat baris lirik berikutnya mulai.
        let next_line = self.lyrics_on().then(|| self.synced()).flatten().and_then(|lines| {
            let p = info.position_now() + self.offset() + LEAD;
            let t = lines.iter().find(|l| l.t > p)?.t;
            Some((((t - p) * 1000.0) as u32).clamp(30, 5000))
        });
        match (expanded, next_line) {
            (true, Some(ms)) => Some(ms.min(500)),
            (true, None) => Some(500),
            (false, Some(ms)) => Some(ms),
            (false, None) => None,
        }
    }
}
