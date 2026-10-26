# Amora

[English](README.md) · **Bahasa Indonesia**

Notch hidup di tepi atas layar Windows. Ditulis dengan Rust + Direct2D, tanpa web engine.

## Memasang

Jalankan `AmoraSetup.exe`. Amora dipasang untuk akun Anda saja di `%LOCALAPPDATA%\Programs\Amora`
(tanpa hak admin), muncul di Start Menu, langsung berjalan, dan ikut mulai bersama Windows.
Hapus kapan saja dari **Settings → Apps → Amora**.

Membangun installer sendiri (butuh Rust + Visual Studio Build Tools dengan Windows SDK):

```
powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
```

Hasilnya `dist\AmoraSetup.exe`, beserta SHA-256-nya. Bila `target\release\amora.exe` sedang
berjalan dan terkunci, tambahkan `-TargetDir target-dev`.

## Menjalankan dari kode

```
cargo run --release
```

Klik kanan notch atau klik ikon Amora di tray untuk menu: **Mulai bersama Windows**, **Maskot Amora**, **Notch minimal**, **Riwayat clipboard**, **Tentang Amora**, **Keluar**.

Untuk pengembangan:

- `AmoraSetup.exe /S` memasang tanpa pertanyaan; `amora --uninstall --quiet [--wipe]` menghapus tanpa pertanyaan.
- `AMORA_DEV=1` menjalankan instance uji di samping Amora yang sedang jalan.
- `AMORA_DEV_X=<piksel>` menggeser instance uji (bawaan: ke kanan), berguna di layar sempit.
- `AMORA_DEMO_LYRIC="<baris>"` memutar lagu palsu dengan baris lirik itu, untuk menguji tampilan lirik tanpa musik.
- `cargo run --example volume_ping` menggeser volume 1% lalu mengembalikannya, untuk menguji OSD.
- `AMORA_MASCOT=mata|tidur|halo` memaksa keadaan maskot.
- `AMORA_FORCE_OPEN=1` membuat notch selalu terbuka; `AMORA_PAGE=Fokus` memilih halamannya.
- `cargo run -- --probe-lyrics "<judul>" "<artis>" <durasi-detik>` menguji pencarian lirik.
- `amora --probe-table <file>` menampilkan hasil urai tabel dari file CF_HTML atau teks Markdown.
- `cargo run --example clip_put -- <fragmen.html> <teks.txt>` mengisi clipboard seperti browser, untuk menguji "Rapikan tabel".
- `amora --probe-system` mencetak status jaringan, output audio, dan bahasa keyboard.
- `amora --autostart on|off|status` menguji opsi mulai bersama Windows.
- `amora --crash-test` memicu panic untuk memastikan tercatat di `%LOCALAPPDATA%\Amora\log.txt`.
- `cargo test` menjalankan tes pembersih judul dan parser LRC.

## Struktur

- `src/main.rs` — jendela, hover, animasi, pemilihan modul
- `src/gfx.rs` — gambar (Direct2D ke jendela layered; area transparan tembus klik)
- `src/media.rs` — membaca musik dari Windows (SMTC) di thread terpisah
- `src/lyrics/` — pencarian lirik LRCLIB (`matcher.rs`: pembersih judul + parser LRC)
- `src/tray.rs` — ikon tray (digambar di kode) dan event-nya
- `src/autostart.rs` — mulai bersama Windows lewat `HKCU\...\Run`
- `src/keyboard.rs` — Caps Lock dan bahasa (hook hanya bereaksi pada tombol itu)
- `src/network.rs` — status internet/Wi-Fi
- `src/privacy.rs` — pemakaian mic/kamera dari registry ConsentStore (event, bukan polling)
- `src/screenshots.rs` — memantau folder Screenshots
- `src/occlusion.rs` — deteksi jendela di posisi notch (event jendela Windows)
- `src/system.rs` — event volume (Core Audio) dan kecerahan (power setting)
- `src/tables.rs` — urai tabel (HTML/Markdown) dan susun ulang jadi tabel rapi
- `src/time.rs` — waktu lokal untuk pengingat
- `src/uninstall.rs` — `amora --uninstall`, dipanggil dari Settings → Apps
- `installer/` — AmoraSetup.exe (membawa amora.exe di dalamnya)
- `assets/amora.ico` — ikon, dibuat oleh `tools/make_icon.py`
- `src/settings.rs` — setelan di `%LOCALAPPDATA%\Amora\settings.json`
- `src/modules/` — satu file per fitur; implementasikan trait `Module` lalu daftarkan di `main.rs`

## Status

- [x] Notch, hover melebar, animasi pegas
- [x] Jam (saat tidak ada musik)
- [x] Musik: judul, artis, sampul, putar/jeda/lompat, progress + seek
- [x] Lirik pintar: LRCLIB, judul YouTube/VEVO/Topic dibersihkan, cache di `%LOCALAPPDATA%\Amora\lyrics`
- [x] Ikon tray + menu, mulai bersama Windows
- [x] Halaman (Musik · Fokus · Minum · To-do · Jadwal · Shelf · Clipboard): klik nama halaman atau scroll
- [x] Fokus (Pomodoro), sisa waktu tampil di notch tertutup
- [x] Minum: catat, grafik per jam, pengingat di jam aktif
- [x] To-do: ketik di notch, `@14:30` untuk pengingat
- [x] Shelf: seret file ke notch, klik untuk membuka, seret keluar (salin, tidak memindahkan)
- [x] OSD volume (ikut ganti perangkat audio) dan kecerahan layar laptop
- [x] Minggir saat ada jendela di belakang notch dan kursor mengarah ke sana (mis. tab browser); tempel kursor ke tepi atas layar untuk membuka notch
- [x] Notifikasi: Caps Lock, bahasa keyboard, charger/baterai lemah, internet/Wi-Fi, ganti output audio (headphone)
- [x] Maskot: mata muncul saat PC diam 1 menit, tidur setelah 2 menit, menyapa saat kembali
- [x] Mode notch minimal: garis tipis, tetap muncul penuh untuk lirik dan notifikasi
- [x] Indikator mic (oranye) / kamera (hijau) + notifikasi nama aplikasinya
- [x] Screenshot baru otomatis masuk Shelf
- [x] Sleep timer musik 15/30/60 menit, volume memudar 10 detik lalu musik dijeda
- [x] Jadwal kuliah mingguan: ketik `Senin 08:00-09:40 Basis Data @R301`, pengingat 10 menit sebelum kelas, hitung mundur di notch tertutup
- [x] Halaman Setelan di notch (semua pilihan, langsung berlaku)
- [x] Lirik: cari ulang, geser waktu per lagu, animasi pergantian baris, baris panjang dibungkus
- [x] Ikon aplikasi, installer tanpa admin, uninstall dari Settings → Apps
- [x] Rapikan tabel yang disalin dari chat AI (diblok atau tombol Copy tabel): Ctrl+V di Google Docs/Word langsung jadi tabel bergaris, judul tebal, lebar kolom pas, huruf mengikuti dokumen
- [x] Riwayat clipboard 5 teks terakhir (hanya di memori, mati default, lewati password manager)

## Setelan

Semua bisa diubah di halaman **Setelan** pada notch. Tersimpan di `%LOCALAPPDATA%\Amora\settings.json`:

- `tidy_tables`: rapikan tabel yang disalin (default `true`)
- `clipboard`: riwayat clipboard (default `false`, juga lewat menu tray)
- `screenshot_to_shelf`: screenshot baru masuk Shelf (default `true`)
- `mascot`, `minimal`: juga bisa diubah lewat menu tray
- `auto_hide`: notch minggir saat kursor mengarah ke jendela di belakangnya (default `true`)
- `pomodoro`: `focus_min`, `short_break_min`, `long_break_min`, `rounds`
- `water`: `goal_ml`, `sip_ml`, `remind`, `interval_min`, `start_hour`, `end_hour`

Data: `water.json` (hari ini), `todo.json`, `schedule.json`, `lyric_offsets.json`, `shelf.json` (lokasi file saja), dan cache lirik di `lyrics\`.
