//! Lirik tersinkron dari LRCLIB, dicari di thread terpisah.
//! Yang dikirim ke lrclib.net hanya judul, artis, dan durasi lagu.

mod http;
pub mod matcher;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

pub use matcher::Line;
use http::Http;
use matcher::{candidates, free_query, parse_lrc, title_distance, Query};

const HOST: &str = "lrclib.net";
const API: &str = "/api";
const USER_AGENT: &str = concat!("Leo/", env!("CARGO_PKG_VERSION"));

#[derive(Debug)]
pub enum Lyrics {
    Synced(Vec<Line>),
    /// Ada lirik, tapi tanpa waktu per baris.
    Unsynced,
    Instrumental,
    NotFound,
}

#[derive(Clone)]
pub enum State {
    Searching,
    Ready(Arc<Lyrics>),
}

struct Req {
    /// Abaikan cache dan cari lagi ke LRCLIB.
    fresh: bool,
    key: String,
    title: String,
    artist: String,
    duration: f64,
}

type Shared = Arc<Mutex<Option<(String, State)>>>;

#[derive(Clone)]
pub struct LyricsService {
    tx: Sender<Req>,
    shared: Shared,
}

impl LyricsService {
    pub fn spawn(hwnd: HWND, notify_msg: u32) -> Self {
        let shared: Shared = Arc::new(Mutex::new(None));
        let (tx, rx) = channel();
        let hwnd_raw = hwnd.0 as isize;
        {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let notify = move || unsafe {
                    let _ = PostMessageW(HWND(hwnd_raw as _), notify_msg, WPARAM(0), LPARAM(0));
                };
                run(&shared, rx, &notify);
            });
        }
        Self { tx, shared }
    }

    pub fn request(&self, key: &str, title: &str, artist: &str, duration: f64) {
        self.send(false, key, title, artist, duration);
    }

    /// Cari ulang walau sudah ada di cache (mis. lirik salah atau tadi tidak ketemu).
    pub fn request_fresh(&self, key: &str, title: &str, artist: &str, duration: f64) {
        self.send(true, key, title, artist, duration);
    }

    fn send(&self, fresh: bool, key: &str, title: &str, artist: &str, duration: f64) {
        let _ = self.tx.send(Req {
            fresh,
            key: key.into(),
            title: title.into(),
            artist: artist.into(),
            duration,
        });
    }

    /// Status lirik untuk `key`, atau `None` kalau belum pernah diminta.
    pub fn get(&self, key: &str) -> Option<State> {
        let g = self.shared.lock().unwrap();
        g.as_ref().filter(|(k, _)| k == key).map(|(_, s)| s.clone())
    }
}

fn run(shared: &Shared, rx: Receiver<Req>, notify: &dyn Fn()) {
    // Dibuat saat pertama kali dibutuhkan, supaya tanpa lirik tidak ada DLL jaringan yang dimuat.
    let mut client: Option<Http> = None;
    let mut memo: HashMap<String, Arc<Lyrics>> = HashMap::new();

    while let Ok(mut req) = rx.recv() {
        // Lagu bisa diganti cepat; cukup kerjakan permintaan terakhir.
        while let Ok(newer) = rx.try_recv() {
            req = newer;
        }
        let set = |s: State| *shared.lock().unwrap() = Some((req.key.clone(), s));

        let cached = if req.fresh {
            None
        } else {
            memo.get(&req.key).cloned().or_else(|| disk_load(&req.key).map(Arc::new))
        };
        let found = match cached {
            // "Tidak ketemu" dicoba lagi kalau durasi baru diketahui.
            Some(l) if !matches!(*l, Lyrics::NotFound) || req.duration <= 0.0 => l,
            _ => {
                set(State::Searching);
                notify();
                if client.is_none() {
                    client = Http::new(HOST, USER_AGENT);
                }
                let l = match &client {
                    Some(c) => find(c, &req.title, &req.artist, req.duration),
                    None => Lyrics::NotFound,
                };
                disk_save(&req.key, &l);
                Arc::new(l)
            }
        };
        memo.insert(req.key.clone(), found.clone());
        set(State::Ready(found));
        notify();
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Record {
    track_name: Option<String>,
    duration: Option<f64>,
    instrumental: Option<bool>,
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
}

impl Record {
    fn to_lyrics(&self) -> Lyrics {
        if let Some(s) = self.synced_lyrics.as_deref().filter(|s| !s.trim().is_empty()) {
            let lines = parse_lrc(s);
            if !lines.is_empty() {
                return Lyrics::Synced(lines);
            }
        }
        if self.instrumental == Some(true) {
            Lyrics::Instrumental
        } else if self.plain_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty()) {
            Lyrics::Unsynced
        } else {
            Lyrics::NotFound
        }
    }
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn get_exact(c: &Http, q: &Query, duration: f64) -> Option<Record> {
    let url = format!(
        "{API}/get?track_name={}&artist_name={}&duration={}",
        enc(&q.track),
        enc(&q.artist),
        duration.round() as i64
    );
    let body = c.get(&url)?;
    serde_json::from_str(&body).ok()
}

fn search(c: &Http, params: &str) -> Vec<Record> {
    let Some(body) = c.get(&format!("{API}/search?{params}")) else {
        return Vec::new();
    };
    serde_json::from_str(&body).unwrap_or_default()
}

/// Pilih hasil terbaik: durasinya dalam `tol` detik, judulnya mirip, utamakan yang tersinkron.
fn best<'a>(recs: &'a [Record], want_title: &str, duration: f64, tol: f64, exact_title: bool) -> Option<&'a Record> {
    recs.iter()
        .filter(|r| duration <= 0.0 || r.duration.is_some_and(|d| (d - duration).abs() <= tol))
        .filter(|r| !exact_title || title_distance(want_title, r.track_name.as_deref().unwrap_or("")) == 0)
        .filter(|r| plausible(&r.to_lyrics()))
        .min_by_key(|r| {
            let synced = if r.synced_lyrics.as_deref().is_some_and(|s| !s.is_empty()) { 0 } else { 100 };
            let dur = r.duration.map(|d| (d - duration).abs() as u32).unwrap_or(5);
            let name = title_distance(want_title, r.track_name.as_deref().unwrap_or(""));
            synced + name * 4 + if duration > 0.0 { dur } else { 0 }
        })
}

/// LRCLIB diisi komunitas; buang entri sampah (mis. lirik tersinkron 1 baris).
fn plausible(l: &Lyrics) -> bool {
    match l {
        Lyrics::Synced(lines) => lines.iter().filter(|l| !l.text.is_empty()).count() >= 5,
        Lyrics::NotFound => false,
        _ => true,
    }
}

/// Urutan pencarian, dari yang paling tepat sampai paling longgar.
pub fn find(c: &Http, title: &str, artist: &str, duration: f64) -> Lyrics {
    let cands = candidates(title, artist);
    let want = cands.first().map(|q| q.track.clone()).unwrap_or_else(|| title.to_string());
    let mut fallback: Option<Lyrics> = None;
    let mut keep = |l: Lyrics| -> Option<Lyrics> {
        if !plausible(&l) {
            return None;
        }
        match l {
            Lyrics::Unsynced => {
                fallback.get_or_insert(Lyrics::Unsynced);
                None
            }
            _ => Some(l),
        }
    };
    let mut seen: Vec<Record> = Vec::new();

    // 1) Cocok persis (butuh durasi).
    if duration > 0.0 {
        for q in &cands {
            if let Some(l) = get_exact(c, q, duration).map(|r| r.to_lyrics()).and_then(&mut keep) {
                return l;
            }
        }
    }
    // 2) Pencarian per kandidat, lalu 3) teks bebas.
    let mut searches: Vec<(String, String)> = cands
        .iter()
        .map(|q| (format!("track_name={}&artist_name={}", enc(&q.track), enc(&q.artist)), q.track.clone()))
        .collect();
    searches.push((format!("q={}", enc(&free_query(title, artist))), want.clone()));
    for (params, track) in searches {
        let recs = search(c, &params);
        if let Some(l) = best(&recs, &track, duration, 5.0, false).map(|r| r.to_lyrics()).and_then(&mut keep) {
            return l;
        }
        seen.extend(recs);
    }
    if duration > 0.0 {
        // 4) Nama channel sering bukan nama artis: cari judul saja, durasi harus sangat dekat.
        let recs = search(c, &format!("track_name={}", enc(&want)));
        if let Some(l) = best(&recs, &want, duration, 3.0, true).map(|r| r.to_lyrics()).and_then(&mut keep) {
            return l;
        }
        seen.extend(recs);
        // 5) Video klip sering lebih panjang dari lagunya: longgarkan durasi, judul harus sama.
        if let Some(l) = best(&seen, &want, duration, 30.0, true).map(|r| r.to_lyrics()).and_then(&mut keep) {
            return l;
        }
    }
    fallback.unwrap_or(Lyrics::NotFound)
}

/// Cetak hasil pencarian untuk satu lagu (tanpa cache).
pub fn probe(title: &str, artist: &str, duration: f64) -> windows::core::Result<()> {
    let c = Http::new(HOST, USER_AGENT).ok_or_else(windows::core::Error::from_win32)?;
    println!("kandidat: {:?}", candidates(title, artist));
    match find(&c, title, artist, duration) {
        Lyrics::Synced(l) => println!("TERSINKRON, {} baris; baris 1: {:?}", l.len(), l.first()),
        other => println!("{other:?}"),
    }
    Ok(())
}

// ---- cache di disk: %LOCALAPPDATA%\Leo\lyrics ----

fn cache_path(key: &str) -> Option<PathBuf> {
    // FNV-1a supaya nama file pendek dan aman.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in key.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    Some(crate::settings::data_dir()?.join("lyrics").join(format!("{h:016x}.lrc")))
}

fn disk_load(key: &str) -> Option<Lyrics> {
    let s = std::fs::read_to_string(cache_path(key)?).ok()?;
    let (kind, body) = s.split_once('\n').unwrap_or((&s, ""));
    Some(match kind {
        "S" => Lyrics::Synced(parse_lrc(body)),
        "U" => Lyrics::Unsynced,
        "I" => Lyrics::Instrumental,
        _ => return None,
    })
}

fn disk_save(key: &str, l: &Lyrics) {
    let Some(path) = cache_path(key) else { return };
    let body = match l {
        Lyrics::Synced(lines) => {
            let mut s = String::from("S\n");
            for ln in lines {
                let t = ln.t.max(0.0);
                s.push_str(&format!("[{:02}:{:05.2}]{}\n", (t / 60.0) as u32, t % 60.0, ln.text));
            }
            s
        }
        Lyrics::Unsynced => "U\n".into(),
        Lyrics::Instrumental => "I\n".into(),
        // Jangan simpan "tidak ketemu": siapa tahu besok sudah ada di LRCLIB.
        Lyrics::NotFound => return,
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, body);
}
