//! Modal dialogs: tool editor and settings.

use gpui::{
    AnyElement, BoxShadow, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, FontWeight,
    PathPromptOptions, SharedString, Stateful, Subscription, Window, div, point, prelude::*, px,
};

use crate::{
    config::{Config, ThemeMode, Tool},
    icons::PICKER_ICONS,
    config::DEFAULT_ACCENT,
    input::{InputEvent, TextInput},
    launcher::{FocusNextField, FocusPrevField, Launcher, ToolFilter},
    theme::{ACCENT_PRESETS, TINTS, Theme, parse_hex},
    icon_image,
    view::{icon, kbd, soft_shadow},
    win,
};

pub enum Dialog {
    Tool(ToolEditor),
    Settings(SettingsEditor),
    Category(CategoryEditor),
}

pub struct CategoryEditor {
    /// The category being renamed; `None` when creating one.
    original: Option<String>,
    name: Entity<TextInput>,
    error: Option<SharedString>,
    focus: FocusHandle,
}

pub struct ToolEditor {
    id: Option<String>,
    name: Entity<TextInput>,
    target: Entity<TextInput>,
    args: Entity<TextInput>,
    workdir: Entity<TextInput>,
    description: Entity<TextInput>,
    download_url: Entity<TextInput>,
    category: Entity<TextInput>,
    icon: String,
    icon_path: String,
    color: String,
    favorite: bool,
    error: Option<SharedString>,
    focus: FocusHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Appearance,
    Preview,
    Ai,
    Search,
}

impl SettingsTab {
    const ALL: [SettingsTab; 5] = [
        SettingsTab::General,
        SettingsTab::Appearance,
        SettingsTab::Preview,
        SettingsTab::Ai,
        SettingsTab::Search,
    ];

    fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "通用",
            SettingsTab::Appearance => "外观",
            SettingsTab::Preview => "预览",
            SettingsTab::Ai => "AI 助手",
            SettingsTab::Search => "搜索",
        }
    }
}

pub struct SettingsEditor {
    tab: SettingsTab,
    /// Provider whose fields are shown in the AI tab.
    ai_selected: String,
    ai_base_url: Entity<TextInput>,
    ai_key: Entity<TextInput>,
    ai_model: Entity<TextInput>,
    ai_status: Option<(SharedString, bool)>,
    ai_fetching: bool,
    search_url: Entity<TextInput>,
    hotkey: Entity<TextInput>,
    terminal: Entity<TextInput>,
    accent: Entity<TextInput>,
    _accent_subscription: Subscription,
    status: Option<(SharedString, bool)>,
    detected_terminal: String,
    focus: FocusHandle,
}

impl Dialog {
    fn inputs(&self) -> Vec<Entity<TextInput>> {
        match self {
            Dialog::Tool(e) => vec![
                e.name.clone(),
                e.target.clone(),
                e.args.clone(),
                e.workdir.clone(),
                e.description.clone(),
                e.category.clone(),
                e.download_url.clone(),
            ],
            Dialog::Settings(s) => match s.tab {
                SettingsTab::General => vec![s.hotkey.clone(), s.terminal.clone()],
                SettingsTab::Appearance => vec![s.accent.clone()],
                SettingsTab::Preview => Vec::new(),
                SettingsTab::Ai => vec![s.ai_base_url.clone(), s.ai_key.clone(), s.ai_model.clone()],
                SettingsTab::Search => vec![s.search_url.clone()],
            },
            Dialog::Category(c) => vec![c.name.clone()],
        }
    }

    fn focus_handle(&self) -> &FocusHandle {
        match self {
            Dialog::Tool(e) => &e.focus,
            Dialog::Settings(s) => &s.focus,
            Dialog::Category(c) => &c.focus,
        }
    }
}

fn new_input(cx: &mut Context<Launcher>, placeholder: &str, text: &str) -> Entity<TextInput> {
    let placeholder = placeholder.to_string();
    let text = text.to_string();
    cx.new(|cx| {
        let mut input = TextInput::new(placeholder, cx);
        if !text.is_empty() {
            input.set_text(text, cx);
        }
        input
    })
}

fn button(id: &'static str, label: &'static str, primary: bool, theme: &Theme) -> Stateful<Div> {
    let base = div()
        .id(id)
        .h(px(34.))
        .px(px(16.))
        .rounded(px(9.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.5))
        .cursor_pointer()
        .child(label);
    if primary {
        let hover = theme.accent.opacity(0.88);
        base.bg(theme.accent)
            .text_color(theme.accent_text)
            .font_weight(FontWeight::SEMIBOLD)
            .hover(move |s| s.bg(hover))
    } else {
        base.border_1()
            .border_color(theme.border_strong)
            .text_color(theme.text)
            .hover(|s| s.bg(theme.row_hover))
    }
}

fn switch(id: impl Into<gpui::ElementId>, on: bool, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(38.))
        .h(px(22.))
        .flex_none()
        .rounded_full()
        .p(px(3.))
        .flex()
        .when(on, |d| d.justify_end())
        .bg(if on { theme.accent } else { theme.kbd_border })
        .cursor_pointer()
        .child(
            div()
                .size(px(16.))
                .rounded_full()
                .bg(gpui::white())
                .shadow(vec![BoxShadow {
                    color: gpui::hsla(0., 0., 0., 0.2),
                    offset: point(px(0.), px(1.)),
                    blur_radius: px(2.),
                    spread_radius: px(0.),
                }]),
        )
}

fn label(text: &'static str, theme: &Theme) -> Div {
    div()
        .text_size(px(12.5))
        .text_color(theme.text_muted)
        .mb(px(6.))
        .child(text)
}

impl Launcher {
    fn input_box(&self, input: &Entity<TextInput>, theme: &Theme, window: &Window, cx: &Context<Self>) -> Div {
        let focused = input.focus_handle(cx).is_focused(window);
        div()
            .h(px(36.))
            .px(px(11.))
            .flex()
            .items_center()
            .bg(theme.input_bg)
            .rounded(px(9.))
            .border_1()
            .border_color(if focused { theme.accent } else { theme.border_strong })
            .text_size(px(13.5))
            .line_height(px(20.))
            .child(input.clone())
    }

    // -----------------------------------------------------------------------
    // Opening / closing

    pub fn open_tool_editor(&mut self, id: Option<String>, prefill: Option<Tool>, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        // Dialogs need the full window.
        self.restore_full(cx);
        let tool = id
            .as_ref()
            .and_then(|id| self.config.tools.iter().find(|t| &t.id == id).cloned())
            .or(prefill)
            .unwrap_or_else(|| Tool {
                color: "pink".into(),
                ..Default::default()
            });
        let editor = ToolEditor {
            id: id.filter(|id| self.config.tools.iter().any(|t| &t.id == id)),
            name: new_input(cx, "例如：任务管理器", &tool.name),
            target: new_input(cx, "程序路径、网址、文件夹，或 ms-settings: 之类的协议", &tool.target),
            args: new_input(cx, "可选", &tool.args),
            workdir: new_input(cx, "可选", &tool.workdir),
            description: new_input(cx, "一句话介绍这个工具（可选）", &tool.description),
            download_url: new_input(cx, "可选：目标文件不存在时，先从这里下载", &tool.download_url),
            category: new_input(cx, "未分类（输入新名称会自动创建分类）", &tool.category),
            icon: tool.icon.clone(),
            icon_path: tool.icon_path.clone(),
            color: tool.color.clone(),
            favorite: tool.favorite,
            error: None,
            focus: cx.focus_handle(),
        };
        let first = editor.name.clone();
        self.dialog = Some(Dialog::Tool(editor));
        window.focus(&first.focus_handle(cx));
        cx.notify();
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        // Dialogs need the full window.
        self.restore_full(cx);
        let settings = self.config.settings.clone();
        let accent = new_input(cx, DEFAULT_ACCENT, &settings.accent);
        // Apply the accent live as soon as the text is a valid color.
        let accent_subscription = cx.subscribe(&accent, |this, input, _: &InputEvent, cx| {
            let text = input.read(cx).committed_text();
            if parse_hex(&text).is_some() {
                let hex = format!("#{}", text.trim().trim_start_matches('#').to_uppercase());
                if hex != this.config.settings.accent {
                    this.config.settings.accent = hex;
                    this.config.save();
                    this.refresh_theme(cx);
                }
            }
        });
        let ai = settings.active_ai().cloned().unwrap_or_default();
        let ai_key = cx.new(|cx| {
            let mut input = TextInput::new("sk-…", cx).masked();
            if !ai.api_key.is_empty() {
                input.set_text(ai.api_key.clone(), cx);
            }
            input
        });
        let editor = SettingsEditor {
            tab: SettingsTab::General,
            ai_selected: ai.id.clone(),
            ai_base_url: new_input(cx, "https://…/v1", &ai.base_url),
            ai_key,
            ai_model: new_input(cx, "模型名称，或点“拉取模型”选择", &ai.model),
            ai_status: None,
            ai_fetching: false,
            search_url: new_input(cx, "https://example.com/search?q={q}", &settings.search_engine),
            hotkey: new_input(cx, "Alt+Space", &settings.hotkey),
            terminal: new_input(cx, "留空则自动检测", &settings.terminal),
            accent,
            _accent_subscription: accent_subscription,
            status: None,
            detected_terminal: win::detect_terminal(),
            focus: cx.focus_handle(),
        };
        let focus = editor.focus.clone();
        self.dialog = Some(Dialog::Settings(editor));
        window.focus(&focus);
        cx.notify();
    }

    pub fn open_category_editor(&mut self, original: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        // Dialogs need the full window.
        self.restore_full(cx);
        let name = new_input(cx, "例如：开发、设计、游戏", original.as_deref().unwrap_or(""));
        name.update(cx, |input, cx| input.select_all_text(cx));
        let focus = name.focus_handle(cx);
        self.dialog = Some(Dialog::Category(CategoryEditor {
            original,
            name,
            error: None,
            focus: cx.focus_handle(),
        }));
        window.focus(&focus);
        cx.notify();
    }

    fn save_category(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::Category(editor)) = self.dialog.as_mut() else {
            return;
        };
        let name = editor.name.read(cx).text().trim().to_string();
        let original = editor.original.clone();
        let ok = match &original {
            Some(old) => self.config.rename_category(old, &name),
            None => self.config.add_category(&name),
        };
        if !ok {
            if let Some(Dialog::Category(editor)) = self.dialog.as_mut() {
                editor.error = Some(if name.is_empty() {
                    "分类名称不能为空".into()
                } else {
                    format!("已经有名为「{name}」的分类了").into()
                });
            }
            cx.notify();
            return;
        }
        if let Some(old) = &original {
            if self.tool_filter == ToolFilter::Category(old.clone()) {
                self.tool_filter = ToolFilter::Category(name.clone());
            }
        } else {
            self.tool_filter = ToolFilter::Category(name.clone());
        }
        self.config.save();
        self.close_dialog(window, cx);
    }

    pub fn confirm_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.dialog.as_ref() {
            Some(Dialog::Tool(_)) => self.save_tool(window, cx),
            Some(Dialog::Settings(_)) => self.save_settings(window, cx),
            Some(Dialog::Category(_)) => self.save_category(window, cx),
            None => {}
        }
    }

    fn save_tool(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::Tool(editor)) = self.dialog.as_mut() else {
            return;
        };
        let read = |e: &Entity<TextInput>, cx: &Context<Self>| e.read(cx).text().trim().to_string();
        let name = read(&editor.name, cx);
        let target = read(&editor.target, cx);
        if name.is_empty() || target.is_empty() {
            editor.error = Some("名称和目标都不能为空".into());
            cx.notify();
            return;
        }
        let tool = Tool {
            id: editor.id.clone().unwrap_or_else(crate::config::new_id),
            name: name.clone(),
            description: read(&editor.description, cx),
            target,
            args: read(&editor.args, cx),
            workdir: read(&editor.workdir, cx),
            icon: editor.icon.clone(),
            icon_path: editor.icon_path.clone(),
            color: editor.color.clone(),
            favorite: editor.favorite,
            download_url: read(&editor.download_url, cx),
            category: read(&editor.category, cx),
        };
        if !tool.category.is_empty() {
            self.config.add_category(&tool.category);
        }
        let is_new = editor.id.is_none();
        match self.config.tools.iter_mut().find(|t| t.id == tool.id) {
            Some(existing) => *existing = tool,
            None => self.config.tools.push(tool),
        }
        self.config.save();
        self.close_dialog(window, cx);
        self.refresh_results();
        self.show_toast(
            if is_new {
                format!("已添加“{name}”")
            } else {
                format!("已保存“{name}”")
            },
            false,
            cx,
        );
    }

    /// Copy the AI tab's inputs back into the selected provider's config.
    pub(crate) fn commit_settings_inputs(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Settings(editor)) = self.dialog.as_ref() else {
            return;
        };
        let read = |e: &Entity<TextInput>| e.read(cx).text().trim().to_string();
        let (base_url, key, model) = (read(&editor.ai_base_url), read(&editor.ai_key), read(&editor.ai_model));
        let search = read(&editor.search_url);
        let selected = editor.ai_selected.clone();
        if let Some(p) = self.config.settings.ai_providers.iter_mut().find(|p| p.id == selected) {
            p.base_url = base_url;
            p.api_key = key;
            p.model = model;
        }
        if search.starts_with("http://") || search.starts_with("https://") {
            self.config.settings.search_engine = search;
        }
        self.config.save();
    }

    pub fn open_ai_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(window, cx);
        let active = self.ai_request().map(|(p, _)| p.id);
        if let Some(Dialog::Settings(editor)) = self.dialog.as_mut() {
            editor.tab = SettingsTab::Ai;
        }
        // Show the provider the user was just asking.
        if let Some(id) = active {
            self.select_ai_provider(id, cx);
        }
    }

    fn select_ai_provider(&mut self, id: String, cx: &mut Context<Self>) {
        self.commit_settings_inputs(cx);
        let Some(provider) = self.config.settings.ai_providers.iter().find(|p| p.id == id).cloned() else {
            return;
        };
        if let Some(Dialog::Settings(editor)) = self.dialog.as_mut() {
            editor.ai_selected = id;
            editor.ai_status = None;
            let (b, k, m) = (editor.ai_base_url.clone(), editor.ai_key.clone(), editor.ai_model.clone());
            b.update(cx, |i, cx| i.set_text(provider.base_url.clone(), cx));
            k.update(cx, |i, cx| i.set_text(provider.api_key.clone(), cx));
            m.update(cx, |i, cx| i.set_text(provider.model.clone(), cx));
        }
        cx.notify();
    }

    fn fetch_ai_models(&mut self, cx: &mut Context<Self>) {
        self.commit_settings_inputs(cx);
        let Some(Dialog::Settings(editor)) = self.dialog.as_mut() else {
            return;
        };
        let selected = editor.ai_selected.clone();
        let Some(provider) = self.config.settings.ai_providers.iter().find(|p| p.id == selected).cloned() else {
            return;
        };
        editor.ai_fetching = true;
        editor.ai_status = Some(("正在拉取模型列表…".into(), false));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { crate::ai::list_models(&provider) }).await;
            this.update(cx, |this, cx| {
                let status = match result {
                    Ok(models) => {
                        let n = models.len();
                        if let Some(p) = this.config.settings.ai_providers.iter_mut().find(|p| p.id == selected) {
                            p.models = models;
                        }
                        this.config.save();
                        (format!("已拉取 {n} 个模型，点击即可选用").into(), false)
                    }
                    Err(err) => (err.into(), true),
                };
                if let Some(Dialog::Settings(editor)) = this.dialog.as_mut() {
                    editor.ai_fetching = false;
                    editor.ai_status = Some(status);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_settings_inputs(cx);
        let Some(Dialog::Settings(editor)) = self.dialog.as_ref() else {
            return;
        };
        let hotkey = editor.hotkey.read(cx).text().trim().to_string();
        let terminal = editor.terminal.read(cx).text().trim().to_string();
        if hotkey != self.config.settings.hotkey {
            if let Err(err) = self.apply_hotkey(&hotkey, cx) {
                if let Some(Dialog::Settings(editor)) = self.dialog.as_mut() {
                    editor.status = Some((err.into(), true));
                }
                cx.notify();
                return;
            }
        }
        self.config.settings.terminal = terminal;
        self.config.save();
        self.close_dialog(window, cx);
    }

    fn upload_icon(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择图标图片（PNG / JPG / ICO），或一个程序".into()),
        });
        self.prompting = true;
        cx.spawn(async move |this, cx| {
            let picked = rx.await.ok().and_then(|r| r.ok()).flatten();
            let source = picked.and_then(|p| p.into_iter().next());
            let imported = match source {
                Some(source) => Some(cx.background_spawn(async move { icon_image::import(&source) }).await),
                None => None,
            };
            this.update(cx, |this, cx| {
                this.prompting = false;
                match imported {
                    Some(Ok(path)) => {
                        if let Some(Dialog::Tool(editor)) = this.dialog.as_mut() {
                            editor.icon_path = path.display().to_string();
                            editor.icon.clear();
                        }
                    }
                    Some(Err(err)) => this.show_toast(err, true, cx),
                    None => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn browse_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择程序或文件".into()),
        });
        self.prompting = true;
        let _ = window;
        cx.spawn(async move |this, cx| {
            let picked = rx.await.ok().and_then(|r| r.ok()).flatten();
            this.update(cx, |this, cx| {
                this.prompting = false;
                let Some(path) = picked.and_then(|p| p.into_iter().next()) else {
                    return;
                };
                if let Some(Dialog::Tool(editor)) = this.dialog.as_ref() {
                    let (target, name) = (editor.target.clone(), editor.name.clone());
                    target.update(cx, |input, cx| input.set_text(path.display().to_string(), cx));
                    if name.read(cx).text().trim().is_empty() {
                        let stem = path
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        name.update(cx, |input, cx| input.set_text(stem, cx));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn focus_next_field(&mut self, _: &FocusNextField, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_focus(1, window, cx);
    }

    pub fn focus_prev_field(&mut self, _: &FocusPrevField, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_focus(-1, window, cx);
    }

    fn cycle_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        let inputs = dialog.inputs();
        let current = inputs
            .iter()
            .position(|i| i.focus_handle(cx).is_focused(window));
        let next = match current {
            Some(ix) => (ix as isize + delta).rem_euclid(inputs.len() as isize) as usize,
            None => 0,
        };
        window.focus(&inputs[next].focus_handle(cx));
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Rendering

    pub(crate) fn render_dialog(&mut self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let preview = match self.dialog.as_ref()? {
            Dialog::Tool(editor) => {
                let draft = Tool {
                    target: editor.target.read(cx).text().trim().to_string(),
                    icon: editor.icon.clone(),
                    icon_path: editor.icon_path.clone(),
                    color: editor.color.clone(),
                    ..Default::default()
                };
                let visual = Self::tool_visual(&draft);
                Some(self.tile(&visual, 56., 28., theme, cx))
            }
            Dialog::Settings(_) | Dialog::Category(_) => None,
        };
        let dialog = self.dialog.as_ref()?;
        let focus = dialog.focus_handle().clone();
        let (title, body, footer) = match dialog {
            Dialog::Tool(editor) => {
                let title = if editor.id.is_some() { "编辑工具" } else { "添加工具" };
                let form = self.render_tool_form(editor, preview, theme, window, cx);
                (title, form, self.render_tool_footer(theme, cx))
            }
            Dialog::Settings(editor) => (
                "设置",
                self.render_settings_form(editor, theme, window, cx),
                self.render_settings_footer(theme, cx),
            ),
            Dialog::Category(editor) => (
                if editor.original.is_some() { "重命名分类" } else { "新建分类" },
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(label("分类名称", theme))
                    .child(self.input_box(&editor.name, theme, window, cx))
                    .when_some(editor.error.clone(), |d, err| {
                        d.child(div().text_size(px(12.5)).text_color(theme.danger).child(err))
                    })
                    .into_any_element(),
                div()
                    .flex_none()
                    .px(px(22.))
                    .py(px(16.))
                    .flex()
                    .justify_end()
                    .gap(px(10.))
                    .child(button("category-cancel", "取消", false, theme).on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| this.close_dialog(window, cx),
                    )))
                    .child(button("category-save", "保存", true, theme).on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| this.save_category(window, cx),
                    )))
                    .into_any_element(),
            ),
        };
        let dialog_width = if matches!(dialog, Dialog::Category(_)) { 400. } else { 560. };

        let card_bg = if theme.dark { gpui::rgb(0x29232a) } else { gpui::rgb(0xfffbfc) };
        Some(
            div()
                .id("dialog-overlay")
                .absolute()
                .inset_0()
                .bg(theme.overlay)
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("dialog")
                        .key_context("Dialog")
                        .track_focus(&focus)
                        .w(px(dialog_width))
                        .max_h(gpui::relative(0.92))
                        .flex()
                        .flex_col()
                        .bg(card_bg)
                        .rounded(px(18.))
                        .border_1()
                        .border_color(theme.border_strong)
                        .shadow(vec![BoxShadow {
                            color: gpui::hsla(0., 0., 0., if theme.dark { 0.5 } else { 0.18 }),
                            offset: point(px(0.), px(16.)),
                            blur_radius: px(48.),
                            spread_radius: px(0.),
                        }])
                        .child(
                            div()
                                .flex_none()
                                .h(px(58.))
                                .px(px(22.))
                                .flex()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(17.))
                                        .font_weight(FontWeight::BOLD)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .id("dialog-close")
                                        .size(px(30.))
                                        .rounded(px(8.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(theme.row_hover))
                                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                            this.close_dialog(window, cx)
                                        }))
                                        .child(icon("x", 16., theme.text_muted)),
                                ),
                        )
                        .child(
                            div()
                                .id("dialog-body")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .px(px(22.))
                                .pb(px(8.))
                                .child(body),
                        )
                        .child(footer),
                )
                .into_any_element(),
        )
    }

    fn render_tool_form(
        &self,
        editor: &ToolEditor,
        preview: Option<AnyElement>,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_category = editor.category.read(cx).text().trim().to_string();
        let category_chips = std::iter::once(String::new())
            .chain(self.config.categories.iter().cloned())
            .enumerate()
            .map(|(ix, name)| {
                let selected = current_category == name;
                let input = editor.category.clone();
                let value = name.clone();
                div()
                    .id(("category-chip", ix))
                    .h(px(26.))
                    .px(px(10.))
                    .rounded_full()
                    .border_1()
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.accent_soft).border_color(theme.accent).text_color(theme.accent))
                    .when(!selected, |d| {
                        d.border_color(theme.border).text_color(theme.text_muted).hover(|s| s.bg(theme.row_hover))
                    })
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        let value = value.clone();
                        input.update(cx, |input, cx| input.set_text(value, cx));
                    }))
                    .child(if name.is_empty() { "未分类".to_string() } else { name })
            })
            .collect::<Vec<_>>();
        let custom_icon = !editor.icon_path.is_empty();
        let program_icon = !custom_icon && editor.icon.is_empty();
        let icon_tiles = PICKER_ICONS.iter().filter(|n| !n.is_empty()).enumerate().map(|(ix, name)| {
            let selected = !custom_icon && editor.icon == *name;
            let (fg, bg) = theme.tint(&editor.color);
            let value = name.to_string();
            div()
                .id(("pick-icon", ix))
                .size(px(36.))
                .rounded(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .border_2()
                .border_color(if selected { theme.accent } else { gpui::transparent_black() })
                .bg(if selected { bg } else { theme.kbd_bg })
                .hover(|s| s.bg(bg))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(Dialog::Tool(editor)) = this.dialog.as_mut() {
                        editor.icon = value.clone();
                        editor.icon_path.clear();
                    }
                    cx.notify();
                }))
                .child(icon(name, 18., if selected { fg } else { theme.text_muted }))
        });

        let swatches = TINTS.iter().enumerate().map(|(ix, name)| {
            let selected = editor.color == *name;
            let (fg, _) = theme.tint(name);
            let value = name.to_string();
            div()
                .id(("pick-color", ix))
                .size(px(28.))
                .rounded_full()
                .p(px(3.))
                .border_2()
                .border_color(if selected { fg } else { gpui::transparent_black() })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(Dialog::Tool(editor)) = this.dialog.as_mut() {
                        editor.color = value.clone();
                    }
                    cx.notify();
                }))
                .child(div().size_full().rounded_full().bg(fg))
        });

        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().child(label("名称", theme)).child(self.input_box(&editor.name, theme, window, cx)))
            .child(
                div()
                    .child(label("目标", theme))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(self.input_box(&editor.target, theme, window, cx).flex_1().min_w_0())
                            .child(button("browse", "浏览…", false, theme).h(px(36.)).on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| this.browse_target(window, cx),
                            ))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(div().flex_1().child(label("启动参数", theme)).child(self.input_box(&editor.args, theme, window, cx)))
                    .child(div().flex_1().child(label("起始目录", theme)).child(self.input_box(&editor.workdir, theme, window, cx))),
            )
            .child(div().child(label("描述", theme)).child(self.input_box(&editor.description, theme, window, cx)))
            .child(
                div()
                    .child(label("分类", theme))
                    .child(self.input_box(&editor.category, theme, window, cx))
                    .child(div().mt(px(8.)).flex().flex_wrap().gap(px(6.)).children(category_chips)),
            )
            .child(div().child(label("下载地址", theme)).child(self.input_box(&editor.download_url, theme, window, cx)))
            .child(
                div()
                    .child(label("图标", theme))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .children(preview)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .flex()
                                            .gap(px(8.))
                                            .child(button("upload-icon", "上传图标…", false, theme).on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| this.upload_icon(cx),
                                            )))
                                            .child(
                                                button("program-icon", "使用程序自带图标", false, theme)
                                                    .when(program_icon, |b| b.border_color(theme.accent).text_color(theme.accent))
                                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                                        if let Some(Dialog::Tool(editor)) = this.dialog.as_mut() {
                                                            editor.icon.clear();
                                                            editor.icon_path.clear();
                                                        }
                                                        cx.notify();
                                                    })),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(theme.text_faint)
                                            .child(if custom_icon {
                                                "正在使用自定义图标"
                                            } else {
                                                "支持 PNG / JPG / ICO，也可以选一个 exe 提取它的图标"
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(10.))
                            .text_size(px(12.))
                            .text_color(theme.text_faint)
                            .mb(px(6.))
                            .child("或使用内置图标："),
                    )
                    .child(div().flex().flex_wrap().gap(px(6.)).children(icon_tiles)),
            )
            .child(
                div()
                    .child(label("颜色", theme))
                    .child(div().flex().gap(px(8.)).children(swatches)),
            )
            .child(
                div()
                    .id("favorite-row")
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(Dialog::Tool(editor)) = this.dialog.as_mut() {
                            editor.favorite = !editor.favorite;
                        }
                        cx.notify();
                    }))
                    .child(switch("favorite-switch", editor.favorite, theme))
                    .child(div().text_size(px(13.5)).child("加入收藏工具")),
            )
            .when_some(editor.error.clone(), |d, err| {
                d.child(div().text_size(px(12.5)).text_color(theme.danger).child(err))
            })
            .into_any_element()
    }

    fn render_tool_footer(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_none()
            .px(px(22.))
            .py(px(16.))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .text_color(theme.text_faint)
                    .child(kbd("Tab", theme))
                    .child("切换输入框"),
            )
            .child(div().flex_1())
            .child(button("cancel", "取消", false, theme).on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| this.close_dialog(window, cx),
            )))
            .child(button("save", "保存", true, theme).on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| this.save_tool(window, cx),
            )))
            .into_any_element()
    }

    fn render_settings_form(&self, editor: &SettingsEditor, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let settings = &self.config.settings;
        let section = |title: &'static str| {
            div()
                .mt(px(6.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.accent)
                .child(title)
        };

        let theme_options = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark]
            .into_iter()
            .enumerate()
            .map(|(ix, mode)| {
                let selected = settings.theme == mode;
                div()
                    .id(("theme-mode", ix))
                    .flex_1()
                    .h(px(30.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.panel_hover).shadow(soft_shadow(theme)).text_color(theme.accent))
                    .when(!selected, |d| d.text_color(theme.text_muted))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.config.settings.theme = mode;
                        this.config.save();
                        this.apply_theme(window, cx);
                    }))
                    .child(mode.label())
            });

        let current_accent = settings.accent.to_ascii_uppercase();
        let accent_swatches = ACCENT_PRESETS.iter().enumerate().map(|(ix, (name, hex))| {
            let color = parse_hex(hex).unwrap_or(theme.accent);
            let selected = current_accent == hex.to_ascii_uppercase();
            let hex = hex.to_string();
            div()
                .id(("accent", ix))
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(4.))
                .pr(px(9.))
                .h(px(28.))
                .rounded_full()
                .border_1()
                .border_color(if selected { color } else { theme.border })
                .when(selected, |d| d.bg(theme.accent_soft))
                .cursor_pointer()
                .hover(|s| s.bg(theme.row_hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(Dialog::Settings(editor)) = this.dialog.as_ref() {
                        let input = editor.accent.clone();
                        let hex = hex.clone();
                        // The input's change subscription applies the theme.
                        input.update(cx, |input, cx| input.set_text(hex, cx));
                    }
                }))
                .child(div().size(px(18.)).rounded_full().bg(color))
                .child(div().text_size(px(12.)).child(*name))
        });

        let opacity_options = [70u8, 80, 88, 95, 100].into_iter().enumerate().map(|(ix, value)| {
            let selected = settings.opacity == value;
            div()
                .id(("opacity", ix))
                .flex_1()
                .h(px(28.))
                .rounded(px(7.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.5))
                .cursor_pointer()
                .when(selected, |d| d.bg(theme.panel_hover).shadow(soft_shadow(theme)).text_color(theme.accent))
                .when(!selected, |d| d.text_color(theme.text_muted))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.config.settings.opacity = value;
                    this.config.save();
                    this.refresh_theme(cx);
                }))
                .child(format!("{value}%"))
        });

        let toggle = |id: &'static str, title: &'static str, desc: &'static str, on: bool, theme: &Theme| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(4.))
                .cursor_pointer()
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(13.5)).child(title))
                        .child(div().text_size(px(12.)).text_color(theme.text_faint).child(desc)),
                )
                .child(switch(SharedString::from(format!("{id}-switch")), on, theme))
        };

        let status = editor.status.clone();
        let appearance = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(section("外观"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().flex_1().text_size(px(13.5)).child("主题"))
                    .child(
                        div()
                            .w(px(270.))
                            .p(px(3.))
                            .rounded(px(9.))
                            .bg(theme.kbd_bg)
                            .flex()
                            .gap(px(2.))
                            .children(theme_options),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(div().flex_1().text_size(px(13.5)).child("主题色"))
                            .child(self.input_box(&editor.accent, theme, window, cx).w(px(120.)).h(px(32.))),
                    )
                    .child(div().flex().flex_wrap().gap(px(8.)).children(accent_swatches)),
            )
            .child(
                toggle("tint", "背景跟随主题色", "卡片和背景带一点主题色调；关闭则为中性灰白", settings.tint_surfaces, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.config.settings.tint_surfaces = !this.config.settings.tint_surfaces;
                        this.config.save();
                        this.refresh_theme(cx);
                    })),
            )
            .child(
                toggle("blur", "毛玻璃背景", "窗口背景透出桌面的模糊效果", settings.blur_background, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.config.settings.blur_background = !this.config.settings.blur_background;
                        this.config.save();
                        this.apply_theme(window, cx);
                    })),
            )
            .when(settings.blur_background, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(div().flex_1().text_size(px(13.5)).child("背景不透明度"))
                        .child(
                            div()
                                .w(px(270.))
                                .p(px(3.))
                                .rounded(px(9.))
                                .bg(theme.kbd_bg)
                                .flex()
                                .gap(px(2.))
                                .children(opacity_options),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(theme.text_faint)
                    .child(div().flex_1().child("高级：可在 config.json 的 theme_overrides 中逐项覆盖颜色"))
                    .child(
                        div()
                            .id("reset-theme")
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_color(theme.accent)
                            .hover(|s| s.bg(theme.accent_soft))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                let s = &mut this.config.settings;
                                s.accent = DEFAULT_ACCENT.into();
                                s.tint_surfaces = true;
                                s.opacity = 88;
                                s.theme_overrides = Default::default();
                                this.config.save();
                                if let Some(Dialog::Settings(editor)) = this.dialog.as_ref() {
                                    let accent = editor.accent.clone();
                                    accent.update(cx, |input, cx| input.set_text(DEFAULT_ACCENT, cx));
                                }
                                this.apply_theme(window, cx);
                            }))
                            .child(icon("reset", 12., theme.accent))
                            .child("恢复默认主题"),
                    ),
            )
;
        let general = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(section("快捷键"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(13.5)).child("呼出 / 隐藏"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_faint)
                                    .child("例如 Alt+Space、Ctrl+Shift+L、Win+Space"),
                            ),
                    )
                    .child(self.input_box(&editor.hotkey, theme, window, cx).w(px(200.))),
            )
            .when_some(status, |d, (msg, error)| {
                d.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(if error { theme.danger } else { theme.accent })
                        .child(msg),
                )
            })
            .child(section("行为"))
            .child(
                toggle("mini", "快捷键唤起迷你搜索框", "只显示一个小输入框，点“还原窗口”回到完整界面", settings.mini_on_hotkey, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.config.settings.mini_on_hotkey = !this.config.settings.mini_on_hotkey;
                        this.config.save();
                        cx.notify();
                    })),
            )
            .child(
                toggle(
                    "alt-ring",
                    "按住 Alt 弹出圆环菜单",
                    "单独按住 Alt 约 0.3 秒，屏幕右下角会弹出常用工具的圆环（收藏的排在前面）；指向图标松开 Alt 即可打开",
                    settings.alt_ring,
                    theme,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    let on = !this.config.settings.alt_ring;
                    this.config.settings.alt_ring = on;
                    this.config.save();
                    crate::preview::hook::RING_ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
                    cx.notify();
                })),
            )
            .child(
                toggle(
                    "everything",
                    "Everything 文件搜索",
                    if crate::everything::is_running() {
                        "已连接 Everything，搜索时会一并显示匹配的文件"
                    } else {
                        "未检测到正在运行的 Everything（voidtools），启动后即可使用"
                    },
                    settings.everything,
                    theme,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.config.settings.everything = !this.config.settings.everything;
                    this.config.save();
                    cx.notify();
                })),
            )
            .child(
                toggle("autostart", "开机自动启动", "登录 Windows 后在托盘中静默运行", settings.autostart, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let enabled = !this.config.settings.autostart;
                        this.set_autostart(enabled, cx);
                    })),
            )
            .child(
                toggle("tray", "关闭时最小化到托盘", "点击关闭按钮不会退出程序", settings.close_to_tray, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.config.settings.close_to_tray = !this.config.settings.close_to_tray;
                        this.config.save();
                        cx.notify();
                    })),
            )
            .child(
                toggle("hide-launch", "启动工具后自动隐藏", "打开工具后收起启动器窗口", settings.hide_after_launch, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.config.settings.hide_after_launch = !this.config.settings.hide_after_launch;
                        this.config.save();
                        cx.notify();
                    })),
            )
            .child(
                toggle("hide-blur", "失去焦点时自动隐藏", "像 Spotlight 一样，点到别处就收起", settings.hide_on_blur, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.config.settings.hide_on_blur = !this.config.settings.hide_on_blur;
                        this.config.save();
                        cx.notify();
                    })),
            )
            .child(section("终端"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(13.5)).child("终端程序"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_faint)
                                    .child(format!("留空自动检测（当前：{}）", editor.detected_terminal)),
                            ),
                    )
                    .child(self.input_box(&editor.terminal, theme, window, cx).w(px(200.))),
            )
            .child(section("关于与更新"))
            .child(self.render_update_row(theme, cx))
            .child(
                toggle(
                    "auto-update",
                    "启动时自动检查更新",
                    "每天最多检查一次，发现新版本时提醒你",
                    self.config.settings.auto_check_update,
                    theme,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.config.settings.auto_check_update = !this.config.settings.auto_check_update;
                    this.config.save();
                    cx.notify();
                })),
            );

        let tabs = SettingsTab::ALL.into_iter().enumerate().map(|(ix, tab)| {
            let selected = editor.tab == tab;
            div()
                .id(("settings-tab", ix))
                .flex_1()
                .h(px(30.))
                .rounded(px(7.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(theme.panel_hover)
                        .shadow(soft_shadow(theme))
                        .text_color(theme.accent)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!selected, |d| d.text_color(theme.text_muted))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.commit_settings_inputs(cx);
                    if let Some(Dialog::Settings(editor)) = this.dialog.as_mut() {
                        editor.tab = tab;
                    }
                    cx.notify();
                }))
                .child(tab.label())
        })
        .collect::<Vec<_>>();
        let body = match editor.tab {
            SettingsTab::General => general.into_any_element(),
            SettingsTab::Appearance => appearance.into_any_element(),
            SettingsTab::Preview => self.render_preview_settings(theme, cx),
            SettingsTab::Ai => self.render_ai_settings(editor, theme, window, cx),
            SettingsTab::Search => self.render_search_settings(editor, theme, window, cx),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .p(px(3.))
                    .rounded(px(9.))
                    .bg(theme.kbd_bg)
                    .flex()
                    .gap(px(2.))
                    .children(tabs),
            )
            .child(body)
            .into_any_element()
    }

    fn render_preview_settings(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::preview::ffmpeg::{self, FfmpegState};
        let state = cx.global::<FfmpegState>();
        let path = state.path.clone();
        let progress = state.download.as_ref().map(|p| p.fraction().unwrap_or(0.));
        let error = state.error.clone();
        let enabled = self.config.settings.space_preview;
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .id("space-preview")
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let on = !this.config.settings.space_preview;
                        this.config.settings.space_preview = on;
                        this.config.save();
                        crate::preview::hook::ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(13.5)).child("空格快速预览"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_faint)
                                    .child("在资源管理器或桌面选中文件后按空格预览，再按空格或 Esc 关闭；方向键切换文件"),
                            ),
                    )
                    .child(switch("space-preview-switch", enabled, theme)),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text_faint)
                    .child("支持：Markdown、图片、文本与代码、PDF、网页、Word（.docx）、Excel（.xlsx）、CSV / TSV、压缩包、音频、视频、文件夹"),
            )
            .child(
                div()
                    .mt(px(6.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.accent)
                    .child("FFmpeg（视频与 FLAC / OGG / M4A 等音频）"),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(if path.is_some() { theme.text_muted } else { theme.text_faint })
                    .child(match &path {
                        Some(p) => format!("已就绪：{}", p.display()),
                        None => "未安装。MP3 / WAV / WMA 无需 FFmpeg 也能播放".to_string(),
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(match progress {
                        Some(f) => button("ffmpeg-progress", "下载中…", true, theme)
                            .child(format!(" {:.0}%", f * 100.))
                            .into_any_element(),
                        None => button("ffmpeg-download", if path.is_some() { "重新下载" } else { "一键下载" }, path.is_none(), theme)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.download_ffmpeg(cx)))
                            .into_any_element(),
                    })
                    .child(button("ffmpeg-choose", "选择本地 ffmpeg.exe…", false, theme).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| this.choose_ffmpeg(cx),
                    ))),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(theme.text_faint)
                    .child(format!("一键下载来自 gyan.dev 的 FFmpeg essentials 版本（{}），保存在本机应用数据目录", ffmpeg::DOWNLOAD_SIZE_HINT)),
            )
            .when_some(error, |d, err| d.child(div().text_size(px(12.)).text_color(theme.danger).child(err)))
            .into_any_element()
    }

    fn render_update_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let update = &self.update;
        let busy = update.checking || update.progress.is_some();
        let available = update.available.as_ref();
        // First few lines of the release notes, rendered as Markdown.
        let notes: Option<String> = available.map(|i| {
            i.notes
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .take(6)
                .collect::<Vec<_>>()
                .join("\n")
        });
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .child(format!("LovelyLauncher v{}", crate::update::CURRENT_VERSION)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_faint)
                                    .child(format!("github.com/{}", crate::update::REPO)),
                            ),
                    )
                    .when(available.is_some() && !busy, |d| {
                        d.child(button("update-now", "下载并更新", true, theme).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| this.install_update(cx),
                        )))
                    })
                    .child(
                        button(
                            if available.is_some() { "release-page" } else { "check-update" },
                            if available.is_some() {
                                "查看发布页"
                            } else if update.checking {
                                "检查中…"
                            } else {
                                "检查更新"
                            },
                            false,
                            theme,
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if this.update.available.is_some() {
                                this.open_release_page(cx);
                            } else {
                                this.check_update(true, cx);
                            }
                        })),
                    ),
            )
            .when_some(update.status.clone(), |d, (msg, error)| {
                d.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(if error { theme.danger } else { theme.accent })
                        .child(msg),
                )
            })
            .when_some(notes.filter(|n| !n.is_empty()), |d, notes| {
                d.child(
                    div()
                        .p(px(10.))
                        .rounded(px(8.))
                        .bg(theme.kbd_bg)
                        .text_color(theme.text_muted)
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .children(crate::markdown::render(&notes, None, theme)),
                )
            })
            .into_any_element()
    }

    fn render_ai_settings(&self, editor: &SettingsEditor, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let settings = &self.config.settings;
        let chip = |id: SharedString, label: String, selected: bool, theme: &Theme| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(12.))
                .rounded_full()
                .border_1()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(12.5))
                .cursor_pointer()
                .when(selected, |d| d.bg(theme.accent_soft).border_color(theme.accent).text_color(theme.accent))
                .when(!selected, |d| {
                    d.border_color(theme.border).text_color(theme.text_muted).hover(|s| s.bg(theme.row_hover))
                })
                .child(label)
        };
        let providers = settings.ai_providers.iter().map(|p| {
            let id = p.id.clone();
            let is_default = p.id == settings.ai_active;
            let label = if is_default { format!("{} ★", p.name) } else { p.name.clone() };
            chip(format!("ai-provider-{}", p.id).into(), label, p.id == editor.ai_selected, theme)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.select_ai_provider(id.clone(), cx)))
        });
        let selected = settings.ai_providers.iter().find(|p| p.id == editor.ai_selected);
        let current_model = editor.ai_model.read(cx).text().trim().to_string();
        let models = selected
            .map(|p| p.models.clone())
            .unwrap_or_default()
            .into_iter()
            .take(60)
            .enumerate()
            .map(|(ix, model)| {
                let input = editor.ai_model.clone();
                let value = model.clone();
                chip(SharedString::from(format!("ai-model-{ix}")), model.clone(), model == current_model, theme)
                    .h(px(24.))
                    .text_size(px(12.))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        let value = value.clone();
                        input.update(cx, |i, cx| i.set_text(value, cx));
                    }))
            })
            .collect::<Vec<_>>();
        let is_default = editor.ai_selected == settings.ai_active;
        let selected_id = editor.ai_selected.clone();
        let field = |title: &'static str, input: &Entity<TextInput>| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().w(px(72.)).flex_none().text_size(px(13.)).child(title))
                .child(self.input_box(input, theme, window, cx).flex_1().min_w_0())
        };

        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(theme.text_faint)
                    .child("在搜索框输入 “@ai 你的问题” 后按 Enter 即可提问；也可以用 @deepseek、@claude 等直接指定服务商。"),
            )
            .child(div().flex().flex_wrap().gap(px(6.)).children(providers))
            .child(field("接口地址", &editor.ai_base_url))
            .child(field("API Key", &editor.ai_key))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().w(px(72.)).flex_none().text_size(px(13.)).child("模型"))
                    .child(self.input_box(&editor.ai_model, theme, window, cx).flex_1().min_w_0())
                    .child(
                        button("fetch-models", if editor.ai_fetching { "拉取中…" } else { "拉取模型" }, false, theme)
                            .h(px(36.))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.fetch_ai_models(cx))),
                    ),
            )
            .when(!models.is_empty(), |d| {
                d.child(
                    div()
                        .id("ai-models")
                        .max_h(px(120.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .children(models),
                )
            })
            .when_some(editor.ai_status.clone(), |d, (msg, error)| {
                d.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(if error { theme.danger } else { theme.accent })
                        .child(msg),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(switch("ai-default-switch", is_default, theme).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.config.settings.ai_active = selected_id.clone();
                            this.config.save();
                            cx.notify();
                        },
                    )))
                    .child(div().text_size(px(13.5)).child("作为 @ai 默认使用的服务商")),
            )
            .into_any_element()
    }

    fn render_search_settings(&self, editor: &SettingsEditor, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let current = self.config.settings.search_engine.clone();
        let engines = crate::config::SEARCH_ENGINES.iter().enumerate().map(|(ix, (name, url))| {
            let selected = current == *url;
            let input = editor.search_url.clone();
            let value = url.to_string();
            div()
                .id(("search-engine", ix))
                .h(px(30.))
                .px(px(14.))
                .rounded_full()
                .border_1()
                .flex()
                .items_center()
                .text_size(px(13.))
                .cursor_pointer()
                .when(selected, |d| d.bg(theme.accent_soft).border_color(theme.accent).text_color(theme.accent))
                .when(!selected, |d| {
                    d.border_color(theme.border).text_color(theme.text_muted).hover(|s| s.bg(theme.row_hover))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.config.settings.search_engine = value.clone();
                    this.config.save();
                    let value = value.clone();
                    input.update(cx, |i, cx| i.set_text(value, cx));
                    cx.notify();
                }))
                .child(*name)
        });
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(theme.text_faint)
                    .child("搜索不到本地结果时，列表最后一项会用这里的搜索引擎在网页上搜索。"),
            )
            .child(div().flex().flex_wrap().gap(px(8.)).children(engines))
            .child(label("自定义搜索地址（用 {q} 表示搜索词）", theme))
            .child(self.input_box(&editor.search_url, theme, window, cx))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text_faint)
                    .child(format!("当前：{}", self.config.settings.search_engine_name())),
            )
            .into_any_element()
    }

    fn render_settings_footer(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_none()
            .px(px(22.))
            .py(px(16.))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .id("open-config")
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.5))
                    .text_color(theme.text_muted)
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme.accent))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let dir = Config::dir();
                        let _ = std::fs::create_dir_all(&dir);
                        this.spawn_launch(dir.display().to_string(), String::new(), String::new(), false, cx);
                    }))
                    .child(icon("folder-open", 14., theme.text_muted))
                    .child("打开配置文件夹"),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text_faint)
                    .child(concat!("v", env!("CARGO_PKG_VERSION"))),
            )
            .child(div().flex_1())
            .child(button("settings-done", "完成", true, theme).on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| this.save_settings(window, cx),
            )))
            .into_any_element()
    }
}
