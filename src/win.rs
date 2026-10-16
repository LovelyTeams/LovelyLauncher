//! Thin Win32 helpers: window show/hide, launching, icon extraction,
//! Start Menu scanning, single-instance signalling and autostart.

use std::{
    collections::HashSet,
    ffi::c_void,
    path::{Path, PathBuf},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, POINT, RECT},
        Graphics::Gdi::{
            GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
            MonitorFromWindow, BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
            GetDIBits, GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC,
        },
        Storage::FileSystem::{FILE_ATTRIBUTE_NORMAL, SearchPathW},
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
            Environment::ExpandEnvironmentStringsW,
            ProcessStatus::EmptyWorkingSet,
            Threading::{
                CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, GetCurrentProcess, INFINITE,
                OpenEventW, SetEvent, WaitForSingleObject,
            },
        },
        UI::{
            Controls::IImageList,
            HiDpi::{GetDpiForMonitor, GetDpiForSystem, MDT_EFFECTIVE_DPI},
            Shell::{
                SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, SHFILEINFOW,
                SHGFI_DISPLAYNAME, SHGFI_FLAGS, SHGFI_SYSICONINDEX, SHGFI_TYPENAME, SHGFI_USEFILEATTRIBUTES,
                SHGetFileInfoW, SHGetImageList,
                SHIL_EXTRALARGE, ShellExecuteExW,
            },
            WindowsAndMessaging::{
                ASFW_ANY, AllowSetForegroundWindow, BringWindowToTop, DestroyIcon, GetIconInfo,
                ICONINFO, IsIconic, IsWindowVisible, SPI_GETWORKAREA, SW_HIDE, SW_MAXIMIZE, SW_RESTORE,
                SW_SHOW,
                SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SWP_SHOWWINDOW,
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, GetClientRect, GetCursorPos, GetWindowRect,
                IsZoomed, SetForegroundWindow, SetWindowPos, ShowWindow, SystemParametersInfoW,
            },
        },
    },
    core::{HSTRING, PCWSTR},
};

const MUTEX_NAME: &str = "Local\\LovelyLauncher.SingleInstance";
const SHOW_EVENT_NAME: &str = "Local\\LovelyLauncher.Show";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "LovelyLauncher";

fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut c_void)
}

// ---------------------------------------------------------------------------
// Window visibility

pub fn show_window(raw: isize) {
    let h = hwnd(raw);
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        } else {
            let _ = ShowWindow(h, SW_SHOW);
        }
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
    }
}

pub fn hide_window(raw: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(raw), SW_HIDE);
    }
}

pub fn is_window_visible(raw: isize) -> bool {
    let h = hwnd(raw);
    unsafe { IsWindowVisible(h).as_bool() && !IsIconic(h).as_bool() }
}

/// Primary monitor work area (excluding the taskbar) in logical pixels: (x, y, w, h).
pub fn primary_work_area() -> Option<(f32, f32, f32, f32)> {
    let mut rect = RECT::default();
    unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .ok()?;
        let scale = GetDpiForSystem().max(96) as f32 / 96.;
        Some((
            rect.left as f32 / scale,
            rect.top as f32 / scale,
            (rect.right - rect.left) as f32 / scale,
            (rect.bottom - rect.top) as f32 / scale,
        ))
    }
}

// ---------------------------------------------------------------------------
// Window geometry (physical pixels unless noted)

#[derive(Clone, Copy, Debug)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub maximized: bool,
}

pub fn window_rect(raw: isize) -> Option<WindowRect> {
    let h = hwnd(raw);
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) }.ok()?;
    Some(WindowRect {
        x: r.left,
        y: r.top,
        w: r.right - r.left,
        h: r.bottom - r.top,
        maximized: unsafe { IsZoomed(h).as_bool() },
    })
}

pub fn set_window_rect(raw: isize, rect: WindowRect) {
    let h = hwnd(raw);
    unsafe {
        if IsZoomed(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetWindowPos(h, None, rect.x, rect.y, rect.w, rect.h, SWP_NOZORDER | SWP_NOACTIVATE);
        if rect.maximized {
            let _ = ShowWindow(h, SW_MAXIMIZE);
        }
    }
}

/// Window size minus client size: the invisible resize borders around our content.
fn frame_delta(h: HWND) -> (i32, i32) {
    let (mut wr, mut cr) = (RECT::default(), RECT::default());
    unsafe {
        if GetWindowRect(h, &mut wr).is_err() || GetClientRect(h, &mut cr).is_err() {
            return (0, 0);
        }
    }
    (
        (wr.right - wr.left) - (cr.right - cr.left),
        (wr.bottom - wr.top) - (cr.bottom - cr.top),
    )
}

/// Place the window as a compact bar: `width` x `height` logical pixels of content,
/// centred horizontally, a fifth of the way down the monitor under the cursor.
pub fn place_mini(raw: isize, width: f32, height: f32) {
    let h = hwnd(raw);
    unsafe {
        if IsZoomed(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x.max(96) as f32 / 96.;
        let (dw, dh) = frame_delta(h);
        let work = info.rcWork;
        let (work_w, work_h) = (work.right - work.left, work.bottom - work.top);
        let w = ((width * scale) as i32 + dw).min(work_w);
        let ht = (height * scale) as i32 + dh;
        let x = work.left + (work_w - w) / 2;
        let y = work.top + work_h / 5;
        let _ = SetWindowPos(h, None, x, y, w, ht, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Show a window without activating it, centred on the monitor of `anchor`
/// (or the cursor), with `width` x `height` logical pixels of content.
pub fn place_window(raw: isize, width: f32, height: f32, anchor: Option<isize>) {
    let h = hwnd(raw);
    unsafe {
        let monitor: HMONITOR = match anchor {
            Some(a) => MonitorFromWindow(hwnd(a), MONITOR_DEFAULTTONEAREST),
            None => {
                let mut cursor = POINT::default();
                let _ = GetCursorPos(&mut cursor);
                MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST)
            }
        };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x.max(96) as f32 / 96.;
        let (dw, dh) = frame_delta(h);
        let work = info.rcWork;
        let (work_w, work_h) = (work.right - work.left, work.bottom - work.top);
        let w = ((width * scale) as i32 + dw).min(work_w * 9 / 10);
        let ht = ((height * scale) as i32 + dh).min(work_h * 9 / 10);
        let x = work.left + (work_w - w) / 2;
        let y = work.top + (work_h - ht) / 2;
        let _ = SetWindowPos(h, None, x, y, w, ht, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

/// Maximize the window, or put it back if it is maximized.
pub fn toggle_maximize(raw: isize) {
    use windows::Win32::UI::WindowsAndMessaging::ShowWindowAsync;
    let h = hwnd(raw);
    unsafe {
        let _ = ShowWindowAsync(h, if IsZoomed(h).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
    }
}

/// Make the bottom-right `size` × `size` (logical pixels) of the window a resize
/// grip, on top of the thin edges Windows already offers.
pub fn add_resize_grip(raw: isize, size: u32) {
    use windows::Win32::{
        Foundation::{LPARAM, LRESULT, RECT, WPARAM},
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{DefWindowProcW, HTBOTTOMRIGHT, HTCLIENT, WM_NCHITTEST, WM_NCLBUTTONDOWN},
        },
    };
    fn in_grip(h: HWND, l: LPARAM, size: usize) -> bool {
        let mut rect = RECT::default();
        if unsafe { IsZoomed(h).as_bool() || GetWindowRect(h, &mut rect).is_err() } {
            return false;
        }
        let (x, y) = ((l.0 & 0xffff) as i16 as i32, ((l.0 >> 16) & 0xffff) as i16 as i32);
        let side = size as i32 * unsafe { GetDpiForWindow(h) }.max(96) as i32 / 96;
        x >= rect.right - side && y >= rect.bottom - side
    }
    unsafe extern "system" fn grip(h: HWND, msg: u32, w: WPARAM, l: LPARAM, _: usize, size: usize) -> LRESULT {
        // GPUI would treat a press inside its client area as a click (and swallow it):
        // let Windows start resizing instead.
        if msg == WM_NCLBUTTONDOWN && w.0 as u32 == HTBOTTOMRIGHT && in_grip(h, l, size) {
            return unsafe { DefWindowProcW(h, msg, w, l) };
        }
        let hit = unsafe { DefSubclassProc(h, msg, w, l) };
        if msg == WM_NCHITTEST && hit.0 == HTCLIENT as isize && in_grip(h, l, size) {
            return LRESULT(HTBOTTOMRIGHT as isize);
        }
        hit
    }
    unsafe {
        let _ = SetWindowSubclass(hwnd(raw), Some(grip), 1, size as usize);
    }
}

/// Drop everything the system draws around a window: the 1px border and rounded
/// corners of Windows 11, and the shadow. Only our own (transparent) pixels remain.
pub fn remove_window_frame(raw: isize) {
    use windows::Win32::Graphics::Dwm::{
        DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_DONOTROUND, DwmSetWindowAttribute,
    };
    let h = hwnd(raw);
    let set = |attr, value: u32| unsafe {
        let _ = DwmSetWindowAttribute(h, attr, &value as *const u32 as *const c_void, 4);
    };
    // DWMWA_COLOR_NONE; Windows 10 ignores the attributes it doesn't know.
    set(DWMWA_BORDER_COLOR, 0xFFFF_FFFE);
    set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND.0 as u32);
    set(DWMWA_NCRENDERING_POLICY, DWMNCRP_DISABLED.0 as u32);
}

/// Show a borderless popup without activating it, above everything: `size` logical
/// pixels square, in the bottom-right corner of the work area under the cursor.
/// Put the square popup in the bottom-right corner of the mouse's monitor, or
/// (`at_cursor`) centred on the mouse, kept inside that monitor's work area.
pub fn place_corner_popup(raw: isize, size: f32, margin: f32, at_cursor: bool) {
    use windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST;
    remove_window_frame(raw);
    let h = hwnd(raw);
    unsafe {
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x.max(96) as f32 / 96.;
        let side = (size * scale) as i32;
        let gap = (margin * scale) as i32;
        let work = info.rcWork;
        let (x, y) = if at_cursor {
            // Near an edge the ring moves inwards rather than being cut off.
            let clamp = |v: i32, lo: i32, hi: i32| v.min(hi - side).max(lo);
            (clamp(cursor.x - side / 2, work.left, work.right), clamp(cursor.y - side / 2, work.top, work.bottom))
        } else {
            (work.right - side - gap, work.bottom - side - gap)
        };
        let _ = SetWindowPos(h, Some(HWND_TOPMOST), x, y, side, side, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

/// Is a full-screen program in front: a game, a full-screen video, a presentation?
pub fn fullscreen_app_in_front() -> bool {
    use windows::Win32::UI::{
        Shell::{QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState},
        WindowsAndMessaging::{GWL_STYLE, GetForegroundWindow, GetWindowLongW, WS_CAPTION},
    };
    // What Windows itself uses to hold back notifications: exclusive full-screen
    // Direct3D, presentation mode, and other full-screen apps it knows about.
    if let Ok(state) = unsafe { SHQueryUserNotificationState() } {
        if matches!(state, QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE) {
            return true;
        }
    }
    // Borderless "full-screen windowed" games and videos: a window without a title
    // bar covering its whole monitor. (A maximized window keeps its title bar.)
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() || matches!(class_name(fg).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return false;
        }
        if GetWindowLongW(fg, GWL_STYLE) as u32 & WS_CAPTION.0 == WS_CAPTION.0 {
            return false;
        }
        let mut rect = RECT::default();
        if GetWindowRect(fg, &mut rect).is_err() {
            return false;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            return false;
        }
        let m = info.rcMonitor;
        rect.left <= m.left && rect.top <= m.top && rect.right >= m.right && rect.bottom >= m.bottom
    }
}

/// File name (lower case) of the program that owns the foreground window.
pub fn foreground_program() -> Option<String> {
    program_of(unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() })
}

fn program_of(h: HWND) -> Option<String> {
    use windows::Win32::{
        System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
        UI::WindowsAndMessaging::GetWindowThreadProcessId,
    };
    if h.is_invalid() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        ok.ok()?;
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    Path::new(&path).file_name().map(|n| n.to_string_lossy().to_lowercase())
}

/// Programs with a visible, titled window right now (to pick exclusions from),
/// leaving out this app and parts of Windows itself.
pub fn windowed_programs() -> Vec<String> {
    use windows::{
        Win32::{
            Foundation::{LPARAM, TRUE},
            UI::WindowsAndMessaging::{EnumWindows, GWL_EXSTYLE, GetWindowLongW, GetWindowTextLengthW, WS_EX_TOOLWINDOW},
        },
        core::BOOL,
    };
    unsafe extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
        let list = unsafe { &mut *(out.0 as *mut Vec<HWND>) };
        let visible = unsafe { IsWindowVisible(h).as_bool() };
        let tool = unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32 & WS_EX_TOOLWINDOW.0 != 0;
        if visible && !tool && unsafe { GetWindowTextLengthW(h) } > 0 {
            list.push(h);
        }
        TRUE
    }
    let mut windows: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut windows as *mut Vec<HWND> as isize));
    }
    let own = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    const SYSTEM: &[&str] = &[
        "explorer.exe", "applicationframehost.exe", "textinputhost.exe", "shellexperiencehost.exe",
        "searchhost.exe", "startmenuexperiencehost.exe", "systemsettings.exe", "lockapp.exe",
    ];
    let mut names: Vec<String> = windows
        .into_iter()
        .filter_map(program_of)
        .filter(|n| *n != own && !SYSTEM.contains(&n.as_str()))
        .collect();
    names.sort();
    names.dedup();
    names
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// The pointer's position relative to a window's top-left corner, in logical pixels.
pub fn cursor_in_window(raw: isize) -> Option<(f32, f32)> {
    let h = hwnd(raw);
    let mut cursor = POINT::default();
    let mut rect = RECT::default();
    unsafe {
        GetCursorPos(&mut cursor).ok()?;
        GetWindowRect(h, &mut rect).ok()?;
    }
    let scale = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(h) }.max(96) as f32 / 96.;
    Some(((cursor.x - rect.left) as f32 / scale, (cursor.y - rect.top) as f32 / scale))
}

/// Unassigned virtual key that [`send_mask_key`] presses.
pub const MASK_KEY: u16 = 0xE8;

/// Press and release a key nobody listens to. Releasing Alt right after a lone Alt
/// press opens the active window's menu bar; this makes it an "Alt + something" instead.
pub fn send_mask_key() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
    };
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(MASK_KEY), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Explorer's description of a file type, e.g. "Markdown 源文件".
pub fn type_name(path: &Path) -> Option<String> {
    let wide = HSTRING::from(path.as_os_str());
    let mut info = SHFILEINFOW::default();
    let ok = unsafe {
        SHGetFileInfoW(
            &wide,
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_TYPENAME,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = info.szTypeName.iter().position(|&c| c == 0).unwrap_or(info.szTypeName.len());
    let name = String::from_utf16_lossy(&info.szTypeName[..len]);
    (!name.is_empty()).then_some(name)
}

/// Resize the content height (logical pixels), keeping the top-left corner.
pub fn set_content_height(raw: isize, height: f32) {
    let h = hwnd(raw);
    let Some(rect) = window_rect(raw) else { return };
    let scale = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(h) }.max(96) as f32 / 96.;
    let (_, dh) = frame_delta(h);
    let ht = (height * scale) as i32 + dh;
    if ht != rect.h {
        unsafe {
            let _ = SetWindowPos(h, None, 0, 0, rect.w, ht, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE);
        }
    }
}

/// Return the process's resident pages to the OS while we sit hidden in the tray.
pub fn trim_working_set() {
    unsafe {
        let _ = EmptyWorkingSet(GetCurrentProcess());
    }
}

// ---------------------------------------------------------------------------
// Launching

pub fn init_com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

pub fn expand_env(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let src = HSTRING::from(s);
    unsafe {
        let needed = ExpandEnvironmentStringsW(&src, None);
        if needed == 0 {
            return s.to_string();
        }
        let mut buf = vec![0u16; needed as usize];
        let written = ExpandEnvironmentStringsW(&src, Some(&mut buf));
        if written == 0 {
            return s.to_string();
        }
        String::from_utf16_lossy(&buf[..written as usize - 1])
    }
}

/// Launch a file, folder, URL or shell URI through the shell.
/// Blocking; call from a background thread.
pub fn shell_execute(target: &str, args: &str, workdir: &str, admin: bool) -> Result<(), String> {
    init_com();
    let target = expand_env(target.trim());
    let workdir = expand_env(workdir.trim());
    let file = HSTRING::from(target.as_str());
    let params = HSTRING::from(args);
    let dir = HSTRING::from(workdir.as_str());
    let verb = HSTRING::from(if admin { "runas" } else { "open" });

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: if args.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(params.as_ptr())
        },
        lpDirectory: if workdir.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(dir.as_ptr())
        },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
        let msg = e.message();
        if msg.is_empty() {
            format!("无法启动 {target}")
        } else {
            format!("无法启动 {target}：{msg}")
        }
    })
}

/// Resolve a bare program name ("taskmgr.exe") to a full path via the search path.
pub fn search_path(name: &str) -> Option<PathBuf> {
    let name = HSTRING::from(name);
    let mut buf = vec![0u16; 1024];
    let len = unsafe { SearchPathW(PCWSTR::null(), &name, PCWSTR::null(), Some(&mut buf), None) };
    if len == 0 || len as usize >= buf.len() {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

/// Resolve a launch target to a filesystem path suitable for icon lookup.
/// A target typed as a whole command line, like a shortcut's 目标 field
/// (`G:\tools\app\pythonw.exe -c "run()"`): split into the program and its
/// arguments. `None` when the target already is a path, URL or shell URI as it is.
pub fn split_command(target: &str) -> Option<(String, String)> {
    let t = target.trim();
    if t.is_empty() || t.contains("://") || resolve_target_path(t).is_some() {
        return None;
    }
    if let Some(rest) = t.strip_prefix('"') {
        let end = rest.find('"')?;
        let args = rest[end + 1..].trim();
        return (!args.is_empty()).then(|| (rest[..end].to_string(), args.to_string()));
    }
    // Unquoted: the shortest start (up to a space) that names an existing program,
    // so paths with spaces in them still work.
    t.match_indices(' ').map(|(i, _)| i).find_map(|i| {
        let program = &t[..i];
        let exists = resolve_target_path(program).is_some_and(|p| p.is_file())
            || (Path::new(program).extension().is_none() && resolve_target_path(&format!("{program}.exe")).is_some());
        exists.then(|| (program.to_string(), t[i + 1..].trim().to_string()))
    })
}

/// Where a program starts when its tool has no 工作目录: its own folder, as when it
/// is opened from Explorer. Documents, shortcuts and URIs are left to the shell.
/// Unix seconds → local (year, month, day, hour, minute), daylight saving included.
pub fn local_time(ts: u64) -> Option<(u16, u16, u16, u16, u16)> {
    use windows::Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
    };
    // FILETIME counts 100 ns steps since 1601.
    let ticks = (ts + 11_644_473_600) * 10_000_000;
    let file_time = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        FileTimeToSystemTime(&file_time, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some((local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute))
}

pub fn default_workdir(program: &str) -> String {
    let Some(path) = resolve_target_path(program) else { return String::new() };
    let runnable = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| matches!(e.as_str(), "exe" | "bat" | "cmd" | "com"));
    match path.parent() {
        Some(dir) if runnable => dir.display().to_string(),
        _ => String::new(),
    }
}

pub fn resolve_target_path(target: &str) -> Option<PathBuf> {
    let target = expand_env(target.trim().trim_matches('"'));
    if target.is_empty() || target.contains("://") || target.ends_with(':') {
        return None;
    }
    let path = PathBuf::from(&target);
    if path.is_absolute() {
        return path.exists().then_some(path);
    }
    search_path(&target)
}

/// The file whose icon best represents `path`. MMC snap-ins (.msc) only carry a
/// generic document icon, so prefer a same-named launcher exe, else mmc.exe.
pub fn icon_source(path: &Path) -> PathBuf {
    let is_msc = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("msc"));
    if is_msc {
        let exe = path
            .file_stem()
            .map(|stem| format!("{}.exe", stem.to_string_lossy()));
        if let Some(found) = exe.and_then(|exe| search_path(&exe)).or_else(|| search_path("mmc.exe")) {
            return found;
        }
    }
    path.to_path_buf()
}

pub fn detect_terminal() -> String {
    if search_path("wt.exe").is_some() {
        "wt.exe".into()
    } else if search_path("pwsh.exe").is_some() {
        "pwsh.exe".into()
    } else {
        "powershell.exe".into()
    }
}

/// Arguments to open `terminal` in `dir`.
pub fn terminal_args_for_dir(terminal: &str, dir: &str) -> String {
    let lower = terminal.to_ascii_lowercase();
    if lower.ends_with("wt.exe") || lower == "wt" {
        format!("-d \"{dir}\"")
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// Icons

/// A 32-bit BGRA bitmap with straight alpha.
pub struct IconBitmap {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Extract the shell's 48px icon for a file, folder or shortcut.
/// Blocking; call from a background thread.
pub fn file_icon(path: &Path) -> Option<IconBitmap> {
    shell_icon(path, SHGFI_SYSICONINDEX)
}

/// The generic icon for files with extension `ext` (e.g. "pdf"); the file need not exist.
pub fn file_type_icon(ext: &str) -> Option<IconBitmap> {
    shell_icon(Path::new(&format!("file.{ext}")), SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES)
}

fn shell_icon(path: &Path, flags: SHGFI_FLAGS) -> Option<IconBitmap> {
    init_com();
    let wide = HSTRING::from(path.as_os_str());
    let mut info = SHFILEINFOW::default();
    let ok = unsafe {
        SHGetFileInfoW(
            &wide,
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if ok == 0 {
        return None;
    }
    unsafe {
        let list: IImageList = SHGetImageList(SHIL_EXTRALARGE as i32).ok()?;
        // ILD_TRANSPARENT = 1
        let icon = list.GetIcon(info.iIcon, 1).ok()?;
        let bitmap = hicon_to_bitmap(icon);
        let _ = DestroyIcon(icon);
        bitmap
    }
}

unsafe fn hicon_to_bitmap(icon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<IconBitmap> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(icon, &mut ii).ok()?;
        let color = ii.hbmColor;
        let mask = ii.hbmMask;
        let result = (|| {
            if color.is_invalid() {
                return None;
            }
            let (w, h) = bitmap_size(color)?;
            let mut bgra = read_bits(color, w, h)?;
            // Legacy icons have no alpha channel: derive it from the AND mask.
            if bgra.chunks_exact(4).all(|p| p[3] == 0) {
                let mask_bits = read_bits(mask, w, h)?;
                for (px, m) in bgra.chunks_exact_mut(4).zip(mask_bits.chunks_exact(4)) {
                    px[3] = if m[0] == 0 { 255 } else { 0 };
                }
            }
            Some(IconBitmap {
                width: w as u32,
                height: h as u32,
                bgra,
            })
        })();
        if !color.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(color.0));
        }
        if !mask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(mask.0));
        }
        result
    }
}

unsafe fn bitmap_size(bmp: HBITMAP) -> Option<(i32, i32)> {
    let mut bm = BITMAP::default();
    let n = unsafe {
        GetObjectW(
            HGDIOBJ(bmp.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut c_void),
        )
    };
    (n != 0 && bm.bmWidth > 0 && bm.bmHeight > 0).then_some((bm.bmWidth, bm.bmHeight))
}

unsafe fn read_bits(bmp: HBITMAP, w: i32, h: i32) -> Option<Vec<u8>> {
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    unsafe {
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            bmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        (lines == h).then_some(buf)
    }
}

// ---------------------------------------------------------------------------
// Start Menu

#[derive(Clone, Debug)]
pub struct InstalledApp {
    pub name: String,
    pub path: PathBuf,
}

/// Enumerate shortcuts in the per-user and all-users Start Menu.
/// Blocking; call from a background thread.
pub fn scan_start_menu() -> Vec<InstalledApp> {
    init_com();
    let mut roots = Vec::new();
    for var in ["APPDATA", "PROGRAMDATA"] {
        if let Some(base) = std::env::var_os(var) {
            roots.push(PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs"));
        }
    }

    let mut files = Vec::new();
    for root in roots {
        walk(&root, 0, &mut files);
    }

    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for path in files {
        let name = display_name(&path).unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        let lower = name.to_lowercase();
        if name.is_empty()
            || ["uninstall", "卸载", "readme", "license", "帮助", "help"]
                .iter()
                .any(|bad| lower.contains(bad))
        {
            continue;
        }
        if seen.insert(lower) {
            apps.push(InstalledApp { name, path });
        }
    }
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk(&path, depth + 1, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "lnk" | "url" | "appref-ms"))
        {
            out.push(path);
        }
    }
}

/// Localized shell display name (honours desktop.ini LocalizedFileNames).
fn display_name(path: &Path) -> Option<String> {
    let wide = HSTRING::from(path.as_os_str());
    let mut info = SHFILEINFOW::default();
    let ok = unsafe {
        SHGetFileInfoW(
            &wide,
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_DISPLAYNAME,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = info
        .szDisplayName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szDisplayName.len());
    let name = String::from_utf16_lossy(&info.szDisplayName[..len]);
    // Display names keep the extension for .url / .appref-ms when extensions are shown.
    let name = name
        .strip_suffix(".lnk")
        .or_else(|| name.strip_suffix(".url"))
        .or_else(|| name.strip_suffix(".appref-ms"))
        .unwrap_or(&name)
        .to_string();
    (!name.is_empty()).then_some(name)
}

// ---------------------------------------------------------------------------
// Single instance

pub enum Instance {
    Primary,
    /// Another instance is running and has been asked to show itself.
    Secondary,
}

pub fn acquire_single_instance() -> Instance {
    unsafe {
        let mutex = CreateMutexW(None, true, &HSTRING::from(MUTEX_NAME));
        if mutex.is_ok() && GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, &HSTRING::from(SHOW_EVENT_NAME)) {
                // Let the primary instance take the foreground.
                let _ = AllowSetForegroundWindow(ASFW_ANY);
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
            }
            return Instance::Secondary;
        }
        // Intentionally leak the mutex handle: it must live for the whole process.
        Instance::Primary
    }
}

/// Spawn a thread that calls `on_show` whenever a second instance is started.
pub fn listen_for_show_requests(on_show: impl Fn() + Send + 'static) {
    let event = unsafe { CreateEventW(None, false, false, &HSTRING::from(SHOW_EVENT_NAME)) };
    let Ok(event) = event else { return };
    let raw = event.0 as isize;
    std::thread::Builder::new()
        .name("show-listener".into())
        .spawn(move || {
            let event = HANDLE(raw as *mut c_void);
            loop {
                unsafe { WaitForSingleObject(event, INFINITE) };
                on_show();
            }
        })
        .ok();
}

// ---------------------------------------------------------------------------
// Autostart

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let key = windows_registry::CURRENT_USER
        .create(RUN_KEY)
        .map_err(|e| e.to_string())?;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        key.set_string(RUN_VALUE, format!("\"{}\" --hidden", exe.display()))
            .map_err(|e| e.to_string())
    } else {
        match key.remove_value(RUN_VALUE) {
            Ok(()) => Ok(()),
            Err(_) if !is_autostart() => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

pub fn is_autostart() -> bool {
    windows_registry::CURRENT_USER
        .open(RUN_KEY)
        .and_then(|k| k.get_string(RUN_VALUE))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_command_lines() {
        let notepad = r"C:\Windows\System32\notepad.exe";
        assert_eq!(split_command(notepad), None, "a plain path stays as it is");
        assert_eq!(
            split_command(&format!(r#"{notepad} -c "from x import run;run()""#)),
            Some((notepad.to_string(), r#"-c "from x import run;run()""#.to_string()))
        );
        assert_eq!(
            split_command(&format!(r#""{notepad}" C:\a b.txt"#)),
            Some((notepad.to_string(), r"C:\a b.txt".to_string()))
        );
        assert_eq!(split_command("notepad readme.txt"), Some(("notepad".into(), "readme.txt".into())));
        assert_eq!(split_command("ms-settings:display"), None);
        assert_eq!(split_command("https://example.com/a b"), None);
        assert_eq!(split_command("no-such-program --flag"), None);
    }

    #[test]
    fn programs_start_in_their_folder() {
        assert_eq!(default_workdir(r"C:\Windows\System32\notepad.exe"), r"C:\Windows\System32");
        assert_eq!(default_workdir("ms-settings:"), "");
    }
}
