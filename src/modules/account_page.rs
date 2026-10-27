//! Halaman Akun (di jendela Pengaturan): masuk dengan Google lewat Supabase, boleh lebih dari
//! satu akun dan bisa ditukar langsung tanpa keluar-masuk. Dipakai untuk data tugas/makrab.

use super::Module;
use crate::gfx::{Align, Color, Gfx, Rect, DIM, FAINT, WHITE};
use crate::settings::SharedSettings;
use crate::supabase::{self, Accounts, LoginState};

const ACCENT: Color = Color(0.35, 0.85, 0.55, 1.0);
const RED: Color = Color(1.0, 0.42, 0.42, 1.0);
const ROW: f32 = 32.0;
const MAX_ROWS: usize = 5;

enum Hit {
    Use(String),
    Remove(String),
    Add,
}

pub struct AccountPage {
    settings: SharedSettings,
    hits: Vec<(Rect, Hit)>,
}

impl AccountPage {
    pub fn new(settings: SharedSettings) -> Self {
        Self { settings, hits: Vec::new() }
    }
}

impl Module for AccountPage {
    fn tab(&self) -> Option<&'static str> {
        Some("Akun")
    }

    fn expanded_size(&self) -> (f32, f32) {
        (560.0, 44.0 + 40.0 + MAX_ROWS as f32 * ROW + 50.0 + 66.0)
    }

    fn draw_expanded(&mut self, g: &mut Gfx, r: Rect) {
        self.hits.clear();
        let cfg = self.settings.borrow().supabase.clone();
        g.text("Akun Google (Supabase)", Rect::new(r.x + 20.0, r.y + 10.0, 300.0, 24.0), 15.0, 600, WHITE, Align::Left);

        let mut y = r.y + 40.0;
        if !cfg.configured() {
            g.text_wrapped(
                "anon_key belum diisi. Buka %LOCALAPPDATA%\\Leo\\settings.json, isi supabase.anon_key (sama dengan VITE_SUPABASE_ANON_KEY di website), lalu mulai ulang Leo.",
                Rect::new(r.x + 20.0, y, r.w - 40.0, 56.0),
                12.0,
                400,
                RED,
                Align::Left,
            );
            y += 60.0;
        }

        let accounts = Accounts::load();
        for acc in accounts.list.iter().take(MAX_ROWS) {
            let row = Rect::new(r.x + 20.0, y, r.w - 40.0, ROW);
            let active = accounts.active.as_deref() == Some(acc.email.as_str());
            g.text(&acc.email, Rect::new(row.x, row.y, row.w - 150.0, ROW), 12.5, if active { 600 } else { 400 }, WHITE, Align::Left);
            let remove = Rect::new(row.x + row.w - 62.0, row.y + 4.0, 62.0, ROW - 8.0);
            g.button(remove, "Keluar", FAINT, WHITE);
            self.hits.push((remove, Hit::Remove(acc.email.clone())));
            let slot = Rect::new(remove.x - 8.0 - 62.0, remove.y, 62.0, remove.h);
            if active {
                g.text("Aktif", slot, 12.0, 600, ACCENT, Align::Center);
            } else {
                g.button(slot, "Pakai", FAINT, WHITE);
                self.hits.push((slot, Hit::Use(acc.email.clone())));
            }
            y += ROW;
        }
        if accounts.list.is_empty() {
            g.text("Belum ada akun yang masuk.", Rect::new(r.x + 20.0, y, r.w - 40.0, 24.0), 12.5, 400, DIM, Align::Left);
            y += ROW;
        }

        let state = supabase::login_state();
        let add = Rect::new(r.x + 20.0, y + 8.0, 230.0, 30.0);
        match &state {
            LoginState::Waiting => g.button(add, "Menunggu login di browser…", FAINT, DIM),
            _ => {
                g.button(add, "+ Tambah akun Google", ACCENT, Color(0.05, 0.15, 0.08, 1.0));
                self.hits.push((add, Hit::Add));
            }
        }
        if let LoginState::Failed(e) = &state {
            g.text_wrapped(e, Rect::new(add.x + add.w + 12.0, add.y - 2.0, r.x + r.w - 20.0 - (add.x + add.w + 12.0), 36.0), 11.5, 400, RED, Align::Left);
        }

        g.text_wrapped(
            "Pertama kali: di Supabase → Authentication → URL Configuration → Redirect URLs, tambahkan http://127.0.0.1:53682/callback",
            Rect::new(r.x + 20.0, r.y + r.h - 46.0, r.w - 40.0, 40.0),
            11.0,
            400,
            DIM,
            Align::Left,
        );
    }

    fn click(&mut self, _r: Rect, x: f32, y: f32) -> bool {
        let Some((_, hit)) = self.hits.iter().find(|(r, _)| r.contains(x, y)) else { return false };
        match hit {
            Hit::Use(email) => {
                supabase::with_accounts(|a| a.set_active(email));
                supabase::set_snapshot(None, String::new());
                supabase::wake();
            }
            Hit::Remove(email) => {
                supabase::with_accounts(|a| a.remove(email));
                supabase::set_snapshot(None, String::new());
                supabase::wake();
            }
            Hit::Add => supabase::start_login(self.settings.borrow().supabase.clone()),
        }
        true
    }
}
