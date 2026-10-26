//! Membersihkan judul dari pemutar (terutama judul video YouTube) menjadi
//! pasangan (judul lagu, artis) yang bisa dicari di LRCLIB, dan parser LRC.

/// Kata yang menandakan isi kurung hanyalah keterangan video, bukan bagian judul lagu.
const NOISE: &[&str] = &[
    "official", "video", "audio", "lyric", "lirik", "mv", "m/v", "visualizer", "visualiser",
    "hd", "hq", "4k", "clip", "color coded", "sub indo", "eng sub", "full album", "teaser",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub track: String,
    pub artist: String,
}

fn has_noise(s: &str) -> bool {
    let l = s.to_lowercase();
    NOISE.iter().any(|n| {
        l.match_indices(n).any(|(i, _)| {
            // Cocokkan per kata supaya "hd" tidak kena di "shadow".
            let before = l[..i].chars().last();
            let after = l[i + n.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    })
}

/// Buang segmen dalam kurung yang berisi keterangan video, mis. "(Official Video)", "[MV]".
fn strip_noise_brackets(s: &str) -> String {
    const PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('【', '】'), ('「', '」')];
    let mut out = String::new();
    let mut rest = s;
    'outer: while !rest.is_empty() {
        for &(open, close) in PAIRS {
            if rest.starts_with(open) {
                if let Some(end) = rest.find(close) {
                    let inner = &rest[open.len_utf8()..end];
                    if has_noise(inner) {
                        rest = &rest[end + close.len_utf8()..];
                        continue 'outer;
                    }
                }
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn tidy(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(|c: char| c == '-' || c == '|' || c.is_whitespace()).to_string()
}

/// Hapus "feat. X" / "ft. X" (beserta kurungnya bila ada).
fn strip_feat(s: &str) -> String {
    // ASCII-lowercase supaya indeks byte tetap sama dengan `s`.
    let l = s.to_ascii_lowercase();
    for key in ["(feat.", "(ft.", "[feat.", "[ft.", " feat.", " ft.", " featuring "] {
        if let Some(i) = l.find(key) {
            let after = &s[i + key.len()..];
            let tail = if key.starts_with('(') || key.starts_with('[') {
                let close = if key.starts_with('(') { ')' } else { ']' };
                after.find(close).map(|j| &after[j + 1..]).unwrap_or("")
            } else {
                // Tanpa kurung: buang sampai akhir atau sampai kurung berikutnya.
                after.find(['(', '[']).map(|j| &after[j..]).unwrap_or("")
            };
            return tidy(&format!("{}{}", &s[..i], tail));
        }
    }
    tidy(s)
}

/// Nama channel YouTube → nama artis: "RickAstleyVEVO" → "RickAstley", "Hindia - Topic" → "Hindia".
pub fn clean_artist(a: &str) -> String {
    let mut s = a.trim().to_string();
    for suf in [" - Topic", "VEVO", " Official", "Official"] {
        let cut = s.len().saturating_sub(suf.len());
        if cut > 0 && s.is_char_boundary(cut) && s[cut..].eq_ignore_ascii_case(suf) {
            s.truncate(cut);
        }
    }
    tidy(&split_camel(&s))
}

/// "RickAstley" → "Rick Astley" (nama channel tanpa spasi).
fn split_camel(s: &str) -> String {
    if s.contains(' ') || !s.chars().all(char::is_alphanumeric) {
        return s.to_string();
    }
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_lowercase();
        out.push(c);
    }
    out
}

fn clean_title(t: &str) -> String {
    let mut s = strip_noise_brackets(t);
    // "Judul | Channel" atau "Judul // keterangan"
    for sep in [" | ", " // ", " ｜ "] {
        if let Some(i) = s.find(sep) {
            s.truncate(i);
        }
    }
    tidy(&s)
}

fn split_dash(s: &str) -> Option<(String, String)> {
    for sep in [" - ", " – ", " — ", " ~ "] {
        if let Some(i) = s.find(sep) {
            let (a, b) = (tidy(&s[..i]), tidy(&s[i + sep.len()..]));
            if !a.is_empty() && !b.is_empty() {
                return Some((a, b));
            }
        }
    }
    None
}

fn first_artist(a: &str) -> String {
    let l = a.to_ascii_lowercase();
    let mut cut = a.len();
    for sep in [", ", " & ", " x ", " feat. ", " ft. ", " dan "] {
        if let Some(i) = l.find(sep) {
            cut = cut.min(i);
        }
    }
    tidy(&a[..cut])
}

/// Kandidat pencarian, urut dari yang paling mungkin benar.
pub fn candidates(title: &str, artist: &str) -> Vec<Query> {
    let t = clean_title(title);
    let a = clean_artist(artist);
    let mut out: Vec<Query> = Vec::new();
    let mut push = |track: String, artist: String| {
        let q = Query { track, artist };
        if !q.track.is_empty() && !q.artist.is_empty() && !out.contains(&q) {
            out.push(q);
        }
    };

    if let Some((left, right)) = split_dash(&t) {
        // Format paling umum di YouTube: "Artis - Judul".
        push(strip_feat(&right), first_artist(&left));
        push(strip_feat(&t), a.clone());
        // Kadang terbalik: "Judul - Artis".
        push(strip_feat(&left), first_artist(&right));
    } else {
        push(t.clone(), a.clone());
        push(strip_feat(&t), first_artist(&a));
    }
    out
}

/// Teks bebas untuk /api/search?q= sebagai upaya terakhir.
pub fn free_query(title: &str, artist: &str) -> String {
    let t = strip_feat(&clean_title(title));
    if split_dash(&t).is_some() {
        t.replace(" - ", " ")
    } else {
        format!("{} {}", clean_artist(artist), t)
    }
}

/// Seberapa mirip dua judul (0 = sama). Dipakai untuk memilih hasil pencarian.
pub fn title_distance(a: &str, b: &str) -> u32 {
    let norm = |s: &str| {
        strip_feat(&strip_noise_brackets(s))
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
    };
    let (a, b) = (norm(a), norm(b));
    if a == b {
        0
    } else if a.contains(&b) || b.contains(&a) {
        1
    } else {
        3
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub t: f64,
    pub text: String,
}

fn parse_ts(tag: &str) -> Option<f64> {
    let (m, s) = tag.split_once(':')?;
    let m: f64 = m.trim().parse().ok()?;
    let s: f64 = s.trim().parse().ok()?;
    Some(m * 60.0 + s)
}

/// Parse LRC: "[01:02.34] teks". Satu baris bisa punya beberapa timestamp.
pub fn parse_lrc(src: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            match parse_ts(&rest[1..end]) {
                Some(t) => times.push(t),
                None => break, // tag metadata seperti [ar:...]
            }
            rest = &rest[end + 1..];
        }
        for t in times {
            out.push(Line { t, text: rest.trim().to_string() });
        }
    }
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(t: &str, a: &str) -> Query {
        Query { track: t.into(), artist: a.into() }
    }

    #[test]
    fn youtube_artist_dash_title() {
        let c = candidates("Hindia - Evaluasi (Official Lyric Video)", "Hindia");
        assert_eq!(c[0], q("Evaluasi", "Hindia"));
    }

    #[test]
    fn vevo_channel() {
        let c = candidates("Rick Astley - Never Gonna Give You Up (Official Music Video)", "RickAstleyVEVO");
        assert_eq!(c[0], q("Never Gonna Give You Up", "Rick Astley"));
        assert_eq!(clean_artist("RickAstleyVEVO"), "Rick Astley");
        assert_eq!(clean_artist("TULUS"), "TULUS");
    }

    #[test]
    fn topic_channel_clean_title() {
        let c = candidates("Evaluasi", "Hindia - Topic");
        assert_eq!(c[0], q("Evaluasi", "Hindia"));
    }

    #[test]
    fn brackets_mv_and_pipe() {
        let c = candidates("NewJeans (뉴진스) 'Super Shy' [MV] | HYBE LABELS", "HYBE LABELS");
        assert_eq!(c[0].track, "NewJeans (뉴진스) 'Super Shy'");
    }

    #[test]
    fn feat_removed() {
        let c = candidates("Tulus - Hati-Hati di Jalan (feat. Someone) [Official Audio]", "TULUS");
        assert_eq!(c[0], q("Hati-Hati di Jalan", "Tulus"));
    }

    #[test]
    fn spotify_clean_passes_through() {
        let c = candidates("To Let A Good Thing Die", "Bruno Major");
        assert_eq!(c[0], q("To Let A Good Thing Die", "Bruno Major"));
    }

    #[test]
    fn multi_artist_fallback() {
        let c = candidates("Somebody", "Artist A, Artist B");
        assert!(c.contains(&q("Somebody", "Artist A")));
    }

    #[test]
    fn noise_word_boundary() {
        assert!(!has_noise("Shadow"));
        assert!(has_noise("Official HD Video"));
    }

    #[test]
    fn lrc_parse() {
        let l = parse_lrc("[ar:X]\n[00:12.50]halo\n[00:05.00][01:00.00]lagi\n[00:20.00]");
        assert_eq!(l.len(), 4);
        assert_eq!(l[0], Line { t: 5.0, text: "lagi".into() });
        assert_eq!(l[1].text, "halo");
        assert_eq!(l[2].text, "");
        assert_eq!(l[3].t, 60.0);
    }

    #[test]
    fn distance() {
        assert_eq!(title_distance("Evaluasi", "evaluasi"), 0);
        assert_eq!(title_distance("Evaluasi", "Evaluasi (Live)"), 1);
        assert_eq!(title_distance("Evaluasi", "Membasuh"), 3);
    }
}
