//! File search through a running Everything (voidtools) instance, using its
//! WM_COPYDATA IPC protocol (QUERY2, Everything 1.4+). No SDK DLL required.

use std::{
    cell::RefCell,
    ffi::c_void,
    path::PathBuf,
    sync::Once,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{DataExchange::COPYDATASTRUCT, LibraryLoader::GetModuleHandleW},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, MSG,
            MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, RegisterClassW,
            SendMessageW, TranslateMessage, WINDOW_EX_STYLE, WM_COPYDATA, WNDCLASSW, WS_OVERLAPPED,
        },
    },
    core::{PCWSTR, w},
};

const EVERYTHING_CLASS: PCWSTR = w!("EVERYTHING_TASKBAR_NOTIFICATION");
const REPLY_CLASS: PCWSTR = w!("LovelyLauncher.EverythingReply");
const COPYDATA_QUERY2W: usize = 18;
const REPLY_ID: usize = 0x4C4C_0001;
const REQUEST_FULL_PATH_AND_FILE_NAME: u32 = 0x0000_0004;
/// Most recently modified first — a decent proxy for relevance in a launcher.
const SORT_DATE_MODIFIED_DESCENDING: u32 = 14;
const ITEM_FOLDER: u32 = 0x1;
const TIMEOUT: Duration = Duration::from_millis(1500);

/// Path fragments of system / scratch locations that only add noise to results.
const NOISE: &[&str] = &[
    r"\windows\winsxs\",
    r"\windows\prefetch\",
    r"\windows\servicing\",
    r"\windows\installer\",
    r"\windows\softwaredistribution\",
    r"\$recycle.bin\",
    r"ppdata\local	emp\",
    r"ppdata\local\microsoft\windows\inetcache\",
    r"\system volume information\",
    // Developer caches and dependency trees
    r"\node_modules\",
    r"\.git\",
    r"\.cache\",
    r"\__pycache__\",
    r"\.vs\",
];

/// Drop noisy system paths, then put names that start with the query first
/// (keeping Everything's recency order within each group).
pub fn rank(hits: Vec<FileHit>, query: &str, limit: usize) -> Vec<FileHit> {
    let q = query.trim().to_lowercase();
    let mut hits: Vec<FileHit> = hits
        .into_iter()
        .filter(|h| {
            let p = format!("{}\\", h.path.display()).to_lowercase();
            !NOISE.iter().any(|n| p.contains(n))
        })
        .collect();
    let starts = |h: &FileHit| {
        h.path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with(&q))
    };
    hits.sort_by_key(|h| !starts(h));
    hits.truncate(limit);
    hits
}

#[derive(Clone, Debug)]
pub struct FileHit {
    pub path: PathBuf,
    pub is_dir: bool,
}

thread_local! {
    static REPLY: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

pub fn is_running() -> bool {
    unsafe { FindWindowW(EVERYTHING_CLASS, PCWSTR::null()) }.is_ok_and(|h| !h.is_invalid())
}

unsafe extern "system" fn reply_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_COPYDATA {
        let cds = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
        if cds.dwData == REPLY_ID && !cds.lpData.is_null() {
            let bytes = unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) };
            REPLY.with(|r| *r.borrow_mut() = Some(bytes.to_vec()));
            return LRESULT(1);
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn register_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(reply_proc),
            hInstance: GetModuleHandleW(PCWSTR::null()).unwrap_or_default().into(),
            lpszClassName: REPLY_CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);
    });
}

/// Search Everything for `query`. Blocking (up to ~1.5 s); run on a background thread.
pub fn search(query: &str, max_results: u32) -> Result<Vec<FileHit>, String> {
    let everything = unsafe { FindWindowW(EVERYTHING_CLASS, PCWSTR::null()) }
        .ok()
        .filter(|h| !h.is_invalid())
        .ok_or("Everything 未运行")?;

    register_class();
    let reply_hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            REPLY_CLASS,
            PCWSTR::null(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            GetModuleHandleW(PCWSTR::null()).ok().map(Into::into),
            None,
        )
    }
    .map_err(|e| format!("无法创建 Everything 通信窗口：{}", e.message()))?;

    let result = query_with(everything, reply_hwnd, query, max_results);
    unsafe {
        let _ = DestroyWindow(reply_hwnd);
    }
    REPLY.with(|r| r.borrow_mut().take());
    result
}

fn query_with(everything: HWND, reply_hwnd: HWND, query: &str, max_results: u32) -> Result<Vec<FileHit>, String> {
    // EVERYTHING_IPC_QUERY2 header followed by the null-terminated UTF-16 search string.
    let mut packet: Vec<u8> = Vec::new();
    for value in [
        reply_hwnd.0 as usize as u32,
        REPLY_ID as u32,
        0, // search flags
        0, // offset
        max_results,
        REQUEST_FULL_PATH_AND_FILE_NAME,
        SORT_DATE_MODIFIED_DESCENDING,
    ] {
        packet.extend_from_slice(&value.to_le_bytes());
    }
    for unit in query.encode_utf16().chain(std::iter::once(0u16)) {
        packet.extend_from_slice(&unit.to_le_bytes());
    }

    REPLY.with(|r| r.borrow_mut().take());
    let cds = COPYDATASTRUCT {
        dwData: COPYDATA_QUERY2W,
        cbData: packet.len() as u32,
        lpData: packet.as_mut_ptr() as *mut c_void,
    };
    let accepted = unsafe {
        SendMessageW(
            everything,
            WM_COPYDATA,
            Some(WPARAM(reply_hwnd.0 as usize)),
            Some(LPARAM(&cds as *const _ as isize)),
        )
    };
    if accepted.0 == 0 {
        return Err("Everything 拒绝了查询请求".into());
    }

    // The reply arrives as a WM_COPYDATA sent to our window; pump until it does.
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(bytes) = REPLY.with(|r| r.borrow_mut().take()) {
            return parse_list2(&bytes);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err("Everything 响应超时".into());
        }
        unsafe {
            MsgWaitForMultipleObjects(None, false, (deadline - now).as_millis() as u32, QS_ALLINPUT);
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Parse EVERYTHING_IPC_LIST2: 5 header DWORDs, then (flags, data_offset) per item;
/// each item's data is a DWORD length followed by that many UTF-16 units.
fn parse_list2(bytes: &[u8]) -> Result<Vec<FileHit>, String> {
    let bad = || "Everything 返回了无法识别的数据".to_string();
    let numitems = read_u32(bytes, 4).ok_or_else(bad)? as usize;
    let mut hits = Vec::with_capacity(numitems);
    for i in 0..numitems {
        let item = 20 + i * 8;
        let flags = read_u32(bytes, item).ok_or_else(bad)?;
        let data = read_u32(bytes, item + 4).ok_or_else(bad)? as usize;
        let len = read_u32(bytes, data).ok_or_else(bad)? as usize;
        let start = data + 4;
        let units: Vec<u16> = bytes
            .get(start..start + len * 2)
            .ok_or_else(bad)?
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        hits.push(FileHit {
            path: PathBuf::from(String::from_utf16_lossy(&units)),
            is_dir: flags & ITEM_FOLDER != 0,
        });
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reply() {
        let mut b = Vec::new();
        for v in [1u32, 1, 0, REQUEST_FULL_PATH_AND_FILE_NAME, 1] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&ITEM_FOLDER.to_le_bytes());
        b.extend_from_slice(&28u32.to_le_bytes()); // data offset = 20 + 8
        let path: Vec<u16> = "C:\\a".encode_utf16().collect();
        b.extend_from_slice(&(path.len() as u32).to_le_bytes());
        for u in path.iter().chain([0u16].iter()) {
            b.extend_from_slice(&u.to_le_bytes());
        }
        let hits = parse_list2(&b).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].is_dir);
        assert_eq!(hits[0].path, PathBuf::from("C:\\a"));
    }
}

#[cfg(test)]
mod rank_tests {
    use super::*;

    #[test]
    fn filters_and_orders() {
        let hit = |p: &str| FileHit { path: PathBuf::from(p), is_dir: false };
        let ranked = rank(
            vec![
                hit(r"C:\Windows\WinSxS\amd64_notepad\notepad.exe"),
                hit(r"D:\docs\my notes.txt"),
                hit(r"D:\docs\notes.md"),
            ],
            "note",
            10,
        );
        let names: Vec<_> = ranked.iter().map(|h| h.path.display().to_string()).collect();
        assert_eq!(names, vec![r"D:\docs\notes.md", r"D:\docs\my notes.txt"]);
    }
}
