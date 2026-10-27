//! Supabase untuk Leo: login Google (alur PKCE lewat alamat loopback), beberapa akun yang bisa
//! ditukar tanpa keluar-masuk, dan pengambilan data makrab/deadline dari database website.
//!
//! Alur login: Leo membuka browser ke `{url}/auth/v1/authorize?provider=google&redirect_to=
//! http://127.0.0.1:53682/callback&...`. Setelah memilih akun Google, Supabase mengarahkan
//! browser ke alamat itu membawa `?code=...`, yang ditukar Leo menjadi sesi.
//! SYARAT (sekali saja, di dashboard Supabase): tambahkan `http://127.0.0.1:53682/callback`
//! di Authentication -> URL Configuration -> Redirect URLs.
//!
//! Akses data memakai aturan keamanan (RLS) yang sudah ada di website: admin melihat semua
//! tugas makrab, anggota hanya tugasnya sendiri. Token disimpan di `accounts.json`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::settings::{load_json, save_json, SupabaseCfg};

pub const CALLBACK_PORT: u16 = 53682;

pub fn redirect_uri() -> String {
    format!("http://127.0.0.1:{CALLBACK_PORT}/callback")
}

pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// ───────────────────────── akun & sesi ─────────────────────────

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct Account {
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Waktu access token kedaluwarsa (detik sejak epoch).
    pub expires_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Accounts {
    pub active: Option<String>,
    pub list: Vec<Account>,
}

impl Accounts {
    pub fn load() -> Self {
        load_json("accounts.json").unwrap_or_default()
    }

    pub fn save(&self) {
        save_json("accounts.json", self);
    }

    /// Tambah atau perbarui akun, lalu jadikan aktif.
    pub fn upsert(&mut self, acc: Account) {
        self.active = Some(acc.email.clone());
        self.replace_or_push(acc);
    }

    /// Perbarui token akun yang sudah ada tanpa mengubah akun aktif.
    pub fn update_tokens(&mut self, acc: Account) {
        if self.list.iter().any(|a| a.email == acc.email) {
            self.replace_or_push(acc);
        }
    }

    fn replace_or_push(&mut self, acc: Account) {
        match self.list.iter_mut().find(|a| a.email == acc.email) {
            Some(slot) => *slot = acc,
            None => self.list.push(acc),
        }
    }

    pub fn active_account(&self) -> Option<&Account> {
        let email = self.active.as_deref()?;
        self.list.iter().find(|a| a.email == email)
    }

    pub fn set_active(&mut self, email: &str) -> bool {
        let found = self.list.iter().any(|a| a.email == email);
        if found {
            self.active = Some(email.to_string());
        }
        found
    }

    /// Keluar dari satu akun; bila itu yang aktif, pindah ke akun pertama yang tersisa.
    pub fn remove(&mut self, email: &str) {
        self.list.retain(|a| a.email != email);
        if self.active.as_deref() == Some(email) {
            self.active = self.list.first().map(|a| a.email.clone());
        }
    }
}

static LOCK: Mutex<()> = Mutex::new(());

/// Baca-ubah-simpan `accounts.json` secara bergiliran (login dan pemantau jalan di thread berbeda).
pub fn with_accounts<R>(f: impl FnOnce(&mut Accounts) -> R) -> R {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut a = Accounts::load();
    let r = f(&mut a);
    a.save();
    r
}

// ───────────────────────── token & galat ─────────────────────────

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
    user: Option<TokenUser>,
}

#[derive(Deserialize)]
struct TokenUser {
    email: Option<String>,
}

/// Urai balasan `/auth/v1/token`. `fallback_email` dipakai bila balasan tidak menyertakan user.
pub fn parse_token(json: &str, now: i64, fallback_email: Option<&str>) -> Result<Account, String> {
    let t: TokenResp = serde_json::from_str(json).map_err(|e| format!("balasan login tidak dikenali: {e}"))?;
    let email = t
        .user
        .and_then(|u| u.email)
        .or_else(|| fallback_email.map(str::to_string))
        .filter(|e| !e.is_empty())
        .ok_or("balasan login tanpa email")?;
    Ok(Account { email, access_token: t.access_token, refresh_token: t.refresh_token, expires_at: now + t.expires_in })
}

/// Pesan galat dari balasan Supabase (`error_description`, `msg`, atau `message`).
pub fn error_message(body: &str, status: u16) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    ["error_description", "msg", "message", "error"]
        .iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
        .map(|s| format!("{s} (HTTP {status})"))
        .unwrap_or_else(|| format!("HTTP {status}"))
}

// ───────────────────────── login Google (PKCE) ─────────────────────────

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn authorize_url(base: &str, redirect: &str, challenge: &str) -> String {
    format!(
        "{}/auth/v1/authorize?provider=google&redirect_to={}&code_challenge={}&code_challenge_method=s256",
        base.trim_end_matches('/'),
        url_encode(redirect),
        challenge
    )
}

/// Ambil `code` dari baris permintaan HTTP callback, mis. `GET /callback?code=abc HTTP/1.1`.
/// `Ok(None)` = bukan callback (mis. favicon); `Err` = Supabase/Google melaporkan galat.
pub fn parse_callback(request_line: &str) -> Result<Option<String>, String> {
    let mut parts = request_line.split_whitespace();
    let (_method, target) = (parts.next(), parts.next().unwrap_or(""));
    let Some(query) = target.strip_prefix("/callback?") else { return Ok(None) };
    let mut code = None;
    let mut error = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "code" => code = Some(url_decode(v)),
            "error_description" => error = Some(url_decode(v)),
            "error" if error.is_none() => error = Some(url_decode(v)),
            _ => {}
        }
    }
    match (code, error) {
        (Some(c), _) if !c.is_empty() => Ok(Some(c)),
        (_, Some(e)) => Err(e),
        _ => Err("callback tanpa kode login".into()),
    }
}

// ───────────────────────── waktu ─────────────────────────

/// Hari sejak 1970-01-01 untuk tanggal kalender (algoritma Howard Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// `2026-11-02T10:00:00.123+07:00` / `...Z` -> detik sejak epoch (UTC).
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    if s.len() < 19 || s.as_bytes()[4] != b'-' || !matches!(s.as_bytes()[10], b'T' | b' ') {
        return None;
    }
    let (y, mo, d, h, mi, sec) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        rest = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest.as_bytes().first() {
        None | Some(b'Z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let t = &rest[1..];
            let oh: i64 = t.get(0..2)?.parse().ok()?;
            let om: i64 = t.get(3..5).or_else(|| t.get(2..4)).and_then(|x| x.parse().ok()).unwrap_or(0);
            (oh * 3600 + om * 60) * if *sign == b'+' { 1 } else { -1 }
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + sec - offset)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Urgency {
    Overdue,
    /// Kurang dari 3 jam.
    Soon,
    /// Kurang dari 24 jam.
    Today,
    Later,
}

/// Teks sisa waktu bahasa Indonesia dan tingkat mendesaknya.
pub fn relative(deadline: i64, now: i64) -> (String, Urgency) {
    let diff = deadline - now;
    if diff < 0 {
        let late = -diff;
        let text = if late < 3600 {
            format!("telat {} mnt", (late / 60).max(1))
        } else if late < 86400 {
            format!("telat {} jam", late / 3600)
        } else {
            format!("telat {} hari", late / 86400)
        };
        return (text, Urgency::Overdue);
    }
    if diff < 3600 {
        (format!("{} mnt lagi", (diff / 60).max(1)), Urgency::Soon)
    } else if diff < 3 * 3600 {
        (format!("{} jam lagi", diff / 3600), Urgency::Soon)
    } else if diff < 86400 {
        (format!("{} jam lagi", diff / 3600), Urgency::Today)
    } else {
        (format!("{} hari lagi", diff / 86400), Urgency::Later)
    }
}

/// Tingkat peringatan: 0 belum perlu, 1 dalam 24 jam, 2 dalam 3 jam, 3 dalam 1 jam, 4 terlambat.
pub fn alert_level(deadline: i64, now: i64) -> u8 {
    let diff = deadline - now;
    match diff {
        d if d < 0 => 4,
        d if d <= 3600 => 3,
        d if d <= 3 * 3600 => 2,
        d if d <= 86400 => 1,
        _ => 0,
    }
}

// ───────────────────────── data makrab ─────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub judul: String,
    pub kategori: String,
    pub status: String,
    pub deadline: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub email: String,
    /// Tugas yang belum selesai dan punya deadline, terurut paling dekat dulu.
    pub tasks: Vec<Task>,
    pub makrab_total: u32,
    pub makrab_done: u32,
    /// Kiriman yang menunggu ditinjau admin.
    pub review_pending: u32,
    pub fetched_at: i64,
}

#[derive(Deserialize)]
struct TaskRow {
    judul: String,
    #[serde(default)]
    kategori: String,
    #[serde(default)]
    status: String,
    deadline: Option<String>,
}

#[derive(Deserialize)]
struct StatusRow {
    #[serde(default)]
    status: String,
}

pub fn parse_tasks(json: &str) -> Result<Vec<Task>, String> {
    let rows: Vec<TaskRow> = serde_json::from_str(json).map_err(|e| format!("data tugas tidak dikenali: {e}"))?;
    let mut tasks: Vec<Task> = rows
        .into_iter()
        .filter_map(|r| {
            Some(Task { judul: r.judul, kategori: r.kategori, status: r.status, deadline: parse_iso8601(&r.deadline?)? })
        })
        .collect();
    tasks.sort_by_key(|t| t.deadline);
    Ok(tasks)
}

/// (total, selesai) dari daftar status tugas makrab.
pub fn parse_progress(json: &str) -> Result<(u32, u32), String> {
    let rows: Vec<StatusRow> = serde_json::from_str(json).map_err(|e| format!("data progres tidak dikenali: {e}"))?;
    Ok((rows.len() as u32, rows.iter().filter(|r| r.status == "selesai").count() as u32))
}

pub fn count_rows(json: &str) -> Result<u32, String> {
    let v: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| format!("data tidak dikenali: {e}"))?;
    Ok(v.len() as u32)
}

#[cfg(windows)]
mod online {
    use super::*;
    use crate::net;

    fn base(cfg: &SupabaseCfg) -> &str {
        cfg.url.trim_end_matches('/')
    }

    fn token_request(cfg: &SupabaseCfg, grant: &str, body: &str, fallback: Option<&str>) -> Result<Account, String> {
        let url = format!("{}/auth/v1/token?grant_type={grant}", base(cfg));
        let (status, text) =
            net::request("POST", &url, &[("apikey", &cfg.anon_key), ("Content-Type", "application/json")], Some(body))?;
        if status != 200 {
            return Err(error_message(&text, status));
        }
        parse_token(&text, now_unix(), fallback)
    }

    pub fn refresh(cfg: &SupabaseCfg, acc: &Account) -> Result<Account, String> {
        let body = serde_json::json!({ "refresh_token": acc.refresh_token }).to_string();
        token_request(cfg, "refresh_token", &body, Some(&acc.email))
    }

    /// GET ke REST API dengan token akun; menyegarkan token yang hampir/sudah kedaluwarsa.
    /// Mengembalikan teks balasan dan akun (token mungkin baru).
    fn rest_get(cfg: &SupabaseCfg, mut acc: Account, path_query: &str) -> Result<(String, Account), String> {
        if acc.expires_at - 60 < now_unix() {
            acc = refresh(cfg, &acc)?;
        }
        let url = format!("{}/rest/v1/{path_query}", base(cfg));
        for attempt in 0..2 {
            let auth = format!("Bearer {}", acc.access_token);
            let (status, text) = net::request("GET", &url, &[("apikey", &cfg.anon_key), ("Authorization", &auth)], None)?;
            match status {
                200 => return Ok((text, acc)),
                401 if attempt == 0 => acc = refresh(cfg, &acc)?,
                _ => return Err(error_message(&text, status)),
            }
        }
        Err("akses ditolak".into())
    }

    /// Ambil tugas berdeadline, progres makrab, dan kiriman yang menunggu review.
    pub fn fetch_snapshot(cfg: &SupabaseCfg, acc: Account) -> Result<(Snapshot, Account), String> {
        let (t, acc) = rest_get(
            cfg,
            acc,
            "makrab_tasks?select=judul,kategori,status,deadline&deadline=not.is.null&status=neq.selesai&order=deadline.asc&limit=40",
        )?;
        let (p, acc) = rest_get(cfg, acc, "makrab_tasks?select=status&kategori=eq.makrab&parent_id=is.null&limit=1000")?;
        let (r, acc) = rest_get(cfg, acc, "makrab_submissions?select=id&status=eq.menunggu&limit=1000")?;
        let (makrab_total, makrab_done) = parse_progress(&p)?;
        let snap = Snapshot {
            email: acc.email.clone(),
            tasks: parse_tasks(&t)?,
            makrab_total,
            makrab_done,
            review_pending: count_rows(&r)?,
            fetched_at: now_unix(),
        };
        Ok((snap, acc))
    }

    pub fn login_google(cfg: &SupabaseCfg) -> Result<Account, String> {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        if !cfg.configured() {
            return Err("Supabase belum diatur (isi anon_key di settings.json)".into());
        }
        let verifier = crate::crypto::pkce_verifier();
        let challenge = crate::crypto::pkce_challenge(&verifier);
        let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
            .map_err(|e| format!("port {CALLBACK_PORT} tidak bisa dipakai: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        if !crate::ai::open_url(&authorize_url(&cfg.url, &redirect_uri(), &challenge)) {
            return Err("browser tidak bisa dibuka".into());
        }

        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            if Instant::now() > deadline {
                return Err("waktu login habis (3 menit)".into());
            }
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let line = request.lines().next().unwrap_or("");
            let parsed = parse_callback(line);
            let page = match &parsed {
                Ok(Some(_)) => "Berhasil masuk. Kamu bisa menutup tab ini dan kembali ke Leo.",
                Ok(None) => "",
                Err(_) => "Login gagal. Kembali ke Leo untuk melihat alasannya.",
            };
            let (status_line, body) = if page.is_empty() { ("404 Not Found", "") } else { ("200 OK", page) };
            let html = format!(
                "<!doctype html><meta charset=utf-8><title>Leo</title><body style=\"font:16px system-ui;background:#111;color:#eee;display:grid;place-items:center;height:100vh\"><p>{body}</p>"
            );
            let _ = write!(
                stream,
                "HTTP/1.1 {status_line}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                html.len()
            );
            match parsed {
                Ok(Some(code)) => {
                    let body = serde_json::json!({ "auth_code": code, "code_verifier": verifier }).to_string();
                    return token_request(cfg, "pkce", &body, None);
                }
                Ok(None) => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(windows)]
pub use online::{fetch_snapshot, login_google};

// ───────────────────────── status bersama ─────────────────────────

#[derive(Clone, Debug, PartialEq, Default)]
pub enum LoginState {
    #[default]
    Idle,
    Waiting,
    Failed(String),
}

static LOGIN: Mutex<LoginState> = Mutex::new(LoginState::Idle);
static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Naik setiap kali data/status berubah, supaya jendela Pengaturan tahu kapan perlu menggambar ulang.
static VERSION: AtomicU32 = AtomicU32::new(0);

pub fn version() -> u32 {
    VERSION.load(Ordering::Relaxed)
}

pub fn login_state() -> LoginState {
    lock(&LOGIN).clone()
}

pub fn snapshot() -> Option<Snapshot> {
    lock(&SNAPSHOT).clone()
}

pub fn last_error() -> String {
    lock(&LAST_ERROR).clone()
}

/// Ganti data terbaru (dipakai pemantau; `None` mengosongkan, mis. setelah keluar dari semua akun).
pub fn set_snapshot(s: Option<Snapshot>, error: String) {
    *lock(&SNAPSHOT) = s;
    *lock(&LAST_ERROR) = error;
    VERSION.fetch_add(1, Ordering::Relaxed);
}

pub fn set_login_state(s: LoginState) {
    *lock(&LOGIN) = s;
    VERSION.fetch_add(1, Ordering::Relaxed);
}

/// Dikirim ke jendela utama setiap kali data makrab diperbarui (atau gagal diperbarui).
#[cfg(windows)]
pub const WM_APP_MAKRAB: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 14;

#[cfg(windows)]
static WAKE: Mutex<Option<std::sync::mpsc::Sender<()>>> = Mutex::new(None);

/// Bangunkan pemantau supaya langsung memperbarui data (mis. setelah ganti akun).
#[cfg(windows)]
pub fn wake() {
    if let Some(tx) = lock(&WAKE).as_ref() {
        let _ = tx.send(());
    }
}

#[cfg(windows)]
fn refresh_once(cfg: &SupabaseCfg) {
    if !cfg.configured() {
        set_snapshot(None, String::new());
        return;
    }
    let Some(acc) = with_accounts(|a| a.active_account().cloned()) else {
        set_snapshot(None, String::new());
        return;
    };
    match fetch_snapshot(cfg, acc) {
        Ok((snap, acc)) => {
            with_accounts(|a| a.update_tokens(acc));
            set_snapshot(Some(snap), String::new());
        }
        // Data lama tetap ditampilkan; galatnya ditulis di halaman Tugas.
        Err(e) => set_snapshot(snapshot(), e),
    }
}

/// Mulai pemantau di latar belakang: memperbarui data tiap `refresh_min` menit dan memberi tahu
/// jendela `hwnd` lewat `WM_APP_MAKRAB`. Membaca settings.json lagi tiap putaran.
#[cfg(windows)]
pub fn start_worker(hwnd: isize) {
    use std::time::Duration;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    *lock(&WAKE) = Some(tx);
    std::thread::spawn(move || loop {
        let cfg = load_json::<crate::settings::Settings>("settings.json").unwrap_or_default().supabase;
        refresh_once(&cfg);
        unsafe {
            let _ = PostMessageW(HWND(hwnd as _), WM_APP_MAKRAB, WPARAM(0), LPARAM(0));
        }
        let _ = rx.recv_timeout(Duration::from_secs(cfg.refresh_min.clamp(1, 120) * 60));
        while rx.try_recv().is_ok() {}
    });
}

/// Mulai login Google di thread sendiri; hasilnya terlihat lewat `login_state()` dan daftar akun.
#[cfg(windows)]
pub fn start_login(cfg: SupabaseCfg) {
    if login_state() == LoginState::Waiting {
        return;
    }
    set_login_state(LoginState::Waiting);
    std::thread::spawn(move || match login_google(&cfg) {
        Ok(acc) => {
            with_accounts(|a| a.upsert(acc));
            set_snapshot(None, String::new());
            set_login_state(LoginState::Idle);
            wake();
        }
        Err(e) => set_login_state(LoginState::Failed(e)),
    });
}

/// Hitung tugas yang perlu diingatkan sekarang: (indeks tugas, tingkat baru) untuk tugas yang
/// tingkat peringatannya naik sejak `seen` terakhir. `seen` diperbarui.
pub fn new_alerts(tasks: &[Task], now: i64, seen: &mut HashMap<String, u8>) -> Vec<(usize, u8)> {
    let mut out = Vec::new();
    for (i, t) in tasks.iter().enumerate() {
        let level = alert_level(t.deadline, now);
        let key = format!("{}|{}", t.judul, t.deadline);
        let prev = seen.get(&key).copied().unwrap_or(0);
        if level > prev {
            seen.insert(key, level);
            out.push((i, level));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(email: &str) -> Account {
        Account { email: email.into(), access_token: "a".into(), refresh_token: "r".into(), expires_at: 100 }
    }

    #[test]
    fn iso8601_matches_reference_epochs() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601("2026-10-27T10:30:15Z"), Some(1793097015));
        assert_eq!(parse_iso8601("2026-11-02T17:00:00+07:00"), Some(1793613600));
        assert_eq!(parse_iso8601("2026-11-02T10:00:00.123456+00:00"), Some(1793613600));
        assert_eq!(parse_iso8601("2028-02-29T23:59:59-05:00"), Some(1835499599));
        assert_eq!(parse_iso8601("besok"), None);
        assert_eq!(parse_iso8601("2026-13-45"), None);
    }

    #[test]
    fn relative_time_in_indonesian() {
        let now = 1_000_000;
        assert_eq!(relative(now + 30 * 60, now), ("30 mnt lagi".into(), Urgency::Soon));
        assert_eq!(relative(now + 2 * 3600 + 5, now), ("2 jam lagi".into(), Urgency::Soon));
        assert_eq!(relative(now + 5 * 3600, now), ("5 jam lagi".into(), Urgency::Today));
        assert_eq!(relative(now + 3 * 86400, now), ("3 hari lagi".into(), Urgency::Later));
        assert_eq!(relative(now - 90, now), ("telat 1 mnt".into(), Urgency::Overdue));
        assert_eq!(relative(now - 5 * 3600, now), ("telat 5 jam".into(), Urgency::Overdue));
        assert_eq!(relative(now - 2 * 86400, now), ("telat 2 hari".into(), Urgency::Overdue));
    }

    #[test]
    fn alert_levels_escalate_once_each() {
        let now = 1_000_000;
        let mut seen = HashMap::new();
        let mk = |d: i64| vec![Task { judul: "Logo".into(), kategori: "makrab".into(), status: "todo".into(), deadline: d }];
        assert!(new_alerts(&mk(now + 3 * 86400), now, &mut seen).is_empty(), "masih jauh");
        assert_eq!(new_alerts(&mk(now + 20 * 3600), now, &mut seen), vec![(0, 1)]);
        assert!(new_alerts(&mk(now + 20 * 3600), now, &mut seen).is_empty(), "tidak diulang");
        assert_eq!(new_alerts(&mk(now + 2 * 3600), now, &mut seen), vec![(0, 2)]);
        assert_eq!(new_alerts(&mk(now + 600), now, &mut seen), vec![(0, 3)]);
        assert_eq!(new_alerts(&mk(now - 10), now, &mut seen), vec![(0, 4)]);
    }

    #[test]
    fn token_and_error_parsing() {
        let ok = r#"{"access_token":"AT","refresh_token":"RT","expires_in":3600,"user":{"email":"a@b.co"}}"#;
        let a = parse_token(ok, 1000, None).unwrap();
        assert_eq!((a.email.as_str(), a.expires_at), ("a@b.co", 4600));
        let no_user = r#"{"access_token":"AT","refresh_token":"RT","expires_in":60}"#;
        assert_eq!(parse_token(no_user, 0, Some("x@y.z")).unwrap().email, "x@y.z");
        assert!(parse_token(no_user, 0, None).is_err());
        assert_eq!(error_message(r#"{"error_description":"Invalid"}"#, 400), "Invalid (HTTP 400)");
        assert_eq!(error_message("bukan json", 500), "HTTP 500");
    }

    #[test]
    fn callback_parsing() {
        assert_eq!(parse_callback("GET /callback?code=abc123 HTTP/1.1"), Ok(Some("abc123".into())));
        assert_eq!(parse_callback("GET /favicon.ico HTTP/1.1"), Ok(None));
        assert_eq!(
            parse_callback("GET /callback?error=access_denied&error_description=Pengguna+menolak HTTP/1.1"),
            Err("Pengguna menolak".into())
        );
        assert!(parse_callback("GET /callback?x=1 HTTP/1.1").is_err());
    }

    #[test]
    fn authorize_url_is_well_formed() {
        let u = authorize_url("https://p.supabase.co/", "http://127.0.0.1:53682/callback", "CH");
        assert_eq!(
            u,
            "https://p.supabase.co/auth/v1/authorize?provider=google&redirect_to=http%3A%2F%2F127.0.0.1%3A53682%2Fcallback&code_challenge=CH&code_challenge_method=s256"
        );
    }

    #[test]
    fn accounts_switch_and_remove() {
        let mut a = Accounts::default();
        a.upsert(acc("satu@x.co"));
        a.upsert(acc("dua@x.co"));
        assert_eq!(a.active.as_deref(), Some("dua@x.co"));
        assert!(a.set_active("satu@x.co") && !a.set_active("tidak@ada"));
        // token diperbarui tanpa mengganti akun aktif; akun asing diabaikan
        a.update_tokens(Account { access_token: "baru".into(), ..acc("dua@x.co") });
        a.update_tokens(acc("asing@x.co"));
        assert_eq!(a.list.len(), 2);
        assert_eq!(a.active.as_deref(), Some("satu@x.co"));
        a.remove("satu@x.co");
        assert_eq!(a.active.as_deref(), Some("dua@x.co"));
        a.remove("dua@x.co");
        assert!(a.active.is_none() && a.list.is_empty());
    }

    #[test]
    fn makrab_json_parsing() {
        let tasks = r#"[{"judul":"Logo","kategori":"makrab","status":"proses","deadline":"2026-11-02T10:00:00+00:00"},
                        {"judul":"Tanpa tenggat","kategori":"kuliah","status":"todo","deadline":null},
                        {"judul":"Awal","kategori":"admin","status":"todo","deadline":"2026-10-30T00:00:00Z"}]"#;
        let t = parse_tasks(tasks).unwrap();
        assert_eq!(t.iter().map(|x| x.judul.as_str()).collect::<Vec<_>>(), ["Awal", "Logo"], "urut dan tanpa deadline dibuang");
        assert_eq!(parse_progress(r#"[{"status":"selesai"},{"status":"todo"},{"status":"selesai"}]"#), Ok((3, 2)));
        assert_eq!(count_rows(r#"[{"id":1},{"id":2}]"#), Ok(2));
        assert!(parse_tasks("{}").is_err());
    }
}
