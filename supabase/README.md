# Pengingat deadline lewat email (opsional, sisi server)

Notch Leo sudah mengingatkan deadline di layar. Bagian ini menambah **email Gmail** yang dikirim
dari Supabase (jadi tetap jalan walau Leo/laptop mati). Belum pernah diuji pada proyek asli;
cek log fungsi setelah deploy.

1. **Sandi aplikasi Gmail**: akun Google harus memakai verifikasi 2 langkah, lalu buat
   *App password* di myaccount.google.com/apppasswords.
2. **Deploy fungsi** (butuh Supabase CLI, sudah `supabase link` ke proyek):
   ```
   supabase secrets set GMAIL_USER=alamat@gmail.com GMAIL_APP_PASSWORD=xxxxxxxxxxxxxxxx CRON_SECRET=isi-acak-panjang
   supabase functions deploy deadline-reminder --no-verify-jwt
   ```
3. **Jadwal & tabel catatan**: buka `email-reminders.sql`, ganti `<PROJECT-REF>` dan `<CRON-SECRET>`
   (sama dengan `CRON_SECRET` di atas), lalu jalankan di SQL Editor Supabase.

Penerima: anggota yang ditugaskan pada tugas (atau tugas induknya) dan semua admin.
Tingkat: kurang dari 24 jam, kurang dari 3 jam, dan terlambat; tiap tingkat sekali per tugas.
