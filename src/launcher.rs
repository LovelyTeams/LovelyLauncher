//! Launcher state and behaviour. Rendering lives in `view.rs` and `dialogs.rs`.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use gpui::{
    App, AppContext, BorrowAppContext, Context, Entity, ExternalPaths, FocusHandle, Focusable, KeyBinding,
    PathPromptOptions, Pixels, Point, RenderImage, ScrollHandle, SharedString, Subscription,
    Window, WindowAppearance, WindowBackgroundAppearance, actions,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{
    config::{Config, Tool, now_secs},
    dialogs::Dialog,
    download::{self, Progress},
    input::{InputEvent, TextInput},
    search::SearchKey,
    system::{AppEvent, HotkeyService},
    theme::Theme,
    win,
};

actions!(
    launcher,
    [
        SelectPrev,
        SelectNext,
        Confirm,
        ConfirmAdmin,
        Dismiss,
        FocusSearch,
        NewTerminal,
        OpenProject,
        TakeScreenshot,
        QuickNote,
        OpenSettings,
        AddTool,
        FocusNextField,
        FocusPrevField,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let launcher = Some("Launcher");
    let dialog = Some("Dialog");
    cx.bind_keys([
        KeyBinding::new("up", SelectPrev, launcher),
        KeyBinding::new("down", SelectNext, launcher),
        KeyBinding::new("enter", Confirm, launcher),
        KeyBinding::new("ctrl-enter", ConfirmAdmin, launcher),
        KeyBinding::new("escape", Dismiss, launcher),
        KeyBinding::new("ctrl-k", FocusSearch, launcher),
        KeyBinding::new("ctrl-t", NewTerminal, launcher),
        KeyBinding::new("ctrl-o", OpenProject, launcher),
        KeyBinding::new("ctrl-shift-s", TakeScreenshot, launcher),
        KeyBinding::new("ctrl-n", QuickNote, launcher),
        KeyBinding::new("ctrl-,", OpenSettings, launcher),
        KeyBinding::new("ctrl-d", AddTool, launcher),
        KeyBinding::new("tab", FocusNextField, dialog),
        KeyBinding::new("shift-tab", FocusPrevField, dialog),
        KeyBinding::new("enter", Confirm, dialog),
        KeyBinding::new("escape", Dismiss, dialog),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickAction {
    NewTerminal,
    OpenProject,
    Screenshot,
    Note,
}

impl QuickAction {
    pub const ALL: [QuickAction; 4] = [
        QuickAction::NewTerminal,
        QuickAction::OpenProject,
        QuickAction::Screenshot,
        QuickAction::Note,
    ];

    pub fn label(self) -> &'static str {
        match self {
            QuickAction::NewTerminal => "新建终端",
            QuickAction::OpenProject => "打开项目",
            QuickAction::Screenshot => "截图",
            QuickAction::Note => "快速笔记",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            QuickAction::NewTerminal => "terminal",
            QuickAction::OpenProject => "folder-open",
            QuickAction::Screenshot => "scan",
            QuickAction::Note => "note",
        }
    }

    pub fn keys(self) -> &'static [&'static str] {
        match self {
            QuickAction::NewTerminal => &["Ctrl", "T"],
            QuickAction::OpenProject => &["Ctrl", "O"],
            QuickAction::Screenshot => &["Ctrl", "Shift", "S"],
            QuickAction::Note => &["Ctrl", "N"],
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            QuickAction::NewTerminal => "action:terminal",
            QuickAction::OpenProject => "action:project",
            QuickAction::Screenshot => "action:screenshot",
            QuickAction::Note => "action:note",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.key() == key)
    }
}

/// Something the user can activate.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Tool(String),
    App(PathBuf),
    Project(String),
    Action(QuickAction),
    Url(String),
    WebSearch(String),
    Command(String),
    /// A file or folder found by Everything.
    File { path: PathBuf, is_dir: bool },
}

impl Target {
    fn history_key(&self) -> Option<String> {
        match self {
            Target::Tool(id) => Some(format!("tool:{id}")),
            Target::App(path) => Some(format!("app:{}", path.display())),
            Target::Action(action) => Some(action.key().to_string()),
            _ => None,
        }
    }

    fn from_history_key(key: &str) -> Option<Self> {
        if let Some(id) = key.strip_prefix("tool:") {
            Some(Target::Tool(id.to_string()))
        } else if let Some(path) = key.strip_prefix("app:") {
            Some(Target::App(PathBuf::from(path)))
        } else {
            QuickAction::from_key(key).map(Target::Action)
        }
    }
}

/// How an item's icon is drawn.
#[derive(Clone, Debug)]
pub enum Visual {
    Glyph { icon: &'static str, tint: String },
    /// The shell icon of a file; falls back to `fallback` while loading or if missing.
    File {
        target: String,
        tint: String,
        fallback: &'static str,
    },
    Folder,
    /// A user-supplied icon image.
    Image { path: String },
    /// Generic shell icon for a file extension (shared by all files of that type).
    FileType { ext: String },
}

#[derive(Clone, Debug)]
pub struct ResultItem {
    pub target: Target,
    pub title: SharedString,
    pub subtitle: SharedString,
    pub badge: &'static str,
    pub visual: Visual,
    score: i32,
}

pub struct RecentItem {
    pub target: Target,
    pub key: String,
    pub name: SharedString,
    pub when: SharedString,
    pub visual: Visual,
}

/// Which tools the "全部工具" grid shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolFilter {
    All,
    Uncategorized,
    Category(String),
}

/// Payload while a tool tile is being dragged onto a category tab.
#[derive(Clone, Debug)]
pub struct DraggedTool {
    pub id: String,
    pub name: SharedString,
}

/// Small floating label that follows the cursor during a drag.
pub struct DragPreview {
    pub name: SharedString,
}

impl gpui::Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled, div, px};
        let theme = Theme::get(cx);
        div()
            .px(px(12.))
            .py(px(6.))
            .rounded(px(9.))
            .bg(theme.accent)
            .text_color(theme.accent_text)
            .text_size(px(13.))
            .opacity(0.92)
            .child(format!("移动“{}”到…", self.name))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AiState {
    Streaming,
    Done,
    Failed(String),
}

pub struct AiAnswer {
    pub question: String,
    pub provider_id: String,
    pub model: String,
    pub text: String,
    pub state: AiState,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

enum AiMsg {
    Text(String),
    Finished(Result<(), String>),
}

#[derive(Clone, Debug)]
pub enum MenuTarget {
    File { path: PathBuf, is_dir: bool },
    Category(String),
    Tool(String),
    App(PathBuf),
    Project(String),
    History(String),
}

pub struct ContextMenu {
    pub target: MenuTarget,
    pub position: Point<Pixels>,
}

pub struct Toast {
    pub message: SharedString,
    pub error: bool,
    generation: u64,
}

pub struct AppEntry {
    pub app: win::InstalledApp,
    pub key: SearchKey,
    /// The shortcut's own file name, which often differs from the localized display name.
    pub file_key: Option<SearchKey>,
}

enum IconSlot {
    Loading,
    Ready(Arc<RenderImage>),
    Missing,
}

pub struct Launcher {
    pub(crate) config: Config,
    pub(crate) hwnd: Option<isize>,
    /// Last known OS light/dark appearance, for theme refreshes without a `Window`.
    appearance: WindowAppearance,
    pub(crate) focus: FocusHandle,
    pub(crate) search: Entity<TextInput>,
    pub(crate) search_text: String,
    pub(crate) results: Vec<ResultItem>,
    pub(crate) selected: usize,
    pub(crate) results_scroll: ScrollHandle,
    /// The latest `@ai` answer (kept while its question is still in the box).
    pub(crate) ai: Option<AiAnswer>,
    pub(crate) ai_scroll: ScrollHandle,
    pub(crate) body_scroll: ScrollHandle,
    pub(crate) apps: Arc<Vec<AppEntry>>,
    icons: HashMap<String, IconSlot>,
    /// In-flight downloads of preset programs, keyed by tool id.
    pub(crate) downloads: HashMap<String, Arc<Progress>>,
    pub(crate) menu: Option<ContextMenu>,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) toast: Option<Toast>,
    toast_generation: u64,
    pub(crate) logo: Arc<RenderImage>,
    pub(crate) tool_filter: ToolFilter,
    /// Compact search-bar mode (opened by the global hotkey).
    pub(crate) mini: bool,
    /// Full-window geometry to restore when leaving mini mode.
    full_rect: Option<win::WindowRect>,
    everything_generation: u64,
    pub(crate) tools_scroll: ScrollHandle,
    pub(crate) show_all_favorites: bool,
    pub(crate) metrics: crate::view::Metrics,
    pub(crate) code_available: bool,
    /// Set while a native file dialog is open so hide-on-blur doesn't fire.
    pub(crate) prompting: bool,
    hide_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl Launcher {
    pub fn new(
        mut config: Config,
        hotkey_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        config.settings.autostart = win::is_autostart();

        let search = cx.new(|cx| TextInput::new("搜索工具、应用、项目，或输入命令…", cx));
        let hwnd = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
            _ => None,
        };

        let subscriptions = vec![
            cx.subscribe(&search, |this, input, _: &InputEvent, cx| {
                // Ignore pinyin that the IME is still composing.
                let text = input.read(cx).committed_text();
                if text != this.search_text {
                    this.search_text = text;
                    this.on_query_changed(cx);
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                this.apply_theme(window, cx);
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active()
                    && (this.mini || this.config.settings.hide_on_blur)
                    && !this.prompting
                    && this.dialog.is_none()
                {
                    this.hide(window, cx);
                }
            }),
        ];

        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| {
                if this.config.settings.close_to_tray {
                    this.hide(window, cx);
                    false
                } else {
                    this.config.save();
                    cx.quit();
                    true
                }
            })
            .unwrap_or(true)
        });

        let logo_bytes = include_bytes!(concat!(env!("OUT_DIR"), "/logo_96.bgra")).to_vec();
        let logo = Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::from_raw(96, 96, logo_bytes).expect("logo size"),
        )]));

        let mut this = Self {
            config,
            hwnd,
            appearance: window.appearance(),
            focus: cx.focus_handle(),
            search,
            search_text: String::new(),
            results: Vec::new(),
            selected: 0,
            results_scroll: ScrollHandle::new(),
            ai: None,
            ai_scroll: ScrollHandle::new(),
            body_scroll: ScrollHandle::new(),
            apps: Arc::new(Vec::new()),
            icons: HashMap::new(),
            downloads: HashMap::new(),
            menu: None,
            dialog: None,
            toast: None,
            toast_generation: 0,
            logo,
            tool_filter: ToolFilter::All,
            mini: false,
            full_rect: None,
            everything_generation: 0,
            tools_scroll: ScrollHandle::new(),
            show_all_favorites: false,
            metrics: Default::default(),
            code_available: false,
            prompting: false,
            hide_generation: 0,
            _subscriptions: subscriptions,
        };
        this.apply_theme(window, cx);
        window.focus(&this.search.focus_handle(cx));
        if let Some(err) = hotkey_error {
            this.show_toast(err, true, cx);
        }
        this.scan_apps(cx);
        this
    }

    // -----------------------------------------------------------------------
    // Theme & window

    /// Rebuild theme tokens from settings (colors only).
    pub fn refresh_theme(&mut self, cx: &mut Context<Self>) {
        cx.set_global(Theme::from_settings(&self.config.settings, self.appearance));
        cx.notify();
    }

    /// Rebuild the theme and the window backdrop.
    pub fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let blur = self.config.settings.blur_background;
        self.appearance = window.appearance();
        cx.set_global(Theme::from_settings(&self.config.settings, self.appearance));
        window.set_background_appearance(if blur {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Opaque
        });
        cx.notify();
    }

    pub fn handle_app_event(&mut self, event: AppEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            AppEvent::Hotkey => {
                let visible = self.hwnd.is_some_and(win::is_window_visible);
                if visible && window.is_window_active() {
                    self.hide(window, cx);
                } else if self.config.settings.mini_on_hotkey && !(visible && !self.mini) {
                    self.show_mini(window, cx);
                } else {
                    self.show(window, cx);
                }
            }
            AppEvent::Toggle => {
                let visible = self.hwnd.is_some_and(win::is_window_visible);
                if visible && window.is_window_active() && !self.mini {
                    self.hide(window, cx);
                } else {
                    self.restore_full(cx);
                    self.show(window, cx);
                }
            }
            AppEvent::Show => {
                self.restore_full(cx);
                self.show(window, cx);
            }
            AppEvent::Settings => {
                self.restore_full(cx);
                self.show(window, cx);
                self.open_settings(window, cx);
            }
            AppEvent::Quit => {
                self.config.save();
                cx.quit();
            }
        }
    }

    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_generation += 1;
        if let Some(hwnd) = self.hwnd {
            win::show_window(hwnd);
        }
        window.activate_window();
        if self.dialog.is_none() {
            let search = self.search.clone();
            search.update(cx, |input, cx| input.select_all_text(cx));
            window.focus(&search.focus_handle(cx));
        }
        cx.notify();
    }

    pub fn hide(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if let Some(hwnd) = self.hwnd {
            win::hide_window(hwnd);
        }
        // Once we've been hidden for a moment, hand our memory back to the OS.
        self.hide_generation += 1;
        let generation = self.hide_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let still_hidden = this
                .read_with(cx, |this, _| {
                    this.hide_generation == generation && !this.hwnd.is_some_and(win::is_window_visible)
                })
                .unwrap_or(false);
            if still_hidden {
                win::trim_working_set();
            }
        })
        .detach();
    }

    // -----------------------------------------------------------------------
    // Toasts

    pub fn show_toast(&mut self, message: impl Into<SharedString>, error: bool, cx: &mut Context<Self>) {
        self.toast_generation += 1;
        let generation = self.toast_generation;
        self.toast = Some(Toast {
            message: message.into(),
            error,
            generation,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(if error { 5000 } else { 2500 }))
                .await;
            this.update(cx, |this, cx| {
                if this.toast.as_ref().is_some_and(|t| t.generation == generation) {
                    this.toast = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // -----------------------------------------------------------------------
    // Start Menu apps & icons

    pub fn scan_apps(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let apps = cx
                .background_spawn(async move {
                    win::scan_start_menu()
                        .into_iter()
                        .map(|app| AppEntry {
                            key: SearchKey::new(&app.name),
                            file_key: app
                                .path
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .filter(|stem| !stem.eq_ignore_ascii_case(&app.name))
                                .map(|stem| SearchKey::new(&stem)),
                            app,
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let code_available = cx
                .background_spawn(async { win::search_path("code.cmd").is_some() })
                .await;
            this.update(cx, |this, cx| {
                this.apps = Arc::new(apps);
                this.code_available = code_available;
                this.refresh_results();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Cached shell icon for `target`, starting a background load on first request.
    pub fn file_icon(&mut self, target: &str, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        let resolved = if target == "terminal" {
            self.terminal()
        } else {
            target.to_string()
        };
        self.cached_icon(target.to_string(), cx, move || {
            let path = win::resolve_target_path(&resolved)?;
            let b = win::file_icon(&win::icon_source(&path))?;
            // Shell bitmaps are already BGRA, which is what GPUI expects.
            image::RgbaImage::from_raw(b.width, b.height, b.bgra)
        })
    }

    /// Cached user icon image, downscaled so large pictures don't cost GPU memory.
    pub fn image_icon(&mut self, path: &str, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        let file = PathBuf::from(path);
        self.cached_icon(format!("img:{path}"), cx, move || {
            let mut img = crate::icon_image::load_square(&file, 96)?;
            crate::icon_image::rgba_to_bgra(&mut img);
            Some(img)
        })
    }

    fn cached_icon(
        &mut self,
        key: String,
        cx: &mut Context<Self>,
        load: impl FnOnce() -> Option<image::RgbaImage> + Send + 'static,
    ) -> Option<Arc<RenderImage>> {
        match self.icons.get(&key) {
            Some(IconSlot::Ready(image)) => return Some(image.clone()),
            Some(IconSlot::Loading | IconSlot::Missing) => return None,
            None => {}
        }
        self.icons.insert(key.clone(), IconSlot::Loading);
        cx.spawn(async move |this, cx| {
            let bgra = cx.background_spawn(async move { load() }).await;
            let slot = match bgra {
                Some(img) => IconSlot::Ready(Arc::new(RenderImage::new(vec![image::Frame::new(img)]))),
                None => IconSlot::Missing,
            };
            this.update(cx, |this, cx| {
                this.icons.insert(key, slot);
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }

    pub fn type_icon(&mut self, ext: &str, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        let ext_owned = ext.to_string();
        self.cached_icon(format!("ext:{ext}"), cx, move || {
            let b = win::file_type_icon(&ext_owned)?;
            image::RgbaImage::from_raw(b.width, b.height, b.bgra)
        })
    }

    /// Forget a cached icon so it is reloaded (e.g. after a download finishes).
    pub fn invalidate_icon(&mut self, key: &str) {
        self.icons.remove(key);
    }

    pub fn tool_visual(tool: &Tool) -> Visual {
        let tint = tool.color.clone();
        if !tool.icon_path.is_empty() && Path::new(&tool.icon_path).exists() {
            return Visual::Image {
                path: tool.icon_path.clone(),
            };
        }
        if !tool.icon.is_empty() {
            if let Some(icon) = crate::icons::static_name(&tool.icon) {
                return Visual::Glyph { icon, tint };
            }
        }
        let target = tool.target.trim();
        if is_url(target) {
            return Visual::Glyph { icon: "globe", tint };
        }
        Visual::File {
            target: target.to_string(),
            tint,
            fallback: if tool.download_url.is_empty() { "app" } else { "download" },
        }
    }

    /// A preset program that still has to be downloaded before it can run.
    pub fn needs_download(tool: &Tool) -> bool {
        !tool.download_url.is_empty() && !Path::new(&win::expand_env(&tool.target)).exists()
    }

    // -----------------------------------------------------------------------
    // Search

    pub fn refresh_results_with(&mut self, query: &str) {
        let raw = query.trim();
        let q = raw.to_lowercase();
        self.results.clear();
        self.selected = 0;
        self.results_scroll.set_offset(Default::default());
        if q.is_empty() {
            return;
        }

        let mut items: Vec<ResultItem> = Vec::new();

        if let Some(cmd) = raw.strip_prefix('>') {
            let cmd = cmd.trim();
            if !cmd.is_empty() {
                items.push(ResultItem {
                    target: Target::Command(cmd.to_string()),
                    title: format!("运行命令：{cmd}").into(),
                    subtitle: "在新的命令行窗口中执行".into(),
                    badge: "命令",
                    visual: Visual::Glyph {
                        icon: "terminal",
                        tint: "slate".into(),
                    },
                    score: 10_000,
                });
            }
        }
        if looks_like_url(raw) {
            items.push(ResultItem {
                target: Target::Url(normalize_url(raw)),
                title: format!("打开网址 {raw}").into(),
                subtitle: normalize_url(raw).into(),
                badge: "网页",
                visual: Visual::Glyph {
                    icon: "link",
                    tint: "sky".into(),
                },
                score: 9_000,
            });
        }

        let usage_bonus = |config: &Config, key: &str| -> i32 {
            config.usage(key).map_or(0, |e| {
                let recent = if now_secs().saturating_sub(e.last_used) < 86_400 { 30 } else { 0 };
                (e.count.min(20) as i32) * 8 + recent
            })
        };

        for tool in &self.config.tools {
            let program = Path::new(tool.target.trim())
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let program_score = (!program.is_empty())
                .then(|| SearchKey::new(&program).score(&q).map(|s| s - 40))
                .flatten();
            let name_score = SearchKey::new(&tool.name).score(&q).max(program_score);
            let desc_score = (!tool.description.is_empty()
                && tool.description.to_lowercase().contains(&q))
            .then_some(200);
            if let Some(score) = name_score.max(desc_score) {
                items.push(ResultItem {
                    target: Target::Tool(tool.id.clone()),
                    title: tool.name.clone().into(),
                    subtitle: if tool.description.is_empty() {
                        tool.target.clone().into()
                    } else {
                        tool.description.clone().into()
                    },
                    badge: "工具",
                    visual: Self::tool_visual(tool),
                    score: score + 60 + usage_bonus(&self.config, &format!("tool:{}", tool.id)),
                });
            }
        }

        for entry in self.apps.iter() {
            let file_score = entry
                .file_key
                .as_ref()
                .and_then(|k| k.score(&q))
                .map(|s| s - 40);
            if let Some(score) = entry.key.score(&q).max(file_score) {
                let key = format!("app:{}", entry.app.path.display());
                items.push(ResultItem {
                    target: Target::App(entry.app.path.clone()),
                    title: entry.app.name.clone().into(),
                    subtitle: "应用程序".into(),
                    badge: "应用",
                    visual: Visual::File {
                        target: entry.app.path.display().to_string(),
                        tint: "slate".into(),
                        fallback: "app",
                    },
                    score: score + usage_bonus(&self.config, &key),
                });
            }
        }

        for project in &self.config.projects {
            let score = SearchKey::new(&project.name)
                .score(&q)
                .or_else(|| project.path.to_lowercase().contains(&q).then_some(150));
            if let Some(score) = score {
                items.push(ResultItem {
                    target: Target::Project(project.path.clone()),
                    title: project.name.clone().into(),
                    subtitle: project.path.clone().into(),
                    badge: "项目",
                    visual: Visual::Folder,
                    score: score + 20,
                });
            }
        }

        for action in QuickAction::ALL {
            if let Some(score) = SearchKey::new(action.label()).score(&q) {
                items.push(ResultItem {
                    target: Target::Action(action),
                    title: action.label().into(),
                    subtitle: format!("快捷操作 · {}", action.keys().join("+")).into(),
                    badge: "操作",
                    visual: Visual::Glyph {
                        icon: action.icon(),
                        tint: "pink".into(),
                    },
                    score: score + usage_bonus(&self.config, action.key()),
                });
            }
        }

        items.sort_by(|a, b| b.score.cmp(&a.score));
        items.truncate(50);
        items.push(ResultItem {
            target: Target::WebSearch(raw.to_string()),
            title: format!("在网络上搜索“{raw}”").into(),
            subtitle: format!("使用{}", self.config.settings.search_engine_name()).into(),
            badge: "搜索",
            visual: Visual::Glyph {
                icon: "search",
                tint: "blue".into(),
            },
            score: i32::MIN,
        });
        self.results = items;
    }

    pub fn refresh_results(&mut self) {
        if self.ai_request().is_some() {
            self.results.clear();
            self.selected = 0;
            return;
        }
        let query = self.search_text.clone();
        self.refresh_results_with(&query);
    }

    // -----------------------------------------------------------------------
    // @ai quick questions

    /// `@ai …` / `@<provider id> …` in the search box → (provider, question).
    pub fn ai_request(&self) -> Option<(crate::config::AiProvider, String)> {
        let (id, question) = crate::ai::parse_prefix(&self.search_text)?;
        let settings = &self.config.settings;
        let provider = if id == "ai" {
            settings.active_ai()?.clone()
        } else {
            settings.ai_providers.iter().find(|p| p.id == id)?.clone()
        };
        Some((provider, question))
    }

    /// The answer to show for the current question, if one was asked.
    pub fn current_answer(&self) -> Option<&AiAnswer> {
        let (provider, question) = self.ai_request()?;
        self.ai
            .as_ref()
            .filter(|a| a.question == question && a.provider_id == provider.id)
    }

    pub fn start_ai(&mut self, cx: &mut Context<Self>) {
        let Some((provider, question)) = self.ai_request() else {
            return;
        };
        if question.is_empty() {
            return;
        }
        if let Some(previous) = &self.ai {
            previous.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.ai = Some(AiAnswer {
            question: question.clone(),
            provider_id: provider.id.clone(),
            model: provider.model.clone(),
            text: String::new(),
            state: AiState::Streaming,
            cancel: cancel.clone(),
        });
        self.ai_scroll.set_offset(Default::default());
        self.sync_mini_size(cx);
        cx.notify();

        // The HTTP call blocks, so it gets its own thread; text streams back over a channel.
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<AiMsg>();
        let thread_cancel = cancel.clone();
        std::thread::Builder::new()
            .name("ai".into())
            .spawn(move || {
                let result = crate::ai::ask(&provider, &question, &thread_cancel, |text| {
                    let _ = tx.unbounded_send(AiMsg::Text(text.to_string()));
                });
                let _ = tx.unbounded_send(AiMsg::Finished(result));
            })
            .ok();

        cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            while let Some(msg) = rx.next().await {
                let alive = this
                    .update(cx, |this, cx| {
                        let Some(answer) = this.ai.as_mut().filter(|a| Arc::ptr_eq(&a.cancel, &cancel)) else {
                            return false;
                        };
                        match msg {
                            AiMsg::Text(text) => answer.text.push_str(&text),
                            AiMsg::Finished(Ok(())) => answer.state = AiState::Done,
                            AiMsg::Finished(Err(err)) => answer.state = AiState::Failed(err),
                        }
                        this.ai_scroll.scroll_to_bottom();
                        cx.notify();
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

    /// Stop a streaming answer; returns whether one was running.
    pub fn stop_ai(&mut self, cx: &mut Context<Self>) -> bool {
        match self.ai.as_mut() {
            Some(answer) if answer.state == AiState::Streaming => {
                answer.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                answer.state = AiState::Done;
                answer.text.push_str("\n\n（已停止）");
                cx.notify();
                true
            }
            _ => false,
        }
    }

    fn on_query_changed(&mut self, cx: &mut Context<Self>) {
        self.refresh_results();
        self.schedule_everything(cx);
        self.sync_mini_size(cx);
        cx.notify();
    }

    /// Ask Everything for matching files after a short pause in typing.
    fn schedule_everything(&mut self, cx: &mut Context<Self>) {
        self.everything_generation += 1;
        let generation = self.everything_generation;
        let query = self.search_text.trim().to_string();
        if !self.config.settings.everything
            || self.ai_request().is_some()
            || query.chars().count() < 2
            || query.starts_with('>')
            || looks_like_url(&query)
        {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            let current = this
                .read_with(cx, |this, _| this.everything_generation == generation)
                .unwrap_or(false);
            if !current {
                return;
            }
            let search_query = query.clone();
            let hits = cx
                .background_spawn(async move {
                    crate::everything::search(&search_query, 80)
                        .map(|hits| crate::everything::rank(hits, &search_query, 20))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.everything_generation != generation {
                    return;
                }
                if let Ok(hits) = hits {
                    this.merge_file_results(hits);
                    this.sync_mini_size(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Insert Everything hits after the local results, before the web-search row.
    fn merge_file_results(&mut self, hits: Vec<crate::everything::FileHit>) {
        let web = self.results.pop();
        self.results.retain(|r| !matches!(r.target, Target::File { .. }));
        for hit in hits {
            let name = hit
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| hit.path.display().to_string());
            let parent = hit
                .path
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let ext = hit
                .path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let visual = if hit.is_dir {
                Visual::Folder
            } else if matches!(ext.as_str(), "exe" | "lnk" | "ico" | "url" | "msc" | "appref-ms") {
                // These carry their own icons.
                Visual::File {
                    target: hit.path.display().to_string(),
                    tint: "slate".into(),
                    fallback: "file",
                }
            } else {
                Visual::FileType { ext }
            };
            self.results.push(ResultItem {
                target: Target::File {
                    path: hit.path.clone(),
                    is_dir: hit.is_dir,
                },
                title: name.into(),
                subtitle: parent.into(),
                badge: if hit.is_dir { "文件夹" } else { "文件" },
                visual,
                score: 0,
            });
        }
        self.results.extend(web);
        self.selected = self.selected.min(self.results.len().saturating_sub(1));
    }

    // -----------------------------------------------------------------------
    // Mini (search bar) mode

    pub const MINI_WIDTH: f32 = 700.;
    pub const MINI_BAR: f32 = 64.;
    pub const MINI_ROW: f32 = 56.;
    pub const MINI_MAX_ROWS: usize = 8;

    fn mini_height(&self) -> f32 {
        if self.ai_request().is_some() {
            // bar + divider + answer panel (or the one-line hint) + footer
            let body = if self.current_answer().is_some() { 400. } else { 72. };
            return Self::MINI_BAR + 1. + body + 40.;
        }
        let rows = self.results.len().min(Self::MINI_MAX_ROWS);
        if rows == 0 {
            Self::MINI_BAR
        } else {
            // bar + divider + list padding + rows + footer
            Self::MINI_BAR + 1. + 12. + rows as f32 * Self::MINI_ROW + 40.
        }
    }

    // Native moves/resizes synchronously send WM_SIZE back into GPUI. Doing that from
    // inside an update loses the resize (stale, unpainted frame) and can drop input,
    // so — like GPUI's own `Window::resize` — run them as a separate task.

    fn sync_mini_size(&self, cx: &mut Context<Self>) {
        if let (true, Some(hwnd)) = (self.mini, self.hwnd) {
            let height = self.mini_height();
            cx.spawn(async move |_, _| win::set_content_height(hwnd, height))
                .detach();
        }
    }

    pub fn show_mini(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hwnd) = self.hwnd else {
            self.show(window, cx);
            return;
        };
        if !self.mini {
            self.full_rect = win::window_rect(hwnd);
            self.mini = true;
            self.menu = None;
            self.dialog = None;
        }
        self.hide_generation += 1;
        let search = self.search.clone();
        search.update(cx, |input, cx| input.select_all_text(cx));
        window.focus(&search.focus_handle(cx));
        let height = self.mini_height();
        cx.spawn(async move |_, _| {
            win::place_mini(hwnd, Self::MINI_WIDTH, height);
            win::show_window(hwnd);
        })
        .detach();
        cx.notify();
    }

    /// Leave mini mode and put the full window back where it was.
    pub fn restore_full(&mut self, cx: &mut Context<Self>) {
        if !self.mini {
            return;
        }
        self.mini = false;
        if let (Some(hwnd), Some(rect)) = (self.hwnd, self.full_rect) {
            cx.spawn(async move |_, _| win::set_window_rect(hwnd, rect)).detach();
        }
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Dashboard data

    pub fn recent_items(&self, limit: usize) -> Vec<RecentItem> {
        let mut out = Vec::new();
        for entry in &self.config.history {
            if out.len() >= limit {
                break;
            }
            let Some(target) = Target::from_history_key(&entry.key) else {
                continue;
            };
            let visual = match &target {
                Target::Tool(id) => match self.config.tools.iter().find(|t| &t.id == id) {
                    Some(tool) => Self::tool_visual(tool),
                    None => continue,
                },
                Target::App(path) => {
                    if !path.exists() {
                        continue;
                    }
                    Visual::File {
                        target: path.display().to_string(),
                        tint: "slate".into(),
                        fallback: "app",
                    }
                }
                Target::Action(action) => Visual::Glyph {
                    icon: action.icon(),
                    tint: "pink".into(),
                },
                _ => continue,
            };
            let name = match &target {
                Target::Tool(id) => self
                    .config
                    .tools
                    .iter()
                    .find(|t| &t.id == id)
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| entry.name.clone()),
                _ => entry.name.clone(),
            };
            out.push(RecentItem {
                target,
                key: entry.key.clone(),
                name: name.into(),
                when: crate::config::relative_time(entry.last_used).into(),
                visual,
            });
        }
        out
    }

    // -----------------------------------------------------------------------
    // Activation

    pub fn activate(&mut self, target: Target, admin: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let mut hide = true;
        match &target {
            Target::Tool(id) => {
                let Some(tool) = self.config.tools.iter().find(|t| &t.id == id).cloned() else {
                    return;
                };
                if self.downloads.contains_key(id) {
                    self.show_toast(format!("“{}”正在下载，完成后会自动打开", tool.name), false, cx);
                    return;
                }
                if Self::needs_download(&tool) {
                    self.start_download(&tool.id, true, cx);
                    return;
                }
                let program = if tool.target.trim() == "terminal" {
                    self.terminal()
                } else {
                    tool.target.clone()
                };
                self.record(&target, &tool.name);
                self.spawn_launch(program, tool.args.clone(), tool.workdir.clone(), admin, cx);
            }
            Target::App(path) => {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let name = self
                    .apps
                    .iter()
                    .find(|e| &e.app.path == path)
                    .map(|e| e.app.name.clone())
                    .unwrap_or(name);
                self.record(&target, &name);
                self.spawn_launch(path.display().to_string(), String::new(), String::new(), admin, cx);
            }
            Target::Project(path) => {
                self.open_project_in(path.clone(), ProjectOpener::Explorer, cx);
            }
            Target::Action(action) => {
                self.record(&target, action.label());
                hide = self.run_action(*action, window, cx);
            }
            Target::Url(url) => {
                self.spawn_launch(url.clone(), String::new(), String::new(), false, cx);
            }
            Target::WebSearch(q) => {
                let template = &self.config.settings.search_engine;
                let encoded = url_encode(q);
                let url = if template.contains("{q}") {
                    template.replace("{q}", &encoded)
                } else {
                    format!("{template}{encoded}")
                };
                self.spawn_launch(url, String::new(), String::new(), false, cx);
            }
            Target::File { path, .. } => {
                self.spawn_launch(path.display().to_string(), String::new(), String::new(), admin, cx);
            }
            Target::Command(cmd) => {
                self.spawn_launch("cmd.exe".into(), format!("/k {cmd}"), home_dir(), admin, cx);
            }
        }
        self.config.save();
        if hide && (self.mini || self.config.settings.hide_after_launch) {
            self.hide(window, cx);
        }
        cx.notify();
    }

    fn record(&mut self, target: &Target, name: &str) {
        if let Some(key) = target.history_key() {
            self.config.record_launch(&key, name);
        }
    }

    pub fn spawn_launch(&mut self, target: String, args: String, workdir: String, admin: bool, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move { win::shell_execute(&target, &args, &workdir, admin) });
        cx.spawn(async move |this, cx| {
            if let Err(err) = task.await {
                this.update(cx, |this, cx| this.show_toast(err, true, cx)).ok();
            }
        })
        .detach();
    }

    pub fn terminal(&self) -> String {
        let configured = self.config.settings.terminal.trim();
        if configured.is_empty() {
            win::detect_terminal()
        } else {
            configured.to_string()
        }
    }

    /// Returns whether the window should hide afterwards.
    fn run_action(&mut self, action: QuickAction, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match action {
            QuickAction::NewTerminal => {
                self.spawn_launch(self.terminal(), String::new(), home_dir(), false, cx);
                true
            }
            QuickAction::OpenProject => {
                self.prompt_open_project(window, cx);
                false
            }
            QuickAction::Screenshot => {
                // Hide first so the launcher isn't in the capture.
                self.hide(window, cx);
                self.spawn_launch("ms-screenclip:".into(), String::new(), String::new(), false, cx);
                false
            }
            QuickAction::Note => {
                let path = Config::dir().join("notes.md");
                if !path.exists() {
                    let _ = std::fs::create_dir_all(Config::dir());
                    let _ = std::fs::write(&path, "# LovelyLauncher 笔记\n\n");
                }
                self.spawn_launch("notepad.exe".into(), format!("\"{}\"", path.display()), String::new(), false, cx);
                true
            }
        }
    }

    pub fn prompt_open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("打开项目".into()),
        });
        self.prompting = true;
        let _ = window;
        cx.spawn(async move |this, cx| {
            let picked = rx.await.ok().and_then(|r| r.ok()).flatten();
            this.update(cx, |this, cx| {
                this.prompting = false;
                if let Some(path) = picked.and_then(|p| p.into_iter().next()) {
                    this.open_project_in(path.display().to_string(), ProjectOpener::Explorer, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_project_in(&mut self, path: String, opener: ProjectOpener, cx: &mut Context<Self>) {
        if !Path::new(&path).exists() {
            self.show_toast(format!("文件夹不存在：{path}"), true, cx);
            return;
        }
        self.config.touch_project(Path::new(&path));
        self.config.save();
        match opener {
            ProjectOpener::Explorer => self.spawn_launch(path, String::new(), String::new(), false, cx),
            ProjectOpener::Terminal => {
                let terminal = self.terminal();
                let args = win::terminal_args_for_dir(&terminal, &path);
                self.spawn_launch(terminal, args, path, false, cx)
            }
            ProjectOpener::VsCode => {
                self.spawn_launch("code.cmd".into(), format!("\"{path}\""), path, false, cx)
            }
        }
        cx.notify();
    }

    pub fn reveal(&mut self, target: &str, cx: &mut Context<Self>) {
        let resolved = if target == "terminal" {
            win::resolve_target_path(&self.terminal())
        } else {
            win::resolve_target_path(target)
        };
        match resolved {
            Some(path) => self.spawn_launch(
                "explorer.exe".into(),
                format!("/select,\"{}\"", path.display()),
                String::new(),
                false,
                cx,
            ),
            None => self.show_toast("找不到该文件的位置", true, cx),
        }
    }

    // -----------------------------------------------------------------------
    // Mutations

    pub fn toggle_favorite(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(tool) = self.config.tools.iter_mut().find(|t| t.id == id) {
            tool.favorite = !tool.favorite;
            let msg = if tool.favorite { "已加入收藏" } else { "已取消收藏" };
            self.config.save();
            self.show_toast(msg, false, cx);
        }
    }

    // -----------------------------------------------------------------------
    // Downloads

    pub fn start_download(&mut self, id: &str, launch_after: bool, cx: &mut Context<Self>) {
        let Some(tool) = self.config.tools.iter().find(|t| t.id == id).cloned() else {
            return;
        };
        if tool.download_url.is_empty() || self.downloads.contains_key(id) {
            return;
        }
        let progress = Arc::new(Progress::default());
        self.downloads.insert(tool.id.clone(), progress.clone());
        self.show_toast(format!("开始下载“{}”…", tool.name), false, cx);

        // WinHTTP is blocking, so give it a dedicated thread rather than a pool worker.
        let (tx, rx) = futures::channel::oneshot::channel();
        let (url, dest) = (tool.download_url.clone(), PathBuf::from(win::expand_env(&tool.target)));
        let thread_progress = progress.clone();
        std::thread::Builder::new()
            .name("download".into())
            .spawn(move || {
                let _ = tx.send(download::download(&url, &dest, &thread_progress));
            })
            .ok();

        // Repaint while the download runs so the progress bar moves.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let running = this
                    .update(cx, |this, cx| {
                        cx.notify();
                        this.downloads.values().any(|p| Arc::ptr_eq(p, &progress))
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        })
        .detach();

        let id = tool.id.clone();
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("下载线程意外退出".into()));
            this.update(cx, |this, cx| {
                this.downloads.remove(&id);
                this.invalidate_icon(&tool.target);
                match result {
                    Ok(()) => {
                        this.show_toast(format!("“{}”下载完成", tool.name), false, cx);
                        if launch_after {
                            this.record(&Target::Tool(id.clone()), &tool.name);
                            this.config.save();
                            this.spawn_launch(tool.target.clone(), tool.args.clone(), tool.workdir.clone(), false, cx);
                        }
                    }
                    Err(err) => this.show_toast(format!("“{}”{err}", tool.name), true, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn cancel_download(&mut self, id: &str) {
        if let Some(progress) = self.downloads.get(id) {
            progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Delete a downloaded preset program and fetch it again.
    pub fn redownload(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(tool) = self.config.tools.iter().find(|t| t.id == id) {
            let path = PathBuf::from(win::expand_env(&tool.target));
            if path.exists() && std::fs::remove_file(&path).is_err() {
                self.show_toast("无法替换旧版本，请先关闭正在运行的程序", true, cx);
                return;
            }
        }
        self.start_download(id, false, cx);
    }

    pub fn remove_tool(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(pos) = self.config.tools.iter().position(|t| t.id == id) {
            let tool = self.config.tools.remove(pos);
            // Only delete icons we copied into our own folder.
            if Path::new(&tool.icon_path).starts_with(Config::icons_dir()) {
                let _ = std::fs::remove_file(&tool.icon_path);
            }
            let key = format!("tool:{id}");
            self.config.history.retain(|e| e.key != key);
            self.config.save();
            self.show_toast(format!("已删除“{}”", tool.name), false, cx);
        }
    }

    // -----------------------------------------------------------------------
    // Categories

    /// Tools visible under the current category filter.
    pub fn filtered_tools(&self) -> Vec<Tool> {
        let categories = &self.config.categories;
        self.config
            .tools
            .iter()
            .filter(|t| match &self.tool_filter {
                ToolFilter::All => true,
                ToolFilter::Uncategorized => t.category.is_empty() || !categories.contains(&t.category),
                ToolFilter::Category(c) => &t.category == c,
            })
            .cloned()
            .collect()
    }

    pub fn set_filter(&mut self, filter: ToolFilter, cx: &mut Context<Self>) {
        self.tool_filter = filter;
        self.tools_scroll.set_offset(Default::default());
        cx.notify();
    }

    pub fn set_tool_category(&mut self, id: &str, category: &str, cx: &mut Context<Self>) {
        let Some(tool) = self.config.tools.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if tool.category == category {
            return;
        }
        tool.category = category.to_string();
        let name = tool.name.clone();
        self.config.save();
        let target = if category.is_empty() { "未分类" } else { category };
        self.show_toast(format!("已将“{name}”移到「{target}」"), false, cx);
    }

    pub fn delete_category(&mut self, name: &str, cx: &mut Context<Self>) {
        self.config.delete_category(name);
        if self.tool_filter == ToolFilter::Category(name.to_string()) {
            self.tool_filter = ToolFilter::All;
        }
        self.config.save();
        self.show_toast(format!("已删除分类「{name}」，其中的工具变为未分类"), false, cx);
    }

    pub fn add_project(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.config.touch_project(path);
        self.config.save();
        self.show_toast("已添加到最近项目", false, cx);
    }

    pub fn copy_path(&mut self, path: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.to_string()));
        self.show_toast("已复制路径", false, cx);
    }

    pub fn remove_project(&mut self, path: &str, cx: &mut Context<Self>) {
        self.config.projects.retain(|p| p.path != path);
        self.config.save();
        cx.notify();
    }

    pub fn remove_history(&mut self, key: &str, cx: &mut Context<Self>) {
        self.config.history.retain(|e| e.key != key);
        self.config.save();
        cx.notify();
    }

    pub fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.config.history.clear();
        self.config.save();
        cx.notify();
    }

    pub fn add_app_as_tool(&mut self, path: &Path, cx: &mut Context<Self>) {
        let name = self
            .apps
            .iter()
            .find(|e| e.app.path == path)
            .map(|e| e.app.name.clone())
            .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        self.config.tools.push(Tool {
            name: name.clone(),
            target: path.display().to_string(),
            color: "pink".into(),
            ..Default::default()
        });
        self.config.save();
        self.show_toast(format!("已将“{name}”添加到工具"), false, cx);
    }

    pub fn on_drop_paths(&mut self, paths: &ExternalPaths, _window: &mut Window, cx: &mut Context<Self>) {
        let (mut tools, mut projects) = (0, 0);
        for path in paths.paths() {
            if path.is_dir() {
                self.config.touch_project(path);
                projects += 1;
            } else {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.config.tools.push(Tool {
                    name,
                    target: path.display().to_string(),
                    color: "pink".into(),
                    ..Default::default()
                });
                tools += 1;
            }
        }
        self.config.save();
        let msg = match (tools, projects) {
            (0, p) => format!("已添加 {p} 个项目"),
            (t, 0) => format!("已添加 {t} 个工具"),
            (t, p) => format!("已添加 {t} 个工具和 {p} 个项目"),
        };
        self.show_toast(msg, false, cx);
    }

    pub fn set_autostart(&mut self, enabled: bool, cx: &mut Context<Self>) {
        match win::set_autostart(enabled) {
            Ok(()) => {
                self.config.settings.autostart = enabled;
                self.config.save();
            }
            Err(err) => self.show_toast(format!("设置开机自启失败：{err}"), true, cx),
        }
        cx.notify();
    }

    pub fn apply_hotkey(&mut self, spec: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let spec = spec.trim().to_string();
        cx.update_global::<HotkeyService, _>(|service, _| service.apply(&spec))?;
        self.config.settings.hotkey = spec;
        self.config.save();
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Keyboard actions

    pub fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() || self.results.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.results.len() - 1
        } else {
            self.selected - 1
        };
        self.results_scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    pub fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() || self.results.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.results.len();
        self.results_scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    pub fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            self.confirm_dialog(window, cx);
            return;
        }
        if self.ai_request().is_some() {
            self.start_ai(cx);
            return;
        }
        if let Some(item) = self.results.get(self.selected) {
            let target = item.target.clone();
            self.activate(target, false, window, cx);
        }
    }

    pub fn confirm_admin(&mut self, _: &ConfirmAdmin, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        if let Some(item) = self.results.get(self.selected) {
            let target = item.target.clone();
            self.activate(target, true, window, cx);
        }
    }

    pub fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
            return;
        }
        if self.dialog.is_some() {
            self.close_dialog(window, cx);
            return;
        }
        if self.stop_ai(cx) {
            return;
        }
        if !self.search_text.is_empty() {
            self.search.update(cx, |input, cx| input.set_text("", cx));
            return;
        }
        self.hide(window, cx);
    }

    pub fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        self.search.update(cx, |input, cx| input.select_all_text(cx));
        window.focus(&self.search.focus_handle(cx));
    }

    pub fn quick_action(&mut self, action: QuickAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        self.activate(Target::Action(action), false, window, cx);
    }

    pub fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Keep AI keys / search URL typed in settings even when closed with Esc or ×.
        self.commit_settings_inputs(cx);
        self.dialog = None;
        window.focus(&self.search.focus_handle(cx));
        cx.notify();
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ProjectOpener {
    Explorer,
    Terminal,
    VsCode,
}

impl Focusable for Launcher {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn home_dir() -> String {
    std::env::var("USERPROFILE").unwrap_or_default()
}

fn is_url(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn looks_like_url(s: &str) -> bool {
    if s.contains(char::is_whitespace) || s.starts_with('>') {
        return false;
    }
    if is_url(s) || s.to_ascii_lowercase().starts_with("www.") {
        return true;
    }
    // bare domains like "github.com" or "example.cn/path"
    let host = s.split('/').next().unwrap_or("");
    let mut parts = host.rsplit('.');
    match (parts.next(), parts.next()) {
        (Some(tld), Some(name)) => {
            tld.len() >= 2
                && tld.chars().all(|c| c.is_ascii_alphabetic())
                && !name.is_empty()
                && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':')
                && !matches!(
                    tld.to_ascii_lowercase().as_str(),
                    "exe" | "msc" | "lnk" | "bat" | "cmd" | "ps1" | "txt" | "md" | "json"
                )
        }
        _ => false,
    }
}

fn normalize_url(s: &str) -> String {
    if is_url(s) {
        s.to_string()
    } else {
        format!("https://{s}")
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
