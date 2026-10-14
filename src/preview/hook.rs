//! QuickLook-style trigger: a low-level keyboard hook that turns Space in an
//! Explorer / desktop file list into a preview request, plus the Shell COM
//! plumbing that reads the selected item.

use std::{
    ffi::c_void,
    mem::ManuallyDrop,
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use futures::channel::mpsc::UnboundedSender;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, IServiceProvider},
            LibraryLoader::GetModuleHandleW,
            Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4},
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_LWIN, VK_MENU,
                VK_RIGHT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_UP,
            },
            Shell::{
                CSIDL_DESKTOP, IFolderView, IShellBrowser, IShellItemArray, IShellWindows,
                IWebBrowserApp, SID_STopLevelBrowser, SIGDN_FILESYSPATH, SVGIO_SELECTION,
                SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
            },
            WindowsAndMessaging::{
                CallNextHookEx, DispatchMessageW, GUITHREADINFO, GetClassNameW,
                GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetWindowThreadProcessId,
                HC_ACTION, IsWindowVisible, KBDLLHOOKSTRUCT, MSG, SetWindowsHookExW,
                TranslateMessage, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
            },
        },
    },
    core::{Interface, PCWSTR},
};

#[derive(Clone, Copy, Debug)]
pub enum HookEvent {
    /// Space in a file list: open the preview, or close it if one is open.
    Toggle { hwnd: isize },
    /// An arrow key moved the selection while a preview is open.
    Navigate { hwnd: isize },
    /// Esc in the file list while a preview is open.
    Close,
}

static SENDER: OnceLock<Box<dyn Fn(HookEvent) + Send + Sync>> = OnceLock::new();
/// Whether Space should be intercepted at all (Settings → 预览).
pub static ENABLED: AtomicBool = AtomicBool::new(true);
/// Whether a preview window is currently showing.
pub static PREVIEW_OPEN: AtomicBool = AtomicBool::new(false);

/// Install the hook on its own thread (low-level hooks need a message loop).
pub fn start<T: Send + 'static>(tx: UnboundedSender<T>, wrap: fn(HookEvent) -> T) {
    let _ = SENDER.set(Box::new(move |event| {
        let _ = tx.unbounded_send(wrap(event));
    }));
    std::thread::Builder::new()
        .name("space-hook".into())
        .spawn(|| unsafe {
            let module = GetModuleHandleW(PCWSTR::null()).ok().map(Into::into);
            let Ok(_hook) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0) else {
                return;
            };
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .ok();
}

fn send(event: HookEvent) {
    if let Some(send) = SENDER.get() {
        send(event);
    }
}

fn key_down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && ENABLED.load(Ordering::Relaxed) {
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let msg = wparam.0 as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let vk = kb.vkCode as u16;
        let modifiers = [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
            .iter()
            .any(|m| key_down(m.0 as i32));

        if vk == VK_SPACE.0 && is_down && !modifiers {
            let fg = unsafe { GetForegroundWindow() };
            if file_list_focused(fg) {
                send(HookEvent::Toggle { hwnd: fg.0 as isize });
                return LRESULT(1); // swallow: Space would otherwise toggle selection
            }
        } else if PREVIEW_OPEN.load(Ordering::Relaxed) {
            let arrows = [VK_UP.0, VK_DOWN.0, VK_LEFT.0, VK_RIGHT.0];
            if msg == WM_KEYUP && arrows.contains(&vk) {
                let fg = unsafe { GetForegroundWindow() };
                if file_list_focused(fg) {
                    send(HookEvent::Navigate { hwnd: fg.0 as isize });
                }
            } else if is_down && vk == VK_ESCAPE.0 && file_list_focused(unsafe { GetForegroundWindow() }) {
                send(HookEvent::Close);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

const EXPLORER_CLASSES: &[&str] = &["CabinetWClass", "ExploreWClass"];
const DESKTOP_CLASSES: &[&str] = &["Progman", "WorkerW"];

fn is_desktop(hwnd: HWND) -> bool {
    DESKTOP_CLASSES.contains(&class_name(hwnd).as_str())
}

/// Is keyboard focus in an Explorer or desktop file list (and not in a text box)?
fn file_list_focused(fg: HWND) -> bool {
    if fg.is_invalid() {
        return false;
    }
    let class = class_name(fg);
    if !EXPLORER_CLASSES.contains(&class.as_str()) && !DESKTOP_CLASSES.contains(&class.as_str()) {
        return false;
    }
    let thread = unsafe { GetWindowThreadProcessId(fg, None) };
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetGUIThreadInfo(thread, &mut info) }.is_err() {
        return false;
    }
    // A caret means the user is typing (rename box, address bar, search box).
    if !info.hwndCaret.is_invalid() {
        return false;
    }
    matches!(class_name(info.hwndFocus).as_str(), "DirectUIHWND" | "SysListView32")
}

fn variant_i4(value: i32) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_I4,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { lVal: value },
            }),
        },
    }
}

/// Path of the first selected item in the Explorer window / desktop `hwnd`.
/// Must run on an STA thread (the UI thread).
pub fn selected_path(hwnd: isize) -> Option<PathBuf> {
    crate::win::init_com();
    let target = HWND(hwnd as *mut c_void);
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        if is_desktop(target) {
            let mut found = 0i32;
            let disp = windows
                .FindWindowSW(
                    &variant_i4(CSIDL_DESKTOP as i32),
                    &VARIANT::default(),
                    SWC_DESKTOP,
                    &mut found,
                    SWFO_NEEDDISPATCH,
                )
                .ok()?;
            let provider: IServiceProvider = disp.cast().ok()?;
            return first_selected(&provider, false);
        }
        let count = windows.Count().ok()?;
        for i in 0..count {
            let Ok(disp) = windows.Item(&variant_i4(i)) else { continue };
            let Ok(app) = disp.cast::<IWebBrowserApp>() else { continue };
            if app.HWND().ok().map(|h| h.0 as isize) != Some(hwnd) {
                continue;
            }
            let Ok(provider) = disp.cast::<IServiceProvider>() else { continue };
            // With Explorer tabs every tab shares the window handle; only the
            // active tab's view is visible.
            if let Some(path) = first_selected(&provider, true) {
                return Some(path);
            }
        }
        None
    }
}

unsafe fn first_selected(provider: &IServiceProvider, require_visible: bool) -> Option<PathBuf> {
    unsafe {
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser).ok()?;
        let view = browser.QueryActiveShellView().ok()?;
        if require_visible {
            let view_hwnd = view.GetWindow().ok()?;
            if !IsWindowVisible(view_hwnd).as_bool() {
                return None;
            }
        }
        let folder_view: IFolderView = view.cast().ok()?;
        let items: IShellItemArray = folder_view.Items(SVGIO_SELECTION).ok()?;
        if items.GetCount().ok()? == 0 {
            return None;
        }
        let item = items.GetItemAt(0).ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const c_void));
        path.map(PathBuf::from)
    }
}
