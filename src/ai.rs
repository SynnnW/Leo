//! Peluncur chat AI (Claude, Gemini, ChatGPT) tanpa token API: membuka situs masing-masing
//! sebagai jendela aplikasi Edge/Chrome. Tiap profil punya folder data browser sendiri,
//! jadi login (mis. akun Google yang berbeda) tersimpan terpisah dan bisa berganti akun
//! tanpa keluar-masuk akun.

use std::path::{Path, PathBuf};

pub const SERVICES: [(&str, &str); 3] = [
    ("Claude", "https://claude.ai"),
    ("Gemini", "https://gemini.google.com"),
    ("ChatGPT", "https://chatgpt.com"),
];

/// Folder data browser satu profil, mis. `<data>\ai\utama`. Nama diubah jadi aman untuk nama folder.
pub fn profile_dir(base: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '_' })
        .collect();
    base.join("ai").join(if safe.is_empty() { "profil".to_string() } else { safe })
}

/// Argumen untuk membuka `url` sebagai jendela aplikasi dengan data browser di `dir`.
pub fn app_args(url: &str, dir: &Path) -> Vec<String> {
    vec![
        format!("--app={url}"),
        format!("--user-data-dir={}", dir.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
    ]
}

fn browser_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for var in ["PROGRAMFILES(X86)", "PROGRAMFILES", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(var) {
            let root = PathBuf::from(root);
            v.push(root.join(r"Microsoft\Edge\Application\msedge.exe"));
            v.push(root.join(r"Google\Chrome\Application\chrome.exe"));
        }
    }
    v
}

/// Buka layanan ke-`service` (indeks `SERVICES`) memakai profil `profile`. `true` bila berhasil dimulai.
pub fn launch(service: usize, profile: &str) -> bool {
    let Some((_, url)) = SERVICES.get(service) else { return false };
    let Some(base) = crate::settings::data_dir() else { return false };
    let dir = profile_dir(&base, profile);
    let _ = std::fs::create_dir_all(&dir);
    match browser_candidates().into_iter().find(|p| p.is_file()) {
        Some(exe) => std::process::Command::new(exe).args(app_args(url, &dir)).spawn().is_ok(),
        // Tanpa Edge/Chrome: buka di browser bawaan (tanpa profil terpisah).
        None => open_default(url),
    }
}

#[cfg(windows)]
fn open_default(url: &str) -> bool {
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), None, None, SW_SHOWNORMAL).0 as isize > 32 }
}

#[cfg(not(windows))]
fn open_default(_url: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_become_safe_folders() {
        let base = Path::new("data");
        assert_eq!(profile_dir(base, "Utama"), base.join("ai").join("utama"));
        assert_eq!(profile_dir(base, "Akun Kedua/../x"), base.join("ai").join("akun_kedua____x"));
        assert_eq!(profile_dir(base, ""), base.join("ai").join("profil"));
    }

    #[test]
    fn app_args_use_separate_user_data_dir() {
        let a = app_args("https://claude.ai", Path::new("d"));
        assert_eq!(a[0], "--app=https://claude.ai");
        assert!(a[1].starts_with("--user-data-dir="));
    }

    #[test]
    fn unknown_service_does_not_launch() {
        assert!(!launch(99, "Utama"));
    }
}
