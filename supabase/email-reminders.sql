-- ─────────────────────────────────────────────────────────────────────────────
--  Leo: pengingat deadline lewat email (Gmail), dijalankan di sisi Supabase.
--  Jalankan sekali di SQL Editor Supabase SETELAH makrab.sql milik website.
--  Fungsinya (Edge Function `deadline-reminder`) dipanggil tiap 15 menit oleh pg_cron.
-- ─────────────────────────────────────────────────────────────────────────────

-- Catatan email yang sudah terkirim, supaya satu tugas tidak diemail berulang di tingkat yang sama.
-- level: 1 = kurang dari 24 jam, 2 = kurang dari 3 jam, 4 = terlambat
create table if not exists public.makrab_email_log (
  task_id uuid     not null references public.makrab_tasks(id) on delete cascade,
  level   smallint not null,
  sent_at timestamptz not null default now(),
  primary key (task_id, level)
);

-- RLS aktif tanpa policy = tidak bisa diakses dari aplikasi/website; hanya service role (Edge Function).
alter table public.makrab_email_log enable row level security;

create extension if not exists pg_cron;
create extension if not exists pg_net;

-- GANTI <PROJECT-REF> dan <CRON-SECRET> (nilai sama dengan secret CRON_SECRET di Edge Function).
select cron.schedule(
  'makrab-email-reminders',
  '*/15 * * * *',
  $$
  select net.http_post(
    url     := 'https://<PROJECT-REF>.supabase.co/functions/v1/deadline-reminder',
    headers := jsonb_build_object('Content-Type', 'application/json', 'x-cron-secret', '<CRON-SECRET>'),
    body    := '{}'::jsonb
  );
  $$
);

-- Untuk mematikan: select cron.unschedule('makrab-email-reminders');
