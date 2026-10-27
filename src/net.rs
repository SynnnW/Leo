//! HTTPS lewat WinHTTP dengan metode, header, dan isi (body) bebas, untuk Supabase.
//! (Pencari lirik punya klien GET sendiri yang lebih sederhana di `lyrics/http.rs`.)

#[cfg(windows)]
use std::ffi::c_void;

#[cfg(windows)]
use windows::core::{HSTRING, PCWSTR};
#[cfg(windows)]
use windows::Win32::Networking::WinHttp::*;

/// Pecah `https://host/path?query` jadi (host, path+query). Hanya HTTPS tanpa port khusus.
pub fn split_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    (!host.is_empty() && !host.contains(':')).then(|| (host.to_string(), path.to_string()))
}

#[cfg(windows)]
struct Handle(*mut c_void);

#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
/// Kirim satu permintaan. Mengembalikan (kode status, isi sebagai teks); galat jaringan jadi `Err`.
pub fn request(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&str>) -> Result<(u16, String), String> {
    let (host, path) = split_url(url).ok_or("URL harus https://...")?;
    unsafe {
        let session = Handle(WinHttpOpen(
            &HSTRING::from(concat!("Leo/", env!("CARGO_PKG_VERSION"))),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err("WinHttpOpen gagal".into());
        }
        let _ = WinHttpSetTimeouts(session.0, 8000, 8000, 10000, 15000);
        let connect = Handle(WinHttpConnect(session.0, &HSTRING::from(host.as_str()), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if connect.0.is_null() {
            return Err("tidak bisa terhubung ke server".into());
        }
        let req = Handle(WinHttpOpenRequest(
            connect.0,
            &HSTRING::from(method),
            &HSTRING::from(path.as_str()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null_mut(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err("WinHttpOpenRequest gagal".into());
        }

        let header_text: Vec<u16> = headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect::<String>()
            .encode_utf16()
            .collect();
        let body_bytes = body.map(str::as_bytes).unwrap_or(&[]);
        WinHttpSendRequest(
            req.0,
            if header_text.is_empty() { None } else { Some(&header_text) },
            if body_bytes.is_empty() { None } else { Some(body_bytes.as_ptr() as *const c_void) },
            body_bytes.len() as u32,
            body_bytes.len() as u32,
            0,
        )
        .map_err(|e| format!("kirim gagal: {e}"))?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|e| format!("tidak ada balasan: {e}"))?;

        let mut status: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("status tidak terbaca: {e}"))?;

        let mut out = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read)
                .map_err(|e| format!("baca gagal: {e}"))?;
            if read == 0 {
                break;
            }
            out.extend_from_slice(&buf[..read as usize]);
        }
        Ok((status as u16, String::from_utf8_lossy(&out).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_https_urls() {
        assert_eq!(
            split_url("https://abc.supabase.co/rest/v1/x?select=id"),
            Some(("abc.supabase.co".into(), "/rest/v1/x?select=id".into()))
        );
        assert_eq!(split_url("https://abc.supabase.co"), Some(("abc.supabase.co".into(), "/".into())));
        assert_eq!(split_url("http://abc.co/x"), None, "hanya HTTPS");
        assert_eq!(split_url("https://abc.co:8443/x"), None, "port khusus tidak didukung");
        assert_eq!(split_url("https:///x"), None);
    }
}
