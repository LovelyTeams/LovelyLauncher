//! 剪贴板历史: what was copied (text, pictures, files), kept in `clipboard.json`
//! with the pictures in `clipboard\`, next to the config. A hidden window on its own
//! thread is told about every clipboard change.

use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use futures::channel::mpsc::UnboundedSender;
use serde::{Deserialize, Serialize};
use windows::{
    Win32::{
        Foundation::{HGLOBAL, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            DataExchange::{
                AddClipboardFormatListener, CloseClipboard, GetClipboardData, IsClipboardFormatAvailable,
                OpenClipboard, RegisterClipboardFormatW,
            },
            LibraryLoader::GetModuleHandleW,
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
        UI::{
            Shell::{DragQueryFileW, HDROP},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG, RegisterClassW,
                WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WNDCLASSW,
            },
        },
    },
    core::{PCWSTR, w},
};

use crate::config::Config;

/// Entries kept besides the pinned ones.
pub const MAX: usize = 200;
/// Pictures kept besides the pinned ones (they take disk space).
pub const MAX_IMAGES: usize = 40;
/// Longer text is cut (the clipboard can hold whole files).
const MAX_TEXT: usize = 256 * 1024;
/// Bigger pictures are not kept.
const MAX_PIXELS: u64 = 40_000_000;

/// Whether copies are recorded (设置 → 剪贴板).
pub static ENABLED: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Text,
    Image,
    Files,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clip {
    pub id: String,
    pub kind: Kind,
    /// The text, or the copied files' paths one per line.
    pub text: String,
    /// The picture's file name in `clipboard\` (pictures only).
    pub image: String,
    pub width: u32,
    pub height: u32,
    /// The picture's pixels, hashed, to notice the same picture copied again.
    pub hash: u64,
    /// Unix seconds when it was (last) copied.
    pub time: u64,
    /// Pinned entries stay at the top and are never dropped.
    pub pinned: bool,
}

/// What a clipboard change brought.
#[derive(Debug)]
pub enum Copied {
    Text(String),
    Files(Vec<String>),
    /// Already saved as `clipboard\<file>`.
    Image { file: String, width: u32, height: u32, hash: u64 },
}

impl Clip {
    /// One line describing it, for a list.
    pub fn title(&self) -> String {
        match self.kind {
            Kind::Text => {
                let line = self.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
                let mut out: String = line.chars().take(120).collect();
                if out.is_empty() {
                    out = "（空白）".into();
                }
                out
            }
            Kind::Image => format!("图片 {} × {}", self.width, self.height),
            Kind::Files => {
                let names: Vec<String> = self
                    .files()
                    .iter()
                    .take(3)
                    .map(|p| {
                        std::path::Path::new(p)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.clone())
                    })
                    .collect();
                let count = self.files().len();
                if count > 3 {
                    format!("{}… 共 {count} 项", names.join("、"))
                } else {
                    names.join("、")
                }
            }
        }
    }

    /// Kind and size, for the line under the title.
    pub fn detail(&self) -> String {
        match self.kind {
            Kind::Text => {
                let lines = self.text.lines().count().max(1);
                let chars = self.text.chars().filter(|c| !c.is_whitespace()).count();
                if lines > 1 { format!("文本 · {lines} 行 · {chars} 字") } else { format!("文本 · {chars} 字") }
            }
            Kind::Image => "图片".into(),
            Kind::Files => format!("{} 个文件", self.files().len()),
        }
    }

    pub fn files(&self) -> Vec<String> {
        match self.kind {
            Kind::Files => self.text.lines().filter(|l| !l.is_empty()).map(str::to_string).collect(),
            _ => Vec::new(),
        }
    }

    pub fn image_path(&self) -> PathBuf {
        dir().join(&self.image)
    }

    /// Whether `filter` (lower-cased words) is found in it.
    pub fn matches(&self, filter: &str) -> bool {
        let haystack = match self.kind {
            Kind::Image => "图片 image picture".to_string(),
            _ => self.text.to_lowercase(),
        };
        filter.split_whitespace().all(|word| haystack.contains(word))
    }

    fn same_as(&self, copied: &Copied) -> bool {
        match (self.kind, copied) {
            (Kind::Text, Copied::Text(t)) => self.text == *t,
            (Kind::Files, Copied::Files(f)) => self.files() == *f,
            (Kind::Image, Copied::Image { hash, .. }) => self.hash == *hash,
            _ => false,
        }
    }
}

pub fn dir() -> PathBuf {
    Config::dir().join("clipboard")
}

fn path() -> PathBuf {
    Config::dir().join("clipboard.json")
}

/// Newest first.
pub fn load() -> Vec<Clip> {
    fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save(list: &[Clip]) {
    let dir = Config::dir();
    let _ = fs::create_dir_all(&dir);
    let Ok(json) = serde_json::to_string(list) else { return };
    // Write-then-rename so a crash never leaves a truncated file behind.
    let tmp = dir.join("clipboard.json.tmp");
    if fs::write(&tmp, json).is_ok() {
        let _ = fs::rename(&tmp, path());
    }
}

/// Put what was copied on top: an equal older entry moves up (keeping its pin).
/// Returns picture files no longer used, to delete.
pub fn remember(list: &mut Vec<Clip>, copied: Copied, now: u64) -> Vec<String> {
    let mut clip = match list.iter().position(|c| c.same_as(&copied)) {
        Some(ix) => list.remove(ix),
        None => {
            let mut clip = Clip { id: crate::config::new_id(), ..Default::default() };
            match copied {
                Copied::Text(text) => {
                    clip.kind = Kind::Text;
                    clip.text = text;
                }
                Copied::Files(files) => {
                    clip.kind = Kind::Files;
                    clip.text = files.join("\n");
                }
                Copied::Image { file, width, height, hash } => {
                    clip.kind = Kind::Image;
                    clip.image = file;
                    clip.width = width;
                    clip.height = height;
                    clip.hash = hash;
                }
            }
            clip
        }
    };
    clip.time = now;
    list.insert(0, clip);
    trim(list)
}

/// Drop the oldest unpinned entries beyond the limits; returns their picture files.
fn trim(list: &mut Vec<Clip>) -> Vec<String> {
    let mut kept = 0;
    let mut images = 0;
    let mut dropped = Vec::new();
    list.retain(|c| {
        if c.pinned {
            return true;
        }
        kept += 1;
        if c.kind == Kind::Image {
            images += 1;
        }
        let keep = kept <= MAX && (c.kind != Kind::Image || images <= MAX_IMAGES);
        if !keep && c.kind == Kind::Image {
            dropped.push(c.image.clone());
        }
        keep
    });
    // A picture file may be shared by an entry that stays (the same picture).
    dropped.retain(|f| !list.iter().any(|c| c.image == *f));
    dropped
}

/// Delete pictures that no entry uses any more.
pub fn delete_images(files: &[String]) {
    for file in files {
        if !file.is_empty() {
            let _ = fs::remove_file(dir().join(file));
        }
    }
}

// ---------------------------------------------------------------------------
// Listening

static SENDER: OnceLock<UnboundedSender<Copied>> = OnceLock::new();

/// Report every clipboard change on `tx`, from a hidden window on its own thread.
pub fn listen(tx: UnboundedSender<Copied>) {
    if SENDER.set(tx).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(|| unsafe {
            let Ok(module) = GetModuleHandleW(PCWSTR::null()) else { return };
            let class = w!("LovelyLauncherClipboard");
            let wc = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: module.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            let Ok(hwnd) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(module.into()),
                None,
            ) else {
                return;
            };
            if AddClipboardFormatListener(hwnd).is_err() {
                return;
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
        })
        .ok();
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        if ENABLED.load(Ordering::Relaxed)
            && let (Some(copied), Some(tx)) = (read(hwnd), SENDER.get())
        {
            let _ = tx.unbounded_send(copied);
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Open the clipboard, retrying a little while another program holds it.
fn open(hwnd: HWND) -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    false
}

/// What is on the clipboard now, unless it asks not to be recorded (passwords).
fn read(hwnd: HWND) -> Option<Copied> {
    const CF_UNICODETEXT: u32 = 13;
    const CF_HDROP: u32 = 15;
    if !open(hwnd) {
        return None;
    }
    let found = unsafe {
        let available = |format: u32| IsClipboardFormatAvailable(format).is_ok();
        // Password managers mark what they copy with these.
        let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        let ignore = RegisterClipboardFormatW(w!("Clipboard Viewer Ignore"));
        let history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        let excluded = available(exclude) || available(ignore) || (available(history) && dword(history) == Some(0));
        let result = if excluded {
            Err(())
        } else if available(CF_HDROP) {
            Ok(files(CF_HDROP).map(Copied::Files))
        } else if available(CF_UNICODETEXT) {
            Ok(text(CF_UNICODETEXT).map(Copied::Text))
        } else {
            Ok(None)
        };
        let _ = CloseClipboard();
        result
    };
    match found {
        Err(()) => None,
        Ok(Some(copied)) => Some(copied),
        // Neither text nor files: maybe a picture (read with the clipboard closed again).
        Ok(None) => picture(),
    }
}

unsafe fn global_bytes(format: u32) -> Option<Vec<u8>> {
    unsafe {
        let handle = GetClipboardData(format).ok()?;
        let global = HGLOBAL(handle.0);
        let size = GlobalSize(global);
        let ptr = GlobalLock(global) as *const u8;
        if ptr.is_null() || size == 0 {
            return None;
        }
        let data = std::slice::from_raw_parts(ptr, size).to_vec();
        let _ = GlobalUnlock(global);
        Some(data)
    }
}

unsafe fn dword(format: u32) -> Option<u32> {
    let bytes = unsafe { global_bytes(format)? };
    Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
}

unsafe fn text(format: u32) -> Option<String> {
    let bytes = unsafe { global_bytes(format)? };
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
    let text = String::from_utf16_lossy(&units);
    if text.trim().is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(MAX_TEXT) {
        Some((cut, _)) => text[..cut].to_string(),
        None => text,
    })
}

unsafe fn files(format: u32) -> Option<Vec<String>> {
    unsafe {
        let handle = GetClipboardData(format).ok()?;
        let drop = HDROP(handle.0);
        let count = DragQueryFileW(drop, u32::MAX, None);
        let files: Vec<String> = (0..count)
            .filter_map(|i| {
                let len = DragQueryFileW(drop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let got = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
                (got > 0).then(|| String::from_utf16_lossy(&buf[..got]))
            })
            .collect();
        (!files.is_empty()).then_some(files)
    }
}

/// A copied picture, saved as `clipboard\<hash>.png`.
fn picture() -> Option<Copied> {
    let (width, height, bgra) = crate::scan::clipboard_picture()?;
    if width as u64 * height as u64 > MAX_PIXELS {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    (width, height).hash(&mut hasher);
    bgra.hash(&mut hasher);
    let hash = hasher.finish();
    let file = format!("{hash:016x}.png");
    let path = dir().join(&file);
    if !path.exists() {
        let rgba: Vec<u8> = bgra.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        let image = image::RgbaImage::from_raw(width, height, rgba)?;
        let _ = fs::create_dir_all(dir());
        image.save_with_format(&path, image::ImageFormat::Png).ok()?;
    }
    Some(Copied::Image { file, width, height, hash })
}

// ---------------------------------------------------------------------------
// Snippets

/// Fill in a snippet's variables: {date} {time} {datetime} {clipboard}.
pub fn expand(text: &str, clipboard: Option<&str>) -> String {
    let now = crate::config::now_secs();
    let (y, mo, d, h, mi) = crate::win::local_time(now).unwrap_or((1970, 1, 1, 0, 0));
    expand_with(text, (y, mo, d, h, mi), clipboard)
}

fn expand_with(text: &str, (y, mo, d, h, mi): (u16, u16, u16, u16, u16), clipboard: Option<&str>) -> String {
    let date = format!("{y}-{mo:02}-{d:02}");
    let time = format!("{h:02}:{mi:02}");
    text.replace("{datetime}", &format!("{date} {time}"))
        .replace("{date}", &date)
        .replace("{time}", &time)
        .replace("{clipboard}", clipboard.unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Copied {
        Copied::Text(t.into())
    }

    #[test]
    fn copies_go_on_top_without_repeats() {
        let mut list = Vec::new();
        remember(&mut list, text("a"), 1);
        remember(&mut list, text("b"), 2);
        list[1].pinned = true; // "a"
        remember(&mut list, text("a"), 3);
        assert_eq!(list.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(list[0].pinned, "copying again keeps the pin");
        assert_eq!(list[0].time, 3);
    }

    #[test]
    fn old_entries_and_pictures_drop_off() {
        let mut list = Vec::new();
        let mut pinned = Clip { id: "p".into(), text: "keep".into(), pinned: true, ..Default::default() };
        pinned.time = 0;
        list.push(pinned);
        let mut dropped = Vec::new();
        for n in 0..MAX_IMAGES + 3 {
            dropped.extend(remember(&mut list, Copied::Image { file: format!("{n}.png"), width: 1, height: 1, hash: n as u64 }, n as u64));
        }
        assert_eq!(dropped, ["0.png", "1.png", "2.png"]);
        for n in 0..MAX + 10 {
            remember(&mut list, text(&n.to_string()), 1000 + n as u64);
        }
        assert_eq!(list.iter().filter(|c| !c.pinned).count(), MAX);
        assert!(list.iter().any(|c| c.pinned && c.text == "keep"));
    }

    #[test]
    fn titles() {
        let clip = Clip { text: "\n  第一行  \n第二行".into(), ..Default::default() };
        assert_eq!(clip.title(), "第一行");
        assert_eq!(clip.detail(), "文本 · 3 行 · 6 字");
        let files = Clip { kind: Kind::Files, text: "C:\\a\\x.txt\nC:\\b\\y.png".into(), ..Default::default() };
        assert_eq!(files.title(), "x.txt、y.png");
        assert!(files.matches("y.png"));
        assert!(!files.matches("z"));
    }

    #[test]
    fn snippet_variables() {
        let out = expand_with("{date} {time} | {datetime} | {clipboard}", (2026, 3, 7, 9, 5), Some("剪贴"));
        assert_eq!(out, "2026-03-07 09:05 | 2026-03-07 09:05 | 剪贴");
    }
}
