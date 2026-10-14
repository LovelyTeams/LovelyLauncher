//! QuickLook-style file preview window.

pub mod ffmpeg;
pub mod hook;
pub mod media;
pub mod office;

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use gpui::{
    AnyElement, App, AppContext, Bounds, ClickEvent, Context, FocusHandle, FontWeight,
    HighlightStyle, Image, ImageFormat, KeyBinding, ListAlignment, ListState, MouseButton,
    MouseDownEvent, ObjectFit, Pixels, Render, RenderImage, ScrollHandle, SharedString,
    StrikethroughStyle, StyledImage, StyledText, UnderlineStyle, WeakEntity, Window,
    WindowControlArea, actions, canvas, div, img, list, prelude::*, px, uniform_list,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{
    launcher::Launcher,
    preview::{
        ffmpeg::FfmpegState,
        media::Player,
        office::{DocBlock, RunStyle, Sheet},
    },
    theme::Theme,
    view::icon,
    win,
};

actions!(preview, [ClosePreview, TogglePlayback]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Preview");
    cx.bind_keys([
        KeyBinding::new("space", ClosePreview, ctx),
        KeyBinding::new("escape", ClosePreview, ctx),
        KeyBinding::new("k", TogglePlayback, ctx),
    ]);
}

const TEXT_LIMIT: usize = 4 * 1024 * 1024;
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "log", "ini", "cfg", "conf", "config", "json", "jsonc", "json5", "yaml", "yml", "toml",
    "xml", "html", "htm", "css", "scss", "less", "js", "mjs", "cjs", "jsx", "tsx", "vue", "svelte",
    "rs", "py", "go", "java", "kt", "kts", "c", "h", "cpp", "hpp", "cc", "cs", "swift", "dart",
    "rb", "php", "lua", "sh", "bash", "zsh", "ps1", "psm1", "bat", "cmd", "sql", "r", "m", "scala",
    "gradle", "properties", "env", "gitignore", "gitattributes", "editorconfig", "dockerfile",
    "makefile", "cmake", "csv2", "srt", "ass", "vtt", "lrc", "nfo", "diff", "patch", "reg", "inf",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Markdown,
    Image,
    Text,
    Delimited(char),
    Excel,
    Word,
    Audio,
    Video,
    Folder,
    Other,
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn kind_of(path: &Path) -> Kind {
    if path.is_dir() {
        return Kind::Folder;
    }
    match ext_of(path).as_str() {
        "md" | "markdown" | "mdown" | "mkd" => Kind::Markdown,
        "png" | "jpg" | "jpeg" | "jfif" | "gif" | "bmp" | "webp" | "ico" | "tif" | "tiff" | "svg" => Kind::Image,
        "csv" => Kind::Delimited(','),
        "tsv" | "tab" => Kind::Delimited('\t'),
        "xlsx" | "xlsm" => Kind::Excel,
        "docx" => Kind::Word,
        "mp3" | "wav" | "wma" | "mid" | "midi" | "flac" | "ogg" | "oga" | "opus" | "m4a" | "aac" | "ape"
        | "aiff" | "aif" | "amr" => Kind::Audio,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "flv" | "m4v" | "mts" | "m2ts" | "3gp" | "mpg"
        | "mpeg" | "vob" => Kind::Video,
        // ".ts" is either TypeScript or an MPEG transport stream: decided by sniffing.
        "ts" => Kind::Other,
        e if TEXT_EXTENSIONS.contains(&e) => Kind::Text,
        _ => {
            let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if matches!(name.as_str(), "dockerfile" | "makefile" | "license" | "readme" | "changelog") {
                Kind::Text
            } else {
                Kind::Other
            }
        }
    }
}

/// Initial window content size (logical px) for a kind.
pub fn initial_size(kind: Kind) -> (f32, f32) {
    match kind {
        Kind::Audio => (560., 300.),
        Kind::Image | Kind::Video => (1000., 720.),
        Kind::Folder | Kind::Other => (620., 520.),
        _ => (920., 720.),
    }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024. && unit < UNITS.len() - 1 {
        v /= 1024.;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[unit]) }
}

enum Loaded {
    Text(Vec<SharedString>),
    Markdown(String),
    Image,
    Table(Vec<Sheet>, Vec<Vec<f32>>),
    Doc(Vec<DocBlock>, Vec<Option<Arc<Image>>>),
    Folder(Vec<(String, bool, u64)>, usize),
    Info(String),
    Media,
}

enum Content {
    Loading,
    Error(String),
    Text { lines: Arc<Vec<SharedString>>, list: ListState },
    Markdown(Arc<String>),
    Image,
    Table { sheets: Arc<Vec<Sheet>>, widths: Arc<Vec<Vec<f32>>> },
    Doc { blocks: Arc<Vec<DocBlock>>, images: Arc<Vec<Option<Arc<Image>>>> },
    Folder { entries: Arc<Vec<(String, bool, u64)>>, total: usize },
    Info(String),
    Media,
    NeedFfmpeg,
}

pub struct Preview {
    path: PathBuf,
    kind: Kind,
    content: Content,
    meta: SharedString,
    icon: Option<Arc<RenderImage>>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    sheet: usize,
    launcher: WeakEntity<Launcher>,
    player: Option<Player>,
    frame: Option<Arc<RenderImage>>,
    frame_seq: u64,
    bar_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Bumped on every load so stale background results are ignored.
    generation: u64,
}

impl Preview {
    pub fn new(path: PathBuf, anchor: Option<isize>, launcher: WeakEntity<Launcher>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hwnd = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
            _ => None,
        };
        window.on_window_should_close(cx, |_, _| {
            hook::PREVIEW_OPEN.store(false, Ordering::Relaxed);
            true
        });
        let kind = kind_of(&path);
        let mut this = Self {
            path: path.clone(),
            kind,
            content: Content::Loading,
            meta: SharedString::default(),
            icon: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            sheet: 0,
            launcher,
            player: None,
            frame: None,
            frame_seq: 0,
            bar_bounds: Rc::new(Cell::new(Bounds::default())),
            generation: 0,
        };
        window.focus(&this.focus);
        // Show without stealing focus from Explorer, so arrow keys keep working there.
        if let Some(raw) = hwnd {
            let (w, h) = initial_size(kind);
            cx.spawn(async move |_, _| win::place_window(raw, w, h, anchor)).detach();
        }
        hook::PREVIEW_OPEN.store(true, Ordering::Relaxed);
        this.show_path(path, window, cx);
        this
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn close(&mut self, window: &mut Window) {
        if let Some(mut player) = self.player.take() {
            player.close();
        }
        hook::PREVIEW_OPEN.store(false, Ordering::Relaxed);
        window.remove_window();
    }

    fn close_action(&mut self, _: &ClosePreview, window: &mut Window, _: &mut Context<Self>) {
        self.close(window);
    }

    fn toggle_action(&mut self, _: &TogglePlayback, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(player) = self.player.as_mut() {
            player.toggle();
            cx.notify();
        }
    }

    /// (Re)load the preview for `path` — also used when the Explorer selection moves.
    pub fn show_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mut player) = self.player.take() {
            player.close();
        }
        if let Some(frame) = self.frame.take() {
            let _ = window.drop_image(frame);
        }
        self.generation += 1;
        let generation = self.generation;
        self.path = path.clone();
        self.kind = kind_of(&path);
        self.content = Content::Loading;
        self.sheet = 0;
        self.icon = None;
        self.scroll.set_offset(Default::default());
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        window.set_window_title(&format!("{name} - LovelyLauncher 预览"));
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        self.meta = if self.kind == Kind::Folder { "文件夹".into() } else { human_size(size).into() };

        // File icon for the header.
        let icon_path = path.clone();
        cx.spawn(async move |this, cx| {
            let bitmap = cx.background_spawn(async move { win::file_icon(&icon_path) }).await;
            let image = bitmap
                .and_then(|b| image::RgbaImage::from_raw(b.width, b.height, b.bgra))
                .map(|img| Arc::new(RenderImage::new(vec![image::Frame::new(img)])));
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.icon = image;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();

        if matches!(self.kind, Kind::Audio | Kind::Video) {
            self.load_media(window, cx);
            cx.notify();
            return;
        }
        let kind = self.kind;
        cx.spawn_in(window, async move |this, cx| {
            let load_path = path.clone();
            let result = cx.background_spawn(async move { load(&load_path, kind) }).await;
            this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((loaded, meta)) => {
                        if !meta.is_empty() {
                            this.meta = format!("{} · {meta}", this.meta).into();
                        }
                        this.content = match loaded {
                            Loaded::Text(lines) => {
                                let list = ListState::new(lines.len(), ListAlignment::Top, px(600.));
                                Content::Text { lines: Arc::new(lines), list }
                            }
                            Loaded::Markdown(source) => Content::Markdown(Arc::new(source)),
                            Loaded::Image => Content::Image,
                            Loaded::Table(sheets, widths) => Content::Table {
                                sheets: Arc::new(sheets),
                                widths: Arc::new(widths),
                            },
                            Loaded::Doc(blocks, images) => Content::Doc {
                                blocks: Arc::new(blocks),
                                images: Arc::new(images),
                            },
                            Loaded::Folder(entries, total) => Content::Folder { entries: Arc::new(entries), total },
                            Loaded::Info(type_name) => Content::Info(type_name),
                            Loaded::Media => {
                                // Sniffing turned an unknown file (".ts") into a video.
                                this.kind = Kind::Video;
                                this.load_media(window, cx);
                                return;
                            }
                        };
                    }
                    Err(err) => this.content = Content::Error(err),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn load_media(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ext = ext_of(&self.path);
        if self.kind == Kind::Audio && !media::ffmpeg_needed(&ext) {
            match Player::open_mci(&self.path) {
                Ok(mut player) => {
                    player.play();
                    self.player = Some(player);
                    self.content = Content::Media;
                    self.start_ticker(window, cx);
                    return;
                }
                // Fall through to FFmpeg, which handles odd encodings MCI rejects.
                Err(_) if cx.global::<FfmpegState>().path.is_some() => {}
                Err(err) => {
                    self.content = Content::Error(err);
                    return;
                }
            }
        }
        let Some(exe) = cx.global::<FfmpegState>().path.clone() else {
            self.content = Content::NeedFfmpeg;
            self.wait_for_ffmpeg(window, cx);
            return;
        };
        let generation = self.generation;
        let path = self.path.clone();
        self.content = Content::Loading;
        cx.spawn_in(window, async move |this, cx| {
            let probe_exe = exe.clone();
            let probe = cx.background_spawn(async move { ffmpeg::probe(&probe_exe, &path) }).await;
            this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                match probe {
                    Ok(probe) => {
                        if let Some((w, h)) = probe.video {
                            this.meta = format!("{} · {w}×{h}", this.meta).into();
                        }
                        if let Some(d) = probe.duration {
                            this.meta = format!("{} · {}", this.meta, media::format_time(d)).into();
                        }
                        let mut player = Player::open_ffmpeg(&exe, &this.path, &probe, (1280, 720));
                        player.play();
                        this.player = Some(player);
                        this.content = Content::Media;
                        this.start_ticker(window, cx);
                    }
                    Err(err) => this.content = Content::Error(err),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// While FFmpeg is missing, reload as soon as it's been downloaded or chosen.
    fn wait_for_ffmpeg(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.generation;
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(400)).await;
                let keep_waiting = this
                    .update_in(cx, |this, window, cx| {
                        if this.generation != generation || !matches!(this.content, Content::NeedFfmpeg) {
                            return false;
                        }
                        if cx.global::<FfmpegState>().path.is_some() {
                            this.load_media(window, cx);
                            cx.notify();
                            return false;
                        }
                        cx.notify(); // download progress
                        true
                    })
                    .unwrap_or(false);
                if !keep_waiting {
                    break;
                }
            }
        })
        .detach();
    }

    /// Pull decoded video frames into the view and keep the progress bar moving.
    fn start_ticker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.generation;
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let video = this
                    .update(cx, |this, _| this.player.as_ref().is_some_and(|p| p.video_size.is_some()))
                    .unwrap_or(false);
                cx.background_executor()
                    .timer(Duration::from_millis(if video { 15 } else { 250 }))
                    .await;
                let alive = this
                    .update_in(cx, |this, window, cx| {
                        if this.generation != generation {
                            return false;
                        }
                        let Some(player) = this.player.as_mut() else { return false };
                        if player.finished() {
                            player.pause();
                            player.seek(0.);
                        }
                        let playing = player.is_playing();
                        if let Some((w, h)) = player.video_size {
                            let seq = player.frames.seq.load(Ordering::Relaxed);
                            if seq != this.frame_seq {
                                this.frame_seq = seq;
                                let bytes = player.frames.frame.lock().unwrap().take();
                                if let Some(img) = bytes.and_then(|b| image::RgbaImage::from_raw(w, h, b)) {
                                    let frame = Arc::new(RenderImage::new(vec![image::Frame::new(img)]));
                                    if let Some(old) = this.frame.replace(frame) {
                                        let _ = window.drop_image(old);
                                    }
                                    cx.notify();
                                }
                            }
                        }
                        if playing {
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    fn open_default(&mut self, cx: &mut Context<Self>) {
        let path = self.path.display().to_string();
        cx.background_spawn(async move {
            let _ = win::shell_execute(&path, "", "", false);
        })
        .detach();
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        let args = format!("/select,\"{}\"", self.path.display());
        cx.background_spawn(async move {
            let _ = win::shell_execute("explorer.exe", &args, "", false);
        })
        .detach();
    }
}

/// Read and parse a file for preview (background thread). Returns content + extra meta.
fn load(path: &Path, kind: Kind) -> Result<(Loaded, String), String> {
    match kind {
        Kind::Folder => {
            let mut entries: Vec<(String, bool, u64)> = std::fs::read_dir(path)
                .map_err(|e| format!("无法读取文件夹：{e}"))?
                .flatten()
                .map(|e| {
                    let meta = e.metadata().ok();
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        meta.as_ref().is_some_and(|m| m.is_dir()),
                        meta.map(|m| m.len()).unwrap_or(0),
                    )
                })
                .collect();
            entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
            let total = entries.len();
            let dirs = entries.iter().filter(|e| e.1).count();
            entries.truncate(500);
            Ok((Loaded::Folder(entries, total), format!("{dirs} 个文件夹，{} 个文件", total - dirs)))
        }
        Kind::Image => {
            let meta = image::image_dimensions(path).map(|(w, h)| format!("{w}×{h}")).unwrap_or_default();
            Ok((Loaded::Image, meta))
        }
        Kind::Markdown => {
            let (bytes, truncated) = office::read_prefix(path, TEXT_LIMIT)?;
            let (text, encoding) = office::decode_text(&bytes);
            let meta = format!("{encoding}{}", if truncated { " · 仅显示前 4 MB" } else { "" });
            Ok((Loaded::Markdown(text), meta))
        }
        Kind::Text => load_text(path),
        Kind::Delimited(delim) => {
            let (bytes, truncated) = office::read_prefix(path, 64 * 1024 * 1024)?;
            let (text, encoding) = office::decode_text(&bytes);
            let sheet = office::parse_delimited(&text, delim);
            let meta = format!(
                "{encoding} · {} 行{}",
                sheet.rows.len(),
                if truncated || sheet.truncated { "（已截断）" } else { "" }
            );
            let widths = vec![column_widths(&sheet)];
            Ok((Loaded::Table(vec![sheet], widths), meta))
        }
        Kind::Excel => {
            let sheets = office::read_xlsx(path, office::MAX_ROWS)?;
            let widths = sheets.iter().map(column_widths).collect();
            let meta = format!("{} 个工作表", sheets.len());
            Ok((Loaded::Table(sheets, widths), meta))
        }
        Kind::Word => {
            let blocks = office::read_docx(path)?;
            let images = blocks
                .iter()
                .map(|b| match b {
                    DocBlock::Image(bytes) => image_format(bytes).map(|f| Arc::new(Image::from_bytes(f, bytes.clone()))),
                    _ => None,
                })
                .collect();
            let words: usize = blocks
                .iter()
                .map(|b| match b {
                    DocBlock::Paragraph { text, .. } => text.chars().filter(|c| !c.is_whitespace()).count(),
                    _ => 0,
                })
                .sum();
            Ok((Loaded::Doc(blocks, images), format!("约 {words} 字")))
        }
        Kind::Other => {
            let (sample, _) = office::read_prefix(path, 8192)?;
            let is_ts = ext_of(path) == "ts";
            if !sample.is_empty() && office::looks_like_text(&sample) {
                load_text(path)
            } else if is_ts {
                Ok((Loaded::Media, String::new()))
            } else {
                Ok((Loaded::Info(win::type_name(path).unwrap_or_else(|| "文件".into())), String::new()))
            }
        }
        Kind::Audio | Kind::Video => Ok((Loaded::Media, String::new())),
    }
}

fn load_text(path: &Path) -> Result<(Loaded, String), String> {
    let (bytes, truncated) = office::read_prefix(path, TEXT_LIMIT)?;
    let (text, encoding) = office::decode_text(&bytes);
    let lines: Vec<SharedString> = text
        .split('\n')
        .map(|l| SharedString::from(l.trim_end_matches('\r').replace('\t', "    ")))
        .collect();
    let meta = format!(
        "{encoding} · {} 行{}",
        lines.len(),
        if truncated { " · 仅显示前 4 MB" } else { "" }
    );
    Ok((Loaded::Text(lines), meta))
}

fn image_format(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [0xFF, 0xD8, ..] => Some(ImageFormat::Jpeg),
        [b'G', b'I', b'F', ..] => Some(ImageFormat::Gif),
        [b'B', b'M', ..] => Some(ImageFormat::Bmp),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageFormat::Webp),
        _ => None,
    }
}

/// Column widths from the first rows: CJK characters count double.
fn column_widths(sheet: &Sheet) -> Vec<f32> {
    let cols = sheet.rows.iter().take(500).map(Vec::len).max().unwrap_or(0);
    (0..cols)
        .map(|c| {
            let chars = sheet
                .rows
                .iter()
                .take(500)
                .filter_map(|r| r.get(c))
                .map(|v| {
                    v.lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .map(|ch| if ch.is_ascii() { 1 } else { 2 })
                        .sum::<usize>()
                })
                .max()
                .unwrap_or(0);
            (chars as f32 * 7.2 + 20.).clamp(56., 320.)
        })
        .collect()
}

fn column_name(mut index: usize) -> String {
    let mut name = String::new();
    loop {
        name.insert(0, (b'A' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    name
}

fn run_style(style: RunStyle, theme: &Theme) -> HighlightStyle {
    HighlightStyle {
        font_weight: style.bold.then_some(FontWeight::BOLD),
        font_style: style.italic.then_some(gpui::FontStyle::Italic),
        color: style.link.then_some(theme.accent),
        underline: (style.underline || style.link).then_some(UnderlineStyle {
            thickness: px(1.),
            color: None,
            wavy: false,
        }),
        strikethrough: style.strike.then_some(StrikethroughStyle {
            thickness: px(1.),
            color: None,
        }),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Rendering

impl Preview {
    fn render_header(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string());
        let button = |id: &'static str, icon_name: &'static str, label: &'static str, theme: &Theme| {
            div()
                .id(id)
                .occlude()
                .h(px(30.))
                .px(px(10.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.5))
                .text_color(theme.text_muted)
                .cursor_pointer()
                .hover(|s| s.bg(theme.row_hover).text_color(theme.accent))
                .child(icon(icon_name, 14., theme.text_muted))
                .when(!label.is_empty(), |d| d.child(label))
        };
        div()
            .id("preview-header")
            .flex_none()
            .h(px(52.))
            .pl(px(16.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(theme.border)
            .window_control_area(WindowControlArea::Drag)
            .child(match self.icon.clone() {
                Some(image) => img(image).size(px(26.)).flex_none().into_any_element(),
                None => icon("file", 22., theme.text_muted).into_any_element(),
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).truncate().child(name))
                    .child(div().text_size(px(11.5)).text_color(theme.text_faint).truncate().child(self.meta.clone())),
            )
            .child(button("open-default", "external", "打开", theme).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| this.open_default(cx),
            )))
            .child(button("reveal", "folder-open", "", theme).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| this.reveal(cx),
            )))
            .child(
                div()
                    .id("preview-close")
                    .occlude()
                    .size(px(30.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(gpui::rgb(0xe81123)))
                    .group("preview-close")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, _| this.close(window)))
                    .child(icon("x", 15., theme.text_muted).group_hover("preview-close", |s| s.text_color(gpui::white()))),
            )
    }

    fn render_body(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        match &self.content {
            Content::Loading => centered(div().text_color(theme.text_faint).child("正在加载…")),
            Content::Error(err) => centered(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("info", 28., theme.danger))
                    .child(div().text_size(px(13.)).text_color(theme.danger).child(err.clone())),
            ),
            Content::Image => div()
                .size_full()
                .p(px(12.))
                .flex()
                .items_center()
                .justify_center()
                .child(img(self.path.clone()).size_full().object_fit(ObjectFit::Contain))
                .into_any_element(),
            Content::Markdown(source) => {
                let blocks = crate::markdown::render(source, self.path.parent(), theme);
                div()
                    .id("markdown")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(36.))
                    .py(px(24.))
                    .child(div().w_full().flex().flex_col().gap(px(8.)).children(blocks))
                    .into_any_element()
            }
            Content::Text { lines, list: state } => {
                let lines = lines.clone();
                let (faint, text) = (theme.text_faint, theme.text);
                let digits = lines.len().to_string().len().max(2) as f32;
                list(state.clone(), move |ix, _, _| {
                    let line = lines[ix].clone();
                    div()
                        .flex()
                        .px(px(12.))
                        .font_family("Consolas")
                        .text_size(px(13.))
                        .line_height(px(20.))
                        .child(
                            div()
                                .flex_none()
                                .w(px(digits * 8. + 14.))
                                .pr(px(14.))
                                .text_right()
                                .text_color(faint)
                                .child((ix + 1).to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_color(text)
                                .child(if line.is_empty() { SharedString::from(" ") } else { line }),
                        )
                        .into_any_element()
                })
                .size_full()
                .py(px(8.))
                .into_any_element()
            }
            Content::Table { sheets, widths } => self.render_table(sheets.clone(), widths.clone(), theme, cx),
            Content::Doc { blocks, images } => {
                let elements: Vec<AnyElement> = blocks
                    .iter()
                    .zip(images.iter())
                    .map(|(block, image)| doc_block(block, image.clone(), theme))
                    .collect();
                div()
                    .id("document")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(40.))
                    .py(px(28.))
                    .child(div().w_full().flex().flex_col().gap(px(6.)).children(elements))
                    .into_any_element()
            }
            Content::Folder { entries, total } => {
                let rows = entries.iter().map(|(name, dir, size)| {
                    div()
                        .h(px(30.))
                        .px(px(16.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .text_size(px(13.))
                        .child(icon(if *dir { "folder" } else { "file" }, 16., if *dir { theme.folder } else { theme.text_faint }))
                        .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                        .when(!*dir, |d| d.child(div().text_color(theme.text_faint).text_size(px(12.)).child(human_size(*size))))
                });
                div()
                    .id("folder")
                    .size_full()
                    .overflow_y_scroll()
                    .py(px(6.))
                    .children(rows)
                    .when(*total > entries.len(), |d| {
                        d.child(
                            div()
                                .px(px(16.))
                                .py(px(8.))
                                .text_size(px(12.))
                                .text_color(theme.text_faint)
                                .child(format!("……还有 {} 项", total - entries.len())),
                        )
                    })
                    .into_any_element()
            }
            Content::Info(type_name) => {
                let type_name = type_name.clone();
                centered(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(10.))
                        .child(match self.icon.clone() {
                            Some(image) => img(image).size(px(72.)).into_any_element(),
                            None => icon("file", 56., theme.text_faint).into_any_element(),
                        })
                        .child(div().text_size(px(13.)).text_color(theme.text_muted).child(type_name))
                        .child(div().text_size(px(12.)).text_color(theme.text_faint).child("这种文件暂不支持预览内容"))
                        .child(
                            div()
                                .id("info-open")
                                .mt(px(6.))
                                .h(px(34.))
                                .px(px(16.))
                                .rounded(px(9.))
                                .flex()
                                .items_center()
                                .bg(theme.accent)
                                .text_color(theme.accent_text)
                                .text_size(px(13.))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_default(cx)))
                                .child("用默认程序打开"),
                        ),
                )
            }
            Content::Media => self.render_media(theme, cx),
            Content::NeedFfmpeg => self.render_need_ffmpeg(theme, cx),
        }
    }

    fn render_table(&mut self, sheets: Arc<Vec<Sheet>>, widths: Arc<Vec<Vec<f32>>>, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let index = self.sheet.min(sheets.len().saturating_sub(1));
        let Some(sheet) = sheets.get(index) else {
            return centered(div().text_color(theme.text_faint).child("空表格"));
        };
        let col_widths = Arc::new(widths.get(index).cloned().unwrap_or_default());
        let row_count = sheet.rows.len();
        let digits = row_count.to_string().len().max(2) as f32;
        let num_w = digits * 8. + 18.;
        let total: f32 = num_w + col_widths.iter().sum::<f32>();
        let colors = (theme.text, theme.text_faint, theme.border, theme.kbd_bg);

        let cell = move |w: f32, text: SharedString, header: bool| {
            div()
                .flex_none()
                .w(px(w))
                .h_full()
                .px(px(8.))
                .flex()
                .items_center()
                .border_r_1()
                .border_color(colors.2)
                .when(header, |d| d.bg(colors.3).text_color(colors.1).justify_center())
                .child(div().min_w_0().truncate().child(text))
        };
        let header = div()
            .flex_none()
            .h(px(28.))
            .flex()
            .border_b_1()
            .border_color(colors.2)
            .text_size(px(12.))
            .child(cell(num_w, "".into(), true))
            .children(col_widths.iter().enumerate().map(|(i, w)| cell(*w, column_name(i).into(), true)));

        let sheets_for_rows = sheets.clone();
        let widths_for_rows = col_widths.clone();
        let rows = uniform_list("table-rows", row_count, move |range, _, _| {
            let sheet = &sheets_for_rows[index];
            range
                .map(|r| {
                    let row = &sheet.rows[r];
                    div()
                        .h(px(28.))
                        .flex()
                        .border_b_1()
                        .border_color(colors.2)
                        .text_size(px(12.5))
                        .text_color(colors.0)
                        .when(r == 0, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(cell(num_w, (r + 1).to_string().into(), true))
                        .children(widths_for_rows.iter().enumerate().map(|(c, w)| {
                            let value = row.get(c).map(|v| v.replace(['\r', '\n'], " ")).unwrap_or_default();
                            cell(*w, value.into(), false)
                        }))
                })
                .collect()
        })
        .flex_1();

        let tabs = (sheets.len() > 1).then(|| {
            div()
                .flex_none()
                .h(px(34.))
                .px(px(8.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .gap(px(4.))
                .children(sheets.iter().enumerate().map(|(i, s)| {
                    let selected = i == index;
                    div()
                        .id(("sheet-tab", i))
                        .h(px(24.))
                        .px(px(12.))
                        .rounded(px(6.))
                        .flex()
                        .items_center()
                        .text_size(px(12.))
                        .cursor_pointer()
                        .when(selected, |d| d.bg(theme.accent_soft).text_color(theme.accent))
                        .when(!selected, |d| d.text_color(theme.text_muted).hover(|s| s.bg(theme.row_hover)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.sheet = i;
                            cx.notify();
                        }))
                        .child(s.name.clone())
                }))
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("table-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .child(div().w(px(total)).h_full().flex().flex_col().child(header).child(rows)),
            )
            .children(tabs)
            .into_any_element()
    }

    fn render_media(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(player) = self.player.as_ref() else {
            return centered(div().child("…"));
        };
        let (position, duration, playing) = (player.position(), player.duration, player.is_playing());
        let fraction = duration.filter(|d| *d > 0.).map(|d| (position / d) as f32).unwrap_or(0.).clamp(0., 1.);
        let video = player.video_size.is_some();

        let bounds = self.bar_bounds.clone();
        let bar = div()
            .id("seek-bar")
            .flex_1()
            .h(px(18.))
            .flex()
            .items_center()
            .cursor_pointer()
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .w_full()
                    .h(px(5.))
                    .rounded_full()
                    .bg(theme.kbd_border)
                    .child(div().h_full().rounded_full().bg(theme.accent).w(gpui::relative(fraction))),
            )
            .relative()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    let b = this.bar_bounds.get();
                    if let (Some(player), true) = (this.player.as_mut(), b.size.width > px(0.)) {
                        if let Some(d) = player.duration {
                            let f = ((ev.position.x - b.left()) / b.size.width).clamp(0., 1.);
                            player.seek(d * f as f64);
                            cx.notify();
                        }
                    }
                }),
            );
        let controls = div()
            .flex_none()
            .h(px(52.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(14.))
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("play-toggle")
                    .size(px(34.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.accent)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(p) = this.player.as_mut() {
                            p.toggle();
                        }
                        cx.notify();
                    }))
                    .child(icon(if playing { "pause" } else { "play" }, 15., theme.accent_text)),
            )
            .child(bar)
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(theme.text_muted)
                    .font_family("Consolas")
                    .child(format!(
                        "{} / {}",
                        media::format_time(position),
                        duration.map(media::format_time).unwrap_or_else(|| "--:--".into())
                    )),
            );

        let stage = if video {
            div()
                .flex_1()
                .min_h_0()
                .bg(gpui::black())
                .flex()
                .items_center()
                .justify_center()
                .children(self.frame.clone().map(|f| img(f).size_full().object_fit(ObjectFit::Contain)))
                .into_any_element()
        } else {
            let (fg, bg) = theme.tint("pink");
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .child(
                    div()
                        .size(px(84.))
                        .rounded(px(22.))
                        .bg(bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("music", 40., fg)),
                )
                .child(div().text_size(px(12.)).text_color(theme.text_faint).child(if playing { "正在播放" } else { "已暂停" }))
                .into_any_element()
        };
        div().size_full().flex().flex_col().child(stage).child(controls).into_any_element()
    }

    fn render_need_ffmpeg(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = cx.global::<FfmpegState>();
        let progress = state.download.as_ref().map(|p| p.fraction().unwrap_or(0.));
        let error = state.error.clone();
        let button = |id: &'static str, label: String, primary: bool, theme: &Theme| {
            div()
                .id(id)
                .h(px(34.))
                .px(px(16.))
                .rounded(px(9.))
                .flex()
                .items_center()
                .text_size(px(13.))
                .cursor_pointer()
                .when(primary, |d| d.bg(theme.accent).text_color(theme.accent_text))
                .when(!primary, |d| d.border_1().border_color(theme.border_strong).hover(|s| s.bg(theme.row_hover)))
                .child(label)
        };
        let launcher = self.launcher.clone();
        let launcher2 = self.launcher.clone();
        centered(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(10.))
                .px(px(24.))
                .child(icon(if self.kind == Kind::Video { "monitor" } else { "music" }, 36., theme.accent))
                .child(div().text_size(px(14.)).child("播放这个文件需要 FFmpeg"))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_faint)
                        .child("FFmpeg 是开源的音视频解码工具，下载一次即可，以后所有视频都能直接预览"),
                )
                .child(
                    div()
                        .mt(px(6.))
                        .flex()
                        .gap(px(8.))
                        .child(match progress {
                            Some(f) => button("ffmpeg-progress", format!("正在下载… {:.0}%", f * 100.), true, theme).into_any_element(),
                            None => button("ffmpeg-download", format!("一键下载（{}）", ffmpeg::DOWNLOAD_SIZE_HINT), true, theme)
                                .on_click(move |_, _, cx| {
                                    launcher.update(cx, |l, cx| l.download_ffmpeg(cx)).ok();
                                })
                                .into_any_element(),
                        })
                        .child(button("ffmpeg-choose", "选择本地 ffmpeg.exe…".into(), false, theme).on_click(move |_, _, cx| {
                            launcher2.update(cx, |l, cx| l.choose_ffmpeg(cx)).ok();
                        }))
                        .child(button("ffmpeg-open", "用默认程序打开".into(), false, theme).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| this.open_default(cx),
                        ))),
                )
                .when_some(error, |d, err| d.child(div().text_size(px(12.)).text_color(theme.danger).child(err))),
        )
    }
}

fn centered(child: impl IntoElement) -> AnyElement {
    div().size_full().flex().items_center().justify_center().child(child).into_any_element()
}

fn doc_block(block: &DocBlock, image: Option<Arc<Image>>, theme: &Theme) -> AnyElement {
    match block {
        DocBlock::Paragraph { text, runs, heading, list_level } => {
            if text.trim().is_empty() {
                return div().h(px(8.)).into_any_element();
            }
            let highlights: Vec<_> = runs.iter().map(|(r, s)| (r.clone(), run_style(*s, theme))).collect();
            let styled = StyledText::new(text.clone()).with_highlights(highlights);
            let (size, weight) = match heading {
                1 => (22., FontWeight::BOLD),
                2 => (18.5, FontWeight::BOLD),
                3 => (16., FontWeight::SEMIBOLD),
                4..=6 => (14.5, FontWeight::SEMIBOLD),
                _ => (14., FontWeight::NORMAL),
            };
            let body = div()
                .text_size(px(size))
                .line_height(px(size * 1.6))
                .font_weight(weight)
                .when(*heading > 0, |d| d.mt(px(8.)))
                .child(styled);
            match list_level {
                Some(level) => div()
                    .pl(px(*level as f32 * 18. + 4.))
                    .flex()
                    .gap(px(8.))
                    .child(div().flex_none().text_color(theme.accent).line_height(px(22.)).child("•"))
                    .child(body.flex_1().min_w_0())
                    .into_any_element(),
                None => body.into_any_element(),
            }
        }
        DocBlock::Table(rows) => {
            let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
            div()
                .my(px(6.))
                .grid()
                .grid_cols(cols as u16)
                .border_t_1()
                .border_l_1()
                .border_color(theme.border)
                .children(rows.iter().enumerate().flat_map(|(r, row)| {
                    (0..cols).map(move |c| {
                        div()
                            .px(px(8.))
                            .py(px(5.))
                            .border_b_1()
                            .border_r_1()
                            .border_color(theme.border)
                            .text_size(px(13.))
                            .when(r == 0, |d| d.bg(theme.kbd_bg).font_weight(FontWeight::SEMIBOLD))
                            .child(row.get(c).cloned().unwrap_or_default())
                    })
                }))
                .into_any_element()
        }
        DocBlock::Image(_) => match image {
            Some(image) => div()
                .my(px(6.))
                .flex()
                .justify_center()
                .child(img(image).max_w_full().max_h(px(480.)).object_fit(ObjectFit::Contain))
                .into_any_element(),
            None => div()
                .text_size(px(12.))
                .text_color(theme.text_faint)
                .child("[图片：暂不支持此格式]")
                .into_any_element(),
        },
    }
}

impl Render for Preview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let header = self.render_header(&theme, cx);
        let body = self.render_body(&theme, cx);
        div()
            .key_context("Preview")
            .on_action(cx.listener(Self::close_action))
            .on_action(cx.listener(Self::toggle_action))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .text_color(theme.text)
            .text_size(px(14.))
            .child(header)
            .child(
                // Focus lives here rather than on the root so the header stays draggable.
                div()
                    .id("preview-body")
                    .track_focus(&self.focus)
                    .flex_1()
                    .min_h_0()
                    .child(body),
            )
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        hook::PREVIEW_OPEN.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_names() {
        assert_eq!(kind_of(Path::new("a.MD")), Kind::Markdown);
        assert_eq!(kind_of(Path::new("a.csv")), Kind::Delimited(','));
        assert_eq!(kind_of(Path::new("a.mkv")), Kind::Video);
        assert_eq!(kind_of(Path::new("a.flac")), Kind::Audio);
        assert_eq!(kind_of(Path::new("main.rs")), Kind::Text);
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(27), "AB");
        assert_eq!(human_size(1536), "1.5 KB");
    }
}
