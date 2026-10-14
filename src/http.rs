//! Minimal blocking HTTP(S) client on top of WinHTTP (ships with Windows, so no
//! TLS / HTTP crates). Responses are streamed to a callback, which suits both file
//! downloads and server-sent events. Run it on a background thread.

use std::ffi::c_void;

use windows::{
    Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPEN_REQUEST_FLAGS,
        WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
        WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    },
    core::{HSTRING, PCWSTR},
};

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn handle(raw: *mut c_void, what: &str) -> Result<Handle, String> {
    if raw.is_null() {
        Err(format!("{what}失败：{}", windows::core::Error::from_win32().message()))
    } else {
        Ok(Handle(raw))
    }
}

pub struct ParsedUrl {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn parse_url(url: &str) -> Result<ParsedUrl, String> {
    let url = url.trim();
    let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err(format!("不支持的地址：{url}"));
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().map_err(|_| format!("端口无效：{p}"))?),
        None => (authority, if secure { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(format!("地址无效：{url}"));
    }
    Ok(ParsedUrl {
        secure,
        host: host.to_string(),
        port,
        path: path.to_string(),
    })
}

pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: Option<&'a [u8]>,
    /// Max wait for the server between bytes (LLMs can pause for a while).
    pub receive_timeout_ms: i32,
}

/// What a response handler saw: status code and `Content-Length` if sent.
pub struct ResponseHead {
    pub status: u32,
    pub content_length: Option<u64>,
}

/// Send `req`, call `on_head` once, then `on_data` for every received chunk.
/// Returning `false` from `on_data` stops reading early (e.g. on cancel).
pub fn send(
    req: &Request,
    on_head: impl FnOnce(&ResponseHead),
    mut on_data: impl FnMut(&[u8]) -> bool,
) -> Result<u32, String> {
    let url = parse_url(req.url)?;
    unsafe {
        let session = handle(
            WinHttpOpen(
                &HSTRING::from(concat!("LovelyLauncher/", env!("CARGO_PKG_VERSION"))),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "初始化网络",
        )?;
        let _ = WinHttpSetTimeouts(session.0, 0, 30_000, 60_000, req.receive_timeout_ms);
        let connect = handle(
            WinHttpConnect(session.0, &HSTRING::from(url.host.as_str()), url.port, 0),
            "连接服务器",
        )?;
        let flags = if url.secure {
            WINHTTP_FLAG_SECURE
        } else {
            WINHTTP_OPEN_REQUEST_FLAGS(0)
        };
        let request = handle(
            WinHttpOpenRequest(
                connect.0,
                &HSTRING::from(req.method),
                &HSTRING::from(url.path.as_str()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                flags,
            ),
            "创建请求",
        )?;

        let headers: String = req
            .headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect();
        let headers: Vec<u16> = headers.encode_utf16().collect();
        let body = req.body.unwrap_or_default();
        WinHttpSendRequest(
            request.0,
            (!headers.is_empty()).then_some(headers.as_slice()),
            (!body.is_empty()).then_some(body.as_ptr() as *const c_void),
            body.len() as u32,
            body.len() as u32,
            0,
        )
        .map_err(|e| format!("发送请求失败：{}", e.message()))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| format!("服务器无响应：{}", e.message()))?;

        let status = query_number(request.0, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
        on_head(&ResponseHead {
            status,
            content_length: query_number(request.0, WINHTTP_QUERY_CONTENT_LENGTH).map(u64::from),
        });

        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read)
                .map_err(|e| format!("连接中断：{}", e.message()))?;
            if read == 0 || !on_data(&buf[..read as usize]) {
                break;
            }
        }
        Ok(status)
    }
}

/// Convenience: send and collect the whole body (capped at `limit` bytes).
pub fn fetch(req: &Request, limit: usize) -> Result<(u32, Vec<u8>), String> {
    let mut body = Vec::new();
    let status = send(req, |_| {}, |chunk| {
        let room = limit.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
        body.len() < limit
    })?;
    Ok((status, body))
}

unsafe fn query_number(request: *mut c_void, info: u32) -> Option<u32> {
    let mut value = 0u32;
    let mut len = std::mem::size_of::<u32>() as u32;
    unsafe {
        WinHttpQueryHeaders(
            request,
            info | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut value as *mut u32 as *mut c_void),
            &mut len,
            std::ptr::null_mut(),
        )
    }
    .ok()
    .map(|_| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        let u = parse_url("https://example.com/a/b.exe").unwrap();
        assert!(u.secure);
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("example.com", 443, "/a/b.exe"));
        let u = parse_url("http://localhost:8080").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("localhost", 8080, "/"));
        assert!(parse_url("ftp://x").is_err());
    }
}
