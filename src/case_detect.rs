//! Deteksi "case earbud dibuka" dari iklan BLE. Murni logika (tanpa API Windows)
//! supaya mudah dites.
//!
//! Saat case dibuka, earbud memancarkan iklan singkat (kadang cuma beberapa paket) dengan
//! alamat acak yang berganti-ganti, dan kiri/kanan beralamat berbeda. Karena itu yang
//! dipakai adalah sinyal TERKUAT dari semua alamat dalam jendela waktu pendek, bukan per alamat.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Bobot sampel baru pada penghalusan RSSI (0..1).
const EMA_ALPHA: f32 = 0.5;
/// Sinyal terkuat dihitung dari paket dalam jendela ini (kiri + kanan).
const WINDOW: Duration = Duration::from_secs(2);
/// Tanpa paket selama ini, case dianggap sudah tertutup / menjauh.
const LOST: Duration = Duration::from_secs(8);
/// Selisih (dB) antara ambang buka dan ambang tutup, supaya tidak berkedip.
const HYSTERESIS: i32 = 10;
/// Windows melapor -127 bila RSSI tidak tersedia; nilai itu diabaikan.
const MIN_VALID: i16 = -120;

pub struct CaseDetector {
    open_rssi: f32,
    close_rssi: f32,
    open: bool,
    ema: Option<f32>,
    last_seen: Option<Instant>,
    recent: HashMap<u64, (Instant, i16)>,
}

impl CaseDetector {
    /// `open_rssi`: case dianggap terbuka bila sinyal lebih kuat dari ini (dBm).
    pub fn new(open_rssi: i32) -> Self {
        Self {
            open_rssi: open_rssi as f32,
            close_rssi: (open_rssi - HYSTERESIS) as f32,
            open: false,
            ema: None,
            last_seen: None,
            recent: HashMap::new(),
        }
    }

    /// Satu paket iklan. Mengembalikan `true` tepat saat case baru dianggap terbuka.
    pub fn ingest(&mut self, addr: u64, rssi: i16, now: Instant) -> bool {
        if rssi <= MIN_VALID {
            return false;
        }
        // Lama hening: sampel lama sudah basi dan case dianggap sempat tertutup.
        if self.last_seen.is_some_and(|t| now.duration_since(t) > LOST) {
            self.reset();
        }
        self.last_seen = Some(now);

        self.recent.insert(addr, (now, rssi));
        self.recent.retain(|_, (t, _)| now.duration_since(*t) <= WINDOW);
        let strongest = self.recent.values().map(|&(_, r)| r).max().unwrap_or(rssi) as f32;

        let ema = match self.ema {
            None => strongest,
            Some(e) => EMA_ALPHA * strongest + (1.0 - EMA_ALPHA) * e,
        };
        self.ema = Some(ema);

        if self.open {
            if ema < self.close_rssi {
                self.reset();
            }
            return false;
        }
        if ema > self.open_rssi {
            self.open = true;
            return true;
        }
        false
    }

    fn reset(&mut self) {
        self.open = false;
        self.ema = None;
        self.recent.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn opens_once_for_left_and_right_earbud() {
        let t0 = Instant::now();
        let mut d = CaseDetector::new(-50);
        assert!(d.ingest(1, -19, at(t0, 0)), "paket kuat pertama membuka");
        assert!(!d.ingest(2, -45, at(t0, 200)), "earbud satunya tidak membuka lagi");
        assert!(!d.ingest(1, -20, at(t0, 1000)));
    }

    #[test]
    fn weak_signal_does_not_open() {
        let t0 = Instant::now();
        let mut d = CaseDetector::new(-50);
        assert!(!d.ingest(1, -70, at(t0, 0)));
        assert!(!d.ingest(1, -55, at(t0, 500)));
    }

    #[test]
    fn invalid_rssi_is_ignored_and_never_closes() {
        let t0 = Instant::now();
        let mut d = CaseDetector::new(-50);
        assert!(d.ingest(1, -15, at(t0, 0)));
        assert!(!d.ingest(1, -127, at(t0, 500)));
        // -127 tidak boleh merusak status: paket kuat berikutnya tidak membuka ulang.
        assert!(!d.ingest(1, -16, at(t0, 1000)));
    }

    #[test]
    fn reopens_after_long_silence() {
        let t0 = Instant::now();
        let mut d = CaseDetector::new(-50);
        assert!(d.ingest(1, -15, at(t0, 0)));
        assert!(d.ingest(1, -15, at(t0, 9_000)), "hening > 8 dtk lalu paket kuat = pop-up lagi");
    }

    #[test]
    fn closes_when_moving_away_and_opens_again_when_back() {
        let t0 = Instant::now();
        let mut d = CaseDetector::new(-50);
        assert!(d.ingest(1, -19, at(t0, 0)));
        // Menjauh: paket lemah beruntun menurunkan rata-rata di bawah ambang tutup (-60).
        assert!(!d.ingest(1, -90, at(t0, 2_500)));
        assert!(!d.ingest(1, -90, at(t0, 3_000)));
        // Kembali mendekat: pop-up lagi.
        assert!(d.ingest(1, -19, at(t0, 3_500)));
    }
}
