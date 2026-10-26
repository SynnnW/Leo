//! HTTPS GET minimal lewat WinHTTP. Lebih ringan daripada Windows.Web.Http
//! karena tidak memuat tumpukan WinRT.

use std::ffi::c_void;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

pub struct Http {
    session: *mut c_void,
    connect: *mut c_void,
}

// Handle WinHTTP aman dipakai dari thread mana pun; kita hanya memakainya di satu thread.
unsafe impl Send for Http {}

impl Http {
    pub fn new(host: &str, user_agent: &str) -> Option<Self> {
        unsafe {
            let session = WinHttpOpen(
                &HSTRING::from(user_agent),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            );
            if session.is_null() {
                return None;
            }
            // Jangan sampai pencarian lirik menggantung lama.
            let _ = WinHttpSetTimeouts(session, 5000, 5000, 8000, 8000);
            let connect = WinHttpConnect(session, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0);
            if connect.is_null() {
                let _ = WinHttpCloseHandle(session);
                return None;
            }
            Some(Self { session, connect })
        }
    }

    /// GET `path` (mis. "/api/get?..."). `None` bila gagal atau status bukan 200.
    pub fn get(&self, path: &str) -> Option<String> {
        unsafe {
            let req = WinHttpOpenRequest(
                self.connect,
                &HSTRING::from("GET"),
                &HSTRING::from(path),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null_mut(),
                WINHTTP_FLAG_SECURE,
            );
            if req.is_null() {
                return None;
            }
            let body = Self::exchange(req);
            let _ = WinHttpCloseHandle(req);
            body
        }
    }

    unsafe fn exchange(req: *mut c_void) -> Option<String> {
        WinHttpSendRequest(req, None, None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(req, std::ptr::null_mut()).ok()?;

        let mut status: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            req,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut len,
            std::ptr::null_mut(),
        )
        .ok()?;
        if status != 200 {
            return None;
        }

        let mut out = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(req, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read).ok()?;
            if read == 0 {
                break;
            }
            out.extend_from_slice(&buf[..read as usize]);
        }
        String::from_utf8(out).ok()
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.connect);
            let _ = WinHttpCloseHandle(self.session);
        }
    }
}
