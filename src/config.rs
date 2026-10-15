use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn next(self) -> Self {
        match self {
            ThemeMode::System => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
            ThemeMode::Dark => ThemeMode::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::System => "跟随系统",
            ThemeMode::Light => "浅色",
            ThemeMode::Dark => "深色",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Executable / file / folder path, URL, or shell URI (e.g. `ms-settings:`).
    pub target: String,
    pub args: String,
    pub workdir: String,
    /// Built-in glyph name; empty means "use the target's own icon".
    pub icon: String,
    /// User-uploaded icon image (takes precedence over `icon`).
    pub icon_path: String,
    pub color: String,
    pub favorite: bool,
    /// Picked for the Alt ring (the "圆环" tab).
    pub ring: bool,
    /// When set and `target` doesn't exist yet, the program is downloaded there first.
    pub download_url: String,
    /// User category name; empty = uncategorized.
    pub category: String,
}

impl Default for Tool {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: String::new(),
            description: String::new(),
            target: String::new(),
            args: String::new(),
            workdir: String::new(),
            icon: String::new(),
            icon_path: String::new(),
            color: "blue".into(),
            favorite: false,
            ring: false,
            download_url: String::new(),
            category: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub name: String,
    pub path: String,
    pub last_opened: u64,
}

/// One entry of launch history, keyed by a stable launch key
/// (`tool:<id>`, `app:<path>` or `action:<name>`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryEntry {
    pub key: String,
    pub name: String,
    pub last_used: u64,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeMode,
    pub hotkey: String,
    pub autostart: bool,
    pub close_to_tray: bool,
    pub hide_on_blur: bool,
    pub hide_after_launch: bool,
    pub blur_background: bool,
    /// Terminal used for "新建终端" and "在终端中打开"; empty = auto-detect.
    pub terminal: String,
    /// Accent color as "#RRGGBB".
    pub accent: String,
    /// Tint neutral surfaces with the accent hue (otherwise plain grey).
    pub tint_surfaces: bool,
    /// Window background opacity in percent, used when the blur backdrop is on.
    pub opacity: u8,
    /// Per-token color overrides for power users, e.g. `"window_bg": "#ffeef4"`.
    pub theme_overrides: ThemeOverrides,
    /// The global hotkey opens a compact search bar instead of the full window.
    pub mini_on_hotkey: bool,
    /// Include file results from a running Everything instance.
    pub everything: bool,
    /// Web search URL template; `{q}` is replaced with the encoded query.
    pub search_engine: String,
    /// AI providers for `@ai` quick questions.
    pub ai_providers: Vec<AiProvider>,
    /// Id of the provider `@ai` uses.
    pub ai_active: String,
    /// Look for a new release on GitHub shortly after startup (at most daily).
    pub auto_check_update: bool,
    pub last_update_check: u64,
    /// Press Space in Explorer / on the desktop to preview the selected file.
    pub space_preview: bool,
    /// Wrap long lines in text previews (otherwise scroll sideways).
    pub preview_wrap: bool,
    /// Hold Alt to pop up a ring of favourite tools in the bottom-right corner.
    pub alt_ring: bool,
    /// …except while a full-screen program (a game, a video) is in front.
    pub ring_skip_fullscreen: bool,
    /// …or while one of these programs (exe file names) is in front.
    pub ring_excluded: Vec<String>,
    /// User-chosen ffmpeg.exe; empty = our downloaded copy or one on PATH.
    pub ffmpeg_path: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiApi {
    /// `POST {base}/chat/completions` — OpenAI and compatible services.
    #[default]
    OpenAi,
    /// `POST {base}/messages` — Anthropic's Messages API.
    Anthropic,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AiProvider {
    /// Short id, also usable as a prefix: `@deepseek 问题`.
    pub id: String,
    pub name: String,
    pub api: AiApi,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Model ids last fetched from the provider.
    pub models: Vec<String>,
}

fn ai_provider(id: &str, name: &str, api: AiApi, base_url: &str, model: &str) -> AiProvider {
    AiProvider {
        id: id.into(),
        name: name.into(),
        api,
        base_url: base_url.into(),
        model: model.into(),
        ..Default::default()
    }
}

pub fn default_ai_providers() -> Vec<AiProvider> {
    vec![
        ai_provider("openai", "OpenAI", AiApi::OpenAi, "https://api.openai.com/v1", ""),
        ai_provider("deepseek", "DeepSeek", AiApi::OpenAi, "https://api.deepseek.com/v1", "deepseek-chat"),
        ai_provider("claude", "Claude", AiApi::Anthropic, "https://api.anthropic.com/v1", "claude-opus-5-5"),
        ai_provider("kimi", "Kimi", AiApi::OpenAi, "https://api.moonshot.cn/v1", ""),
        ai_provider("glm", "智谱 GLM", AiApi::OpenAi, "https://open.bigmodel.cn/api/paas/v4", ""),
    ]
}

/// Web search presets: (name, URL template).
pub const SEARCH_ENGINES: &[(&str, &str)] = &[
    ("必应", "https://www.bing.com/search?q={q}"),
    ("百度", "https://www.baidu.com/s?wd={q}"),
    ("Google", "https://www.google.com/search?q={q}"),
    ("DuckDuckGo", "https://duckduckgo.com/?q={q}"),
    ("搜狗", "https://www.sogou.com/web?query={q}"),
];

impl Settings {
    /// Display name of the configured search engine.
    pub fn search_engine_name(&self) -> &str {
        SEARCH_ENGINES
            .iter()
            .find(|(_, url)| *url == self.search_engine)
            .map(|(name, _)| *name)
            .unwrap_or("自定义搜索")
    }

    pub fn active_ai(&self) -> Option<&AiProvider> {
        self.ai_providers
            .iter()
            .find(|p| p.id == self.ai_active)
            .or_else(|| self.ai_providers.first())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeOverrides {
    pub light: BTreeMap<String, String>,
    pub dark: BTreeMap<String, String>,
}

pub const DEFAULT_ACCENT: &str = "#EE6585";

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            hotkey: "Alt+Space".into(),
            autostart: false,
            close_to_tray: true,
            hide_on_blur: false,
            hide_after_launch: true,
            blur_background: true,
            terminal: String::new(),
            accent: DEFAULT_ACCENT.into(),
            tint_surfaces: true,
            opacity: 88,
            theme_overrides: ThemeOverrides::default(),
            mini_on_hotkey: true,
            everything: true,
            search_engine: SEARCH_ENGINES[0].1.into(),
            ai_providers: default_ai_providers(),
            ai_active: "deepseek".into(),
            auto_check_update: true,
            last_update_check: 0,
            space_preview: true,
            preview_wrap: true,
            alt_ring: true,
            ring_skip_fullscreen: true,
            ring_excluded: Vec::new(),
            ffmpeg_path: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Schema version; older configs are migrated on load. The field-level default
    /// makes a missing field read as 0 rather than the current version.
    #[serde(default)]
    pub version: u32,
    pub settings: Settings,
    pub tools: Vec<Tool>,
    /// User-defined tool categories, in display order. Field-level default so a
    /// missing field reads as empty (not the defaults), letting migration fill it.
    #[serde(default)]
    pub categories: Vec<String>,
    pub projects: Vec<Project>,
    pub history: Vec<HistoryEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            settings: Settings::default(),
            tools: default_tools(),
            categories: default_categories(),
            projects: Vec::new(),
            history: Vec::new(),
        }
    }
}

impl Config {
    /// Data directory: `LOVELYLAUNCHER_DATA` if set (portable / demo setups),
    /// otherwise `%APPDATA%\LovelyLauncher`.
    pub fn dir() -> PathBuf {
        if let Some(dir) = std::env::var_os("LOVELYLAUNCHER_DATA").filter(|d| !d.is_empty()) {
            return PathBuf::from(dir);
        }
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("LovelyLauncher")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.json")
    }

    pub fn icons_dir() -> PathBuf {
        Self::dir().join("icons")
    }

    pub fn load() -> Self {
        let mut config = match fs::read_to_string(Self::path()) {
            // Editors such as Notepad may save with a UTF-8 BOM, which serde_json rejects.
            Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_else(|err| {
                eprintln!("config.json is invalid ({err}), backing it up and using defaults");
                let _ = fs::rename(Self::path(), Self::dir().join("config.invalid.json"));
                Self::default()
            }),
            Err(_) => Self::default(),
        };
        if config.version < CONFIG_VERSION {
            config.migrate();
            config.save();
        }
        config
    }

    fn migrate(&mut self) {
        if self.version < 2 {
            // v1 shipped built-in glyphs on the default tools; v2 prefers the programs' own icons.
            for tool in &mut self.tools {
                if V1_DEFAULT_GLYPHS
                    .iter()
                    .any(|(target, icon)| tool.target == *target && tool.icon == *icon)
                {
                    tool.icon.clear();
                }
            }
            for preset in download_presets().into_iter().rev() {
                if !self.tools.iter().any(|t| t.download_url == preset.download_url) {
                    self.tools.insert(0, preset);
                }
            }
        }
        // v3 shipped with a bug that skipped this step, hence `< 4`.
        if self.version < 4 {
            if self.categories.is_empty() {
                self.categories = default_categories();
            }
            for tool in &mut self.tools {
                if tool.category.is_empty() {
                    tool.category = default_category_for(tool).to_string();
                }
            }
        }
        if self.version < 6 {
            // v5: the publisher became LovelyTeams; v6: their tools' category is LovelyTools.
            const OLD_BRAND: &str = "LoveTools";
            self.merge_category(OLD_BRAND, TOOLS_CATEGORY);
            self.merge_category(BRAND, TOOLS_CATEGORY);
            for tool in &mut self.tools {
                if tool.description.starts_with(OLD_BRAND) {
                    tool.description = tool.description.replacen(OLD_BRAND, BRAND, 1);
                }
            }
        }
        self.version = CONFIG_VERSION;
    }

    pub fn save(&self) {
        let dir = Self::dir();
        let _ = fs::create_dir_all(&dir);
        let Ok(json) = serde_json::to_string_pretty(self) else {
            return;
        };
        // Write-then-rename so a crash never leaves a truncated config behind.
        let tmp = dir.join("config.json.tmp");
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, Self::path());
        }
    }

    pub fn record_launch(&mut self, key: &str, name: &str) {
        let now = now_secs();
        if let Some(entry) = self.history.iter_mut().find(|e| e.key == key) {
            entry.last_used = now;
            entry.count += 1;
            entry.name = name.to_string();
        } else {
            self.history.push(HistoryEntry {
                key: key.to_string(),
                name: name.to_string(),
                last_used: now,
                count: 1,
            });
        }
        self.history.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        self.history.truncate(200);
    }

    pub fn touch_project(&mut self, path: &Path) {
        let path_str = path.display().to_string();
        let now = now_secs();
        if let Some(p) = self
            .projects
            .iter_mut()
            .find(|p| p.path.eq_ignore_ascii_case(&path_str))
        {
            p.last_opened = now;
        } else {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path_str.clone());
            self.projects.push(Project {
                name,
                path: path_str,
                last_opened: now,
            });
        }
        self.projects
            .sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
        self.projects.truncate(50);
    }

    /// Add a category (trimmed); returns false if empty or it already exists.
    pub fn add_category(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || self.categories.iter().any(|c| c == name) {
            return false;
        }
        self.categories.push(name.to_string());
        true
    }

    pub fn rename_category(&mut self, old: &str, new: &str) -> bool {
        let new = new.trim();
        if new.is_empty() || (new != old && self.categories.iter().any(|c| c == new)) {
            return false;
        }
        for c in &mut self.categories {
            if c == old {
                *c = new.to_string();
            }
        }
        for tool in &mut self.tools {
            if tool.category == old {
                tool.category = new.to_string();
            }
        }
        true
    }

    /// Move every tool in `from` to `to` and drop `from` (renaming it in place
    /// when `to` doesn't exist yet, so its position in the tab bar is kept).
    fn merge_category(&mut self, from: &str, to: &str) {
        if !self.categories.iter().any(|c| c == from) {
            return;
        }
        if self.categories.iter().any(|c| c == to) {
            for tool in &mut self.tools {
                if tool.category == from {
                    tool.category = to.into();
                }
            }
            self.categories.retain(|c| c != from);
        } else {
            self.rename_category(from, to);
        }
    }

    /// Remove a category; its tools become uncategorized.
    pub fn delete_category(&mut self, name: &str) {
        self.categories.retain(|c| c != name);
        for tool in &mut self.tools {
            if tool.category == name {
                tool.category.clear();
            }
        }
    }

    pub fn usage(&self, key: &str) -> Option<&HistoryEntry> {
        self.history.iter().find(|e| e.key == key)
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{:x}{:04x}",
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff
    )
}

/// Human-friendly relative time, e.g. "2 分钟前".
pub fn relative_time(ts: u64) -> String {
    let diff = now_secs().saturating_sub(ts);
    match diff {
        0..=59 => "刚刚".into(),
        60..=3599 => format!("{} 分钟前", diff / 60),
        3600..=86_399 => format!("{} 小时前", diff / 3600),
        86_400..=2_591_999 => format!("{} 天前", diff / 86_400),
        _ => format!("{} 个月前", diff / 2_592_000),
    }
}

fn tool(name: &str, desc: &str, target: &str, icon: &str, color: &str, favorite: bool) -> Tool {
    Tool {
        name: name.into(),
        description: desc.into(),
        target: target.into(),
        icon: icon.into(),
        color: color.into(),
        favorite,
        ..Default::default()
    }
}

pub const CONFIG_VERSION: u32 = 6;

/// Publisher of the downloadable preset programs.
pub const BRAND: &str = "LovelyTeams";
/// Default category of the publisher's tools.
pub const TOOLS_CATEGORY: &str = "LovelyTools";

fn default_categories() -> Vec<String> {
    vec![TOOLS_CATEGORY.into(), "系统工具".into(), "常用".into()]
}

/// Category the built-in defaults belong to (empty for anything else).
fn default_category_for(tool: &Tool) -> &'static str {
    if !tool.download_url.is_empty() {
        return TOOLS_CATEGORY;
    }
    match tool.target.to_ascii_lowercase().as_str() {
        "resmon.exe" | "taskmgr.exe" | "eventvwr.msc" | "mstsc.exe" | "regedit.exe"
        | "ms-settings:" | "explorer.exe" => "系统工具",
        "terminal" | "notepad.exe" | "calc.exe" => "常用",
        _ => "",
    }
}

/// (target, glyph) pairs of the v1 default tools, cleared by the v2 migration.
const V1_DEFAULT_GLYPHS: &[(&str, &str)] = &[
    ("resmon.exe", "activity"),
    ("taskmgr.exe", "cpu"),
    ("eventvwr.msc", "file-text"),
    ("mstsc.exe", "monitor"),
    ("explorer.exe", "folder"),
    ("terminal", "terminal"),
    ("notepad.exe", "edit"),
    ("calc.exe", "calculator"),
    ("regedit.exe", "database"),
];

fn categorized(mut tool: Tool) -> Tool {
    tool.category = default_category_for(&tool).to_string();
    tool
}

fn preset(name: &str, url: &str, color: &str) -> Tool {
    let file = url.rsplit('/').next().unwrap_or("app.exe");
    Tool {
        name: name.into(),
        description: format!("{BRAND} 出品 · 首次打开自动下载"),
        target: format!(r"%LOCALAPPDATA%\LovelyLauncher\apps\{file}"),
        color: color.into(),
        favorite: true,
        download_url: url.into(),
        ..Default::default()
    }
}

/// LovelyTeams programs that are downloaded on first launch.
pub fn download_presets() -> Vec<Tool> {
    vec![
        preset(
            "LovelyMem",
            "https://lovetools-vip-download.tos-cn-beijing.volces.com/lovelymem.v3.exe",
            "purple",
        ),
        preset(
            "LovelySpark",
            "https://lovetools-vip-download.tos-cn-beijing.volces.com/lovelyspark.exe",
            "pink",
        ),
    ]
}

fn default_tools() -> Vec<Tool> {
    let mut tools = download_presets();
    tools.iter_mut().for_each(|t| t.category = TOOLS_CATEGORY.into());
    tools.extend([
        tool("资源监视器", "实时监控网络、磁盘与内存", "resmon.exe", "", "blue", true),
        tool("任务管理器", "查看进程与资源占用", "taskmgr.exe", "", "purple", true),
        tool("事件查看器", "浏览系统与应用程序日志", "eventvwr.msc", "", "green", false),
        tool("远程桌面", "安全地连接远程主机", "mstsc.exe", "", "sky", false),
        tool("文件资源管理器", "轻松管理你的文件", "explorer.exe", "", "orange", false),
        tool("终端", "快速运行命令", "terminal", "", "slate", false),
        tool("记事本", "随手记录文字", "notepad.exe", "", "teal", false),
        tool("计算器", "简单快速的计算", "calc.exe", "", "pink", false),
        tool("注册表编辑器", "查看和修改注册表", "regedit.exe", "", "sky", false),
        tool("系统设置", "调整 Windows 设置", "ms-settings:", "settings", "slate", false),
    ]);
    tools.into_iter().map(categorized).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_v2_categories() {
        let json = r#"{"version":2,"tools":[{"id":"a","name":"x","target":"resmon.exe"},{"id":"b","name":"y","target":"notepad.exe"}]}"#;
        let mut config: Config = serde_json::from_str(json).unwrap();
        assert!(config.categories.is_empty(), "{:?}", config.categories);
        config.migrate();
        assert_eq!(config.categories, default_categories());
        assert_eq!(config.tools[0].category, "系统工具");
        assert_eq!(config.tools[1].category, "常用");
    }
}

#[cfg(test)]
mod v3_tests {
    use super::*;

    #[test]
    fn repairs_v3_without_tool_categories() {
        let json = r#"{"version":3,"categories":["LoveTools","系统工具","常用"],"tools":[{"id":"a","name":"x","target":"calc.exe"},{"id":"b","name":"y","target":"x.exe","category":"LoveTools","description":"LoveTools 出品"}]}"#;
        let mut config: Config = serde_json::from_str(json).unwrap();
        config.migrate();
        assert_eq!(config.tools[0].category, "常用");
        assert_eq!(config.categories, vec!["LovelyTools", "系统工具", "常用"]);
        assert_eq!(config.tools[1].category, "LovelyTools");
        assert_eq!(config.tools[1].description, "LovelyTeams 出品");
    }
}

#[cfg(test)]
mod v6_tests {
    use super::*;

    #[test]
    fn lovelyteams_category_becomes_lovelytools() {
        let json = r#"{"version":5,"categories":["LovelyTeams","常用"],"tools":[{"id":"a","name":"x","target":"x.exe","category":"LovelyTeams","description":"LovelyTeams 出品"}]}"#;
        let mut config: Config = serde_json::from_str(json).unwrap();
        config.migrate();
        assert_eq!(config.categories, vec!["LovelyTools", "常用"]);
        assert_eq!(config.tools[0].category, "LovelyTools");
        assert_eq!(config.tools[0].description, "LovelyTeams 出品");
    }
}
