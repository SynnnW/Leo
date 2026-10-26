//! Tabel yang disalin dari chat AI (Claude, ChatGPT, …) dirapikan sebelum ditempel ke
//! Google Docs/Word: diberi garis, judul kolom tebal, dan lebar kolom sesuai isi.
//! Sumbernya bisa HTML (tabel yang diblok atau tombol Copy tabel) atau teks Markdown.

/// Satu baris tabel; `head` = baris judul.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub cells: Vec<String>,
    pub head: bool,
}

/// Lebar halaman (A4/Letter, huruf 11 pt) dalam karakter, untuk membagi lebar kolom.
const PAGE_CHARS: f32 = 90.0;

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn tidy(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Teks di luar tag, untuk memastikan potongan HTML hanya berisi tabel.
fn visible_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    tidy(&decode_entities(&out))
}

/// Ambil tabel dari potongan HTML. Hanya bila potongan itu memang berisi satu tabel saja:
/// kalau ada teks lain di luar tabel, biarkan (supaya teks itu tidak hilang saat ditempel).
pub fn from_html(fragment: &str) -> Option<Vec<Row>> {
    let lower = fragment.to_ascii_lowercase();
    // Tabel dari Excel/Google Sheets sudah punya formatnya sendiri; jangan diubah.
    if lower.contains("urn:schemas-microsoft-com:office") || lower.contains("google-sheets-html-origin") {
        return None;
    }
    // Sudah dirapikan oleh Leo (atau Amora, nama lama aplikasi ini).
    if lower.contains("data-leo") || lower.contains("data-amora") {
        return None;
    }
    let start = lower.find("<table")?;
    let end = lower.rfind("</table>")? + "</table>".len();
    if lower[start + 6..end - 8].contains("<table") {
        return None; // tabel bersarang: terlalu rumit, biarkan apa adanya
    }
    let outside = format!("{}{}", &fragment[..start], &fragment[end..]);
    if !visible_text(&outside).is_empty() {
        return None;
    }

    let body = &fragment[start..end];
    let mut rows: Vec<Row> = Vec::new();
    let mut row: Option<Row> = None;
    // Semua sel di baris ini <th>?
    let mut all_th = true;
    let mut cell: Option<(String, bool)> = None;
    let mut in_thead = false;
    let mut skip = false; // isi <style>/<script>
    let mut i = 0;
    while i < body.len() {
        if body.as_bytes()[i] == b'<' {
            let Some(close) = body[i..].find('>') else { break };
            let tag = body[i + 1..i + close].trim().to_ascii_lowercase();
            let closing = tag.starts_with('/');
            let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            match (name.as_str(), closing) {
                ("style" | "script", c) => skip = !c,
                ("thead", c) => in_thead = !c,
                ("tr", false) => {
                    row = Some(Row { cells: Vec::new(), head: in_thead });
                    all_th = true;
                }
                ("tr", true) => {
                    if let Some(mut r) = row.take() {
                        // Baris judul = berada di <thead>, atau semua selnya <th>.
                        r.head = r.head || all_th;
                        if !r.cells.is_empty() {
                            rows.push(r);
                        }
                    }
                }
                ("td" | "th", false) => cell = Some((String::new(), name == "th")),
                ("td" | "th", true) => {
                    if let (Some((text, th)), Some(r)) = (cell.take(), row.as_mut()) {
                        r.cells.push(tidy(&decode_entities(&text)));
                        all_th &= th;
                    }
                }
                ("br" | "p" | "li", _) => {
                    if let Some((text, _)) = cell.as_mut() {
                        text.push(' ');
                    }
                }
                _ => {}
            }
            i += close + 1;
        } else {
            let next = body[i..].find('<').map_or(body.len(), |n| i + n);
            if let (Some((text, _)), false) = (cell.as_mut(), skip) {
                text.push_str(&body[i..next]);
            }
            i = next;
        }
    }
    valid(rows)
}

fn split_md_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    cells.push(cur);
    cells.into_iter().map(|c| tidy(&strip_md(&c))).collect()
}

/// Buang penanda Markdown sederhana di dalam sel: **tebal**, `kode`, <br>.
fn strip_md(s: &str) -> String {
    s.replace("**", "").replace("__", "").replace('`', "").replace("<br>", " ").replace("<br/>", " ")
}

fn is_separator(line: &str) -> bool {
    let cells = split_md_row(line);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim_matches(':');
            c.len() >= 3 && c.chars().all(|ch| ch == '-')
        })
}

/// Ambil tabel dari teks Markdown. Hanya bila seluruh teks adalah satu tabel.
pub fn from_markdown(text: &str) -> Option<Vec<Row>> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() < 3 || !lines.iter().all(|l| l.starts_with('|')) || !is_separator(lines[1]) {
        return None;
    }
    let mut rows = vec![Row { cells: split_md_row(lines[0]), head: true }];
    for l in &lines[2..] {
        rows.push(Row { cells: split_md_row(l), head: false });
    }
    valid(rows)
}

/// Minimal 2 kolom dan 2 baris, supaya teks biasa tidak salah dikira tabel.
fn valid(mut rows: Vec<Row>) -> Option<Vec<Row>> {
    let cols = rows.iter().map(|r| r.cells.len()).max()?;
    if cols < 2 || rows.len() < 2 {
        return None;
    }
    for r in &mut rows {
        r.cells.resize(cols, String::new());
    }
    Some(rows)
}

/// Lebar kolom (persen): setiap kolom minimal muat kata terpanjangnya (kata judul dihitung
/// lebih lebar karena tebal), sisa halaman dibagi menurut banyaknya teks di kolom itu.
pub fn column_widths(rows: &[Row]) -> Vec<u32> {
    let cols = rows.first().map_or(0, |r| r.cells.len());
    let longest = |head: bool, c: usize, k: f32| {
        rows.iter()
            .filter(|r| r.head == head)
            .flat_map(|r| r.cells[c].split_whitespace())
            .map(|w| w.chars().count() as f32 * k)
            .fold(1.0, f32::max)
    };
    let mut mins = Vec::new();
    let mut mass = Vec::new();
    for c in 0..cols {
        mins.push(longest(true, c, 1.15).max(longest(false, c, 1.0)) + 2.0);
        let body: Vec<&Row> = rows.iter().filter(|r| !r.head).collect();
        let total: usize = body.iter().map(|r| r.cells[c].chars().count()).sum();
        mass.push(total as f32 / body.len().max(1) as f32);
    }
    let rest = (PAGE_CHARS - mins.iter().sum::<f32>()).max(0.0);
    let all_mass = mass.iter().sum::<f32>().max(1.0);
    let w: Vec<f32> = mins.iter().zip(&mass).map(|(m, a)| m + rest * a / all_mass).collect();
    let sum: f32 = w.iter().sum();
    w.iter().map(|x| (100.0 * x / sum).round() as u32).collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Susun tabel HTML yang rapi. Huruf tidak diatur, jadi mengikuti dokumen tujuan.
pub fn to_html(rows: &[Row]) -> String {
    let pct = column_widths(rows);
    let cell = "border:1pt solid #000000;padding:3pt 5pt;vertical-align:top;color:#000000;";
    let mut out = String::from(r#"<table data-leo="1" border="1" style="border-collapse:collapse;width:100%;"><colgroup>"#);
    for p in &pct {
        out.push_str(&format!(r#"<col width="{p}%" style="width:{p}%;">"#));
    }
    out.push_str("</colgroup>");
    for r in rows {
        let (tag, extra) = if r.head { ("th", "font-weight:bold;text-align:center;") } else { ("td", "") };
        out.push_str("<tr>");
        for (i, c) in r.cells.iter().enumerate() {
            out.push_str(&format!(r#"<{tag} style="{cell}{extra}width:{}%;">{}</{tag}>"#, pct[i], escape(c)));
        }
        out.push_str("</tr>");
    }
    out.push_str("</table>");
    out
}

/// Potongan HTML dari format clipboard "HTML Format" (CF_HTML): header berisi offset byte.
pub fn cf_html_fragment(cf: &[u8]) -> Option<String> {
    // Offset dihitung dari awal data; BOM (bila ada, mis. dari file) bukan bagiannya.
    let cf = cf.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(cf);
    let head = String::from_utf8_lossy(&cf[..cf.len().min(400)]).into_owned();
    let num = |key: &str| -> Option<usize> {
        let i = head.find(key)? + key.len();
        head[i..].lines().next()?.trim().parse().ok()
    };
    let (s, e) = (num("StartFragment:")?, num("EndFragment:")?);
    (s <= e && e <= cf.len()).then(|| String::from_utf8_lossy(&cf[s..e]).into_owned())
}

/// Bungkus potongan HTML menjadi format clipboard CF_HTML.
pub fn cf_html(fragment: &str) -> Vec<u8> {
    let pre = "<html><body><!--StartFragment-->";
    let post = "<!--EndFragment--></body></html>";
    let header_len = format!("Version:0.9\r\nStartHTML:{0:010}\r\nEndHTML:{0:010}\r\nStartFragment:{0:010}\r\nEndFragment:{0:010}\r\n", 0).len();
    let start_frag = header_len + pre.len();
    let end_frag = start_frag + fragment.len();
    let end_html = end_frag + post.len();
    format!(
        "Version:0.9\r\nStartHTML:{header_len:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_frag:010}\r\nEndFragment:{end_frag:010}\r\n{pre}{fragment}{post}"
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Struktur sama seperti salinan nyata dari Claude (blok tabel) dan ChatGPT (tombol Copy),
    // isinya contoh buatan.
    const CLAUDE: &str = r#"<table><thead><tr><th scope="col">Entitas</th><th scope="col">Relasi</th><th scope="col">Kardinalitas</th></tr></thead><tbody><tr><td>Pemesanan</td><td>Pembayaran &amp; Refund</td><td>1 : 0..N</td></tr><tr><td>Sesi</td><td>Ulasan</td><td>0..1 : 0..N</td></tr></tbody></table>"#;
    const CHATGPT_HTML: &str = "<html><head></head><body><table>\n<thead>\n<tr>\n<th>Komponen</th>\n<th>Isi</th>\n</tr>\n</thead>\n<tbody><tr>\n<td>Aktor</td>\n<td>Pemilik usaha</td>\n</tr>\n<tr>\n<td>Pascakondisi</td>\n<td>Ringkasan periode terpilih ditampilkan</td>\n</tr>\n</tbody></table>\n</body></html>";
    const CHATGPT_MD: &str = "| Komponen | Isi |\n|---|---|\n| Aktor | Pemilik usaha |\n| Pascakondisi | Ringkasan periode terpilih ditampilkan |\n";

    #[test]
    fn claude_html() {
        let rows = from_html(CLAUDE).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].head && !rows[1].head);
        assert_eq!(rows[1].cells, vec!["Pemesanan", "Pembayaran & Refund", "1 : 0..N"]);
        assert_eq!(rows[2].cells[2], "0..1 : 0..N");
    }

    #[test]
    fn chatgpt_html_and_markdown_agree() {
        let a = from_html(CHATGPT_HTML).unwrap();
        let b = from_markdown(CHATGPT_MD).unwrap();
        assert_eq!(a, b);
        assert_eq!(a[2].cells, vec!["Pascakondisi", "Ringkasan periode terpilih ditampilkan"]);
    }

    #[test]
    fn ignores_non_tables() {
        assert!(from_markdown("Halo\nini teks biasa").is_none());
        assert!(from_markdown("Penjelasan:\n| a | b |\n|---|---|\n| 1 | 2 |").is_none(), "ada teks selain tabel");
        assert!(from_html("<p>Teks pembuka</p><table><tr><td>a</td><td>b</td></tr><tr><td>1</td><td>2</td></tr></table>").is_none());
        assert!(from_html("<table><tr><td>satu kolom</td></tr><tr><td>x</td></tr></table>").is_none());
        assert!(from_html(r#"<table data-leo="1"><tr><td>a</td><td>b</td></tr><tr><td>1</td><td>2</td></tr></table>"#).is_none());
        assert!(from_html(r#"<table data-amora="1"><tr><td>a</td><td>b</td></tr><tr><td>1</td><td>2</td></tr></table>"#).is_none(), "penanda lama tetap dikenali");
        assert!(from_html(r#"<html xmlns:o="urn:schemas-microsoft-com:office:office"><table><tr><td>a</td><td>b</td></tr><tr><td>1</td><td>2</td></tr></table>"#).is_none());
    }

    #[test]
    fn widths_follow_content() {
        let rows = from_markdown(CHATGPT_MD).unwrap();
        let w = column_widths(&rows);
        assert_eq!(w.iter().sum::<u32>(), 100);
        assert!(w[1] > w[0] * 2, "kolom isi jauh lebih lebar: {w:?}");
        // Kolom sempit tetap muat kata judulnya.
        let rows = from_html(CLAUDE).unwrap();
        let w = column_widths(&rows);
        assert!(w.iter().all(|&p| p >= 15), "{w:?}");
    }

    #[test]
    fn cf_html_roundtrip() {
        let html = to_html(&from_markdown(CHATGPT_MD).unwrap());
        let cf = cf_html(&html);
        assert_eq!(cf_html_fragment(&cf).unwrap(), html);
        assert!(html.contains("border:1pt solid") && html.contains("data-leo"));
    }

    #[test]
    fn escaped_pipes_and_bold() {
        let rows = from_markdown("| **Kunci** | Nilai |\n|:---|---:|\n| a \\| b | `x` |").unwrap();
        assert_eq!(rows[0].cells[0], "Kunci");
        assert_eq!(rows[1].cells, vec!["a | b", "x"]);
    }
}
