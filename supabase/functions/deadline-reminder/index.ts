// Edge Function: kirim email pengingat deadline tugas makrab lewat Gmail (SMTP + App Password).
//
// Dipanggil tiap 15 menit oleh pg_cron (lihat ../../email-reminders.sql).
// Penerima: anggota yang ditugaskan pada tugas (atau tugas induknya) + semua admin.
// Tingkat pengingat sama dengan notch Leo: <= 24 jam (1), <= 3 jam (2), terlambat (4);
// tiap tingkat hanya dikirim sekali per tugas (dicatat di tabel makrab_email_log).
//
// Secret yang dibutuhkan (supabase secrets set ...):
//   GMAIL_USER, GMAIL_APP_PASSWORD (sandi aplikasi Gmail, bukan sandi akun), CRON_SECRET
// SUPABASE_URL dan SUPABASE_SERVICE_ROLE_KEY sudah disediakan otomatis oleh Supabase.

import { createClient } from "npm:@supabase/supabase-js@2";
import nodemailer from "npm:nodemailer@6";

const HOUR = 3_600_000;

type Task = { id: string; judul: string; kategori: string; deadline: string; parent_id: string | null };
type Due = Task & { level: number };

function level(diffMs: number): number {
  if (diffMs < 0) return 4;
  if (diffMs <= 3 * HOUR) return 2;
  if (diffMs <= 24 * HOUR) return 1;
  return 0;
}

function relative(diffMs: number): string {
  const abs = Math.abs(diffMs);
  const text = abs < HOUR ? `${Math.max(1, Math.round(abs / 60_000))} mnt`
    : abs < 24 * HOUR ? `${Math.floor(abs / HOUR)} jam`
    : `${Math.floor(abs / (24 * HOUR))} hari`;
  return diffMs < 0 ? `telat ${text}` : `${text} lagi`;
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

Deno.serve(async (req) => {
  if (req.headers.get("x-cron-secret") !== Deno.env.get("CRON_SECRET")) return json({ error: "forbidden" }, 403);

  const db = createClient(Deno.env.get("SUPABASE_URL")!, Deno.env.get("SUPABASE_SERVICE_ROLE_KEY")!);
  const now = Date.now();

  // Tugas belum selesai dengan deadline dalam 24 jam ke depan atau baru lewat dalam 24 jam terakhir.
  const { data: tasks, error } = await db
    .from("makrab_tasks")
    .select("id, judul, kategori, deadline, parent_id")
    .neq("status", "selesai")
    .not("deadline", "is", null)
    .gte("deadline", new Date(now - 24 * HOUR).toISOString())
    .lte("deadline", new Date(now + 24 * HOUR).toISOString());
  if (error) return json({ error: error.message }, 500);
  if (!tasks?.length) return json({ sent: 0 });

  const { data: logs } = await db.from("makrab_email_log").select("task_id, level").in("task_id", tasks.map((t) => t.id));
  const already = new Set((logs ?? []).map((l) => `${l.task_id}:${l.level}`));
  const due: Due[] = (tasks as Task[])
    .map((t) => ({ ...t, level: level(new Date(t.deadline).getTime() - now) }))
    .filter((t) => t.level > 0 && !already.has(`${t.id}:${t.level}`));
  if (!due.length) return json({ sent: 0 });

  // Penerima: yang ditugaskan (pada tugas atau induknya) + semua admin.
  const taskIds = [...new Set(due.flatMap((t) => [t.id, t.parent_id].filter((x): x is string => !!x)))];
  const [{ data: assigns }, { data: members }] = await Promise.all([
    db.from("makrab_task_assignees").select("task_id, member_id").in("task_id", taskIds),
    db.from("makrab_members").select("id, email, is_admin"),
  ]);
  const byId = new Map((members ?? []).map((m) => [m.id, m]));
  const admins = (members ?? []).filter((m) => m.is_admin).map((m) => m.email as string);

  const digests = new Map<string, Due[]>();
  for (const t of due) {
    const to = new Set<string>(admins);
    for (const a of assigns ?? []) {
      if (a.task_id === t.id || a.task_id === t.parent_id) {
        const m = byId.get(a.member_id);
        if (m) to.add(m.email as string);
      }
    }
    for (const email of to) digests.set(email, [...(digests.get(email) ?? []), t]);
  }

  const user = Deno.env.get("GMAIL_USER")!;
  const transport = nodemailer.createTransport({
    host: "smtp.gmail.com",
    port: 465, // Edge Functions memblokir port 25 dan 587
    secure: true,
    auth: { user, pass: Deno.env.get("GMAIL_APP_PASSWORD")! },
  });

  let sent = 0;
  for (const [to, list] of digests) {
    const lines = list
      .sort((a, b) => a.deadline.localeCompare(b.deadline))
      .map((t) => {
        const diff = new Date(t.deadline).getTime() - now;
        const when = new Date(t.deadline).toLocaleString("id-ID", { timeZone: "Asia/Jakarta", dateStyle: "medium", timeStyle: "short" });
        return `• ${t.judul} (${t.kategori}) — ${relative(diff)}, ${when}`;
      });
    try {
      await transport.sendMail({
        from: `Leo <${user}>`,
        to,
        subject: `⏰ ${list.length} tugas mendekati deadline`,
        text: `Halo,\n\nTugas berikut mendekati atau melewati deadline:\n\n${lines.join("\n")}\n\n— Leo`,
      });
      sent++;
    } catch (e) {
      console.error(`gagal kirim ke ${to}:`, e);
    }
  }

  // Catat supaya tidak terkirim ulang tiap 15 menit. Bila semua pengiriman gagal, coba lagi di putaran berikutnya.
  if (sent > 0 || digests.size === 0) {
    await db.from("makrab_email_log").upsert(
      due.map((t) => ({ task_id: t.id, level: t.level })),
      { onConflict: "task_id,level", ignoreDuplicates: true },
    );
  }
  return json({ sent, tasks: due.length });
});
