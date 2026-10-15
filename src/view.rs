//! Rendering of the main launcher window.

use gpui::{
    AnyElement, BoxShadow, ClickEvent, Context, Div, ExternalPaths, FontWeight, Hsla, MouseButton,
    Focusable, MouseDownEvent, SharedString, Stateful, Window, WindowControlArea, anchored,
    deferred, div,
    img, point, prelude::*, px, rgb, svg,
};

use crate::{
    config::{ThemeMode, relative_time},
    icons,
    launcher::*,
    theme::Theme,
};

/// Which grid a tool tile is in (decides what dropping on it does).
#[derive(Clone, Copy, PartialEq)]
enum TileIn {
    Grid,
    Favorites,
}

/// What dropping a dragged tool on a tab does.
#[derive(Clone)]
enum TabDrop {
    Category(String),
    Ring,
}

/// Size-dependent layout values, recomputed from the viewport every frame so the
/// dashboard fits small logical resolutions (e.g. 1080p at 150% scaling).
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub cols: u16,
    pub bottom_cols: u16,
    pub titlebar: f32,
    pub logo: f32,
    pub title_size: f32,
    pub search_h: f32,
    pub search_pb: f32,
    pub section_gap: f32,
    pub header_gap: f32,
    pub grid_gap: f32,
    pub fav_h: f32,
    pub fav_pad: f32,
    pub fav_tile: f32,
    pub fav_gap: f32,
    pub fav_lines: usize,
    pub tile_cols: u16,
    pub tile_h: f32,
    pub tile_icon: f32,
    pub recent_rows: usize,
    /// Height of exactly two rows of tool tiles; the grid scrolls beyond that.
    pub grid_h: f32,
    /// Favourites as tiles (more than fit as cards): one or two rows, scrolling beyond.
    pub fav_tiles: bool,
    pub fav_grid_h: f32,
    pub bottom_min: f32,
    pub row_h: f32,
    pub action_h: f32,
    pub action_gap: f32,
}

impl Metrics {
    pub fn for_viewport(width: f32, height: f32, favorites: usize) -> Self {
        let mut m = Self::base(width, height);
        m.fav_tiles = favorites > m.cols as usize;
        let fav_rows = favorites.div_ceil(m.tile_cols as usize).clamp(1, 2) as f32;
        m.fav_grid_h = fav_rows * m.tile_h + (fav_rows - 1.) * m.grid_gap;
        // Vertical budget: everything except the tile grid has a known height, so
        // give the grid as many whole rows as fit and the rest to the bottom panels.
        const HEADER: f32 = 22.;
        const TABS: f32 = 28.;
        let avail = height - m.titlebar - m.search_h - m.search_pb - 3. * m.section_gap;
        let fav_block = HEADER + m.header_gap + if m.fav_tiles { m.fav_grid_h } else { m.fav_h };
        let tools_fixed = HEADER + TABS + 2. * m.header_gap;
        // Always two rows of tiles; the rest scrolls inside the grid.
        const ROWS: f32 = 2.;
        m.grid_h = ROWS * m.tile_h + (ROWS - 1.) * m.grid_gap;
        let bottom_h = (avail - fav_block - tools_fixed - m.grid_h).max(m.bottom_min);
        m.recent_rows = ((bottom_h - 52.) / m.row_h).floor().clamp(2., 8.) as usize;
        m
    }

    fn base(width: f32, height: f32) -> Self {
        let compact = height < 700.;
        let cols = if width >= 900. {
            4
        } else if width >= 640. {
            3
        } else {
            2
        };
        let c = |normal: f32, small: f32| if compact { small } else { normal };
        Self {
            cols,
            bottom_cols: if width >= 860. { 3 } else { 1 },
            titlebar: c(64., 54.),
            logo: c(40., 34.),
            title_size: c(18., 16.),
            search_h: c(50., 44.),
            search_pb: c(16., 12.),
            section_gap: c(18., 12.),
            header_gap: c(12., 8.),
            grid_gap: c(14., 10.),
            fav_h: c(124., 108.),
            fav_pad: c(14., 12.),
            fav_tile: c(44., 36.),
            fav_gap: c(10., 8.),
            fav_lines: 1,
            // Roughly square tiles of at least 104px across the 24px-padded content width.
            tile_cols: (((width - 48. + 10.) / 114.).floor() as u16).clamp(3, 12),
            tile_h: c(100., 88.),
            tile_icon: c(44., 38.),
            recent_rows: 0,
            grid_h: 0.,
            fav_tiles: false,
            fav_grid_h: 0.,
            bottom_min: c(206., 184.),
            row_h: c(36., 32.),
            action_h: c(34., 30.),
            action_gap: c(6., 4.),
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::for_viewport(1040., 800., 0)
    }
}

pub fn icon(name: &str, size: f32, color: Hsla) -> gpui::Svg {
    svg()
        .path(icons::path(name))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

pub fn kbd(text: &'static str, theme: &Theme) -> Div {
    div()
        .min_w(px(22.))
        .h(px(22.))
        .px(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .bg(theme.kbd_bg)
        .border_1()
        .border_color(theme.kbd_border)
        .text_size(px(11.5))
        .text_color(theme.text_muted)
        .child(text)
}

pub fn soft_shadow(theme: &Theme) -> Vec<BoxShadow> {
    let alpha = if theme.dark { 0.25 } else { 0.06 };
    vec![BoxShadow {
        color: gpui::hsla(330. / 360., 0.3, 0.2, alpha),
        offset: point(px(0.), px(2.)),
        blur_radius: px(10.),
        spread_radius: px(0.),
    }]
}

fn link(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(6.))
        .py(px(2.))
        .rounded(px(6.))
        .text_size(px(12.5))
        .text_color(theme.accent)
        .cursor_pointer()
        .hover(|s| s.bg(theme.accent_soft))
        .child(label.into())
}

fn panel(theme: &Theme) -> Div {
    div()
        .bg(theme.panel)
        .rounded(px(14.))
        .border_1()
        .border_color(theme.border)
        .shadow(soft_shadow(theme))
}

impl Launcher {
    /// Icon tile for a visual (glyph on a tinted square, or the file's own icon).
    pub(crate) fn tile(&mut self, visual: &Visual, box_size: f32, icon_size: f32, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let base = div()
            .size(px(box_size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(box_size * 0.28));
        match visual {
            Visual::Glyph { icon: name, tint } => {
                let (fg, bg) = theme.tint(tint);
                base.bg(bg).child(icon(name, icon_size, fg)).into_any_element()
            }
            Visual::Folder => {
                let (_, bg) = theme.tint("orange");
                base.bg(bg).child(icon("folder", icon_size, theme.folder)).into_any_element()
            }
            Visual::FileType { ext } => match self.type_icon(ext, cx) {
                Some(image) => base
                    .bg(theme.kbd_bg)
                    .child(img(image).size(px(box_size * 0.72)))
                    .into_any_element(),
                None => {
                    let (fg, bg) = theme.tint("slate");
                    base.bg(bg).child(icon("file", icon_size, fg)).into_any_element()
                }
            },
            Visual::Image { path } => match self.image_icon(path, cx) {
                Some(image) => base
                    .bg(theme.kbd_bg)
                    .child(img(image).size(px(box_size * 0.86)).rounded(px(box_size * 0.2)))
                    .into_any_element(),
                None => {
                    let (fg, bg) = theme.tint("slate");
                    base.bg(bg).child(icon("image", icon_size, fg)).into_any_element()
                }
            },
            Visual::File {
                target,
                tint,
                fallback,
            } => match self.file_icon(target, cx) {
                Some(image) => base
                    .bg(theme.kbd_bg)
                    .child(img(image).size(px(box_size * 0.72)))
                    .into_any_element(),
                None => {
                    let (fg, bg) = theme.tint(tint);
                    base.bg(bg).child(icon(fallback, icon_size, fg)).into_any_element()
                }
            },
        }
    }

    /// Secondary line for a tool card: download progress, or the description.
    fn tool_subtitle(&self, tool: &crate::config::Tool) -> (SharedString, Option<f32>) {
        if let Some(progress) = self.downloads.get(&tool.id) {
            let mb = |b: u64| b as f64 / 1024. / 1024.;
            let done = progress.downloaded.load(std::sync::atomic::Ordering::Relaxed);
            let total = progress.total.load(std::sync::atomic::Ordering::Relaxed);
            let fraction = progress.fraction();
            let text = match fraction {
                Some(f) => format!("下载中 {:.0}% · {:.1} / {:.1} MB", f * 100., mb(done), mb(total)),
                None => format!("下载中 · {:.1} MB", mb(done)),
            };
            return (text.into(), Some(fraction.unwrap_or(0.)));
        }
        if Self::needs_download(tool) {
            let text = if tool.description.is_empty() {
                "点击下载并启动".to_string()
            } else {
                tool.description.clone()
            };
            return (text.into(), None);
        }
        let text = if tool.description.is_empty() {
            tool.target.clone()
        } else {
            tool.description.clone()
        };
        (text.into(), None)
    }

    fn progress_bar(fraction: f32, theme: &Theme) -> Div {
        div()
            .absolute()
            .left(px(14.))
            .right(px(14.))
            .bottom(px(8.))
            .h(px(4.))
            .rounded_full()
            .bg(theme.accent_soft)
            .child(
                div()
                    .h_full()
                    .rounded_full()
                    .bg(theme.accent)
                    .w(gpui::relative(fraction.clamp(0.02, 1.))),
            )
    }

    fn download_pill(theme: &Theme, with_label: bool) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(3.))
            .px(px(if with_label { 6. } else { 4. }))
            .py(px(if with_label { 1. } else { 3. }))
            .rounded(px(6.))
            .bg(theme.accent_soft)
            .text_size(px(11.))
            .text_color(theme.accent)
            .child(icon("download", 11., theme.accent))
            .when(with_label, |d| d.child("需下载"))
    }

    fn more_button(&self, id: impl Into<gpui::ElementId>, target: MenuTarget, theme: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(id)
            .size(px(26.))
            .flex_none()
            .rounded(px(7.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(theme.row_hover))
            .child(icon("more", 16., theme.text_faint))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.menu = Some(ContextMenu {
                        target: target.clone(),
                        position: ev.position,
                    });
                    cx.notify();
                }),
            )
    }

    fn open_menu_on_right_click(target: MenuTarget, cx: &mut Context<Self>) -> impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.menu = Some(ContextMenu {
                target: target.clone(),
                position: ev.position,
            });
            cx.notify();
        })
    }

    // -----------------------------------------------------------------------
    // Title bar

    fn render_titlebar(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let m = self.metrics;
        let theme_icon = match self.config.settings.theme {
            ThemeMode::System => "contrast",
            ThemeMode::Light => "sun",
            ThemeMode::Dark => "moon",
        };
        let title_button = |id: &'static str, name: &'static str, theme: &Theme| {
            div()
                .id(id)
                .size(px(34.))
                .rounded(px(9.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .occlude()
                .hover(|s| s.bg(theme.row_hover))
                .child(icon(name, 17., theme.text_muted))
        };
        let caption = |id: &'static str, name: &'static str, area: WindowControlArea, theme: &Theme| {
            let close = matches!(area, WindowControlArea::Close);
            let hover_fg: Hsla = if close { gpui::white() } else { theme.text };
            div()
                .id(id)
                .group(id)
                .w(px(46.))
                .h(px(34.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .window_control_area(area)
                .text_color(theme.text_muted)
                .when(close, |d| d.hover(|s| s.bg(rgb(0xe81123)).text_color(gpui::white())))
                .when(!close, |d| d.hover(|s| s.bg(theme.row_hover)))
                .child(icon_hover(name, 14., theme.text_muted, id, hover_fg))
        };

        div()
            .id("titlebar")
            .h(px(m.titlebar))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(24.))
            .pr(px(10.))
            .window_control_area(WindowControlArea::Drag)
            .child(img(self.logo.clone()).size(px(m.logo)).flex_none())
            .child(
                div()
                    .ml(px(12.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(m.title_size))
                            .font_weight(FontWeight::BOLD)
                            .child("LovelyLauncher"),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(theme.text_muted)
                            .child("小工具，大进步。"),
                    ),
            )
            .child(div().flex_1())
            .child(
                title_button("theme-toggle", theme_icon, theme).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.config.settings.theme = this.config.settings.theme.next();
                        this.config.save();
                        this.apply_theme(window, cx);
                        let label = this.config.settings.theme.label();
                        this.show_toast(format!("主题：{label}"), false, cx);
                    },
                )),
            )
            .child(
                title_button("settings", "settings", theme)
                    .ml(px(4.))
                    .relative()
                    .when(self.update.available.is_some(), |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(6.))
                                .right(px(6.))
                                .size(px(7.))
                                .rounded_full()
                                .bg(theme.accent),
                        )
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_settings(window, cx)
                    })),
            )
            .child(div().w(px(1.)).h(px(18.)).mx(px(10.)).bg(theme.border_strong))
            .child(caption("min", "minus", WindowControlArea::Min, theme))
            .child(caption("max", "square", WindowControlArea::Max, theme))
            .child(caption("close", "x", WindowControlArea::Close, theme))
    }

    // -----------------------------------------------------------------------
    // Search box

    fn render_search(&mut self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let m = self.metrics;
        let focused = self.search.focus_handle(cx).is_focused(window);
        div().px(px(24.)).pb(px(m.search_pb)).flex_none().child(
            div()
                .id("searchbox")
                .h(px(m.search_h))
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(18.))
                .bg(theme.input_bg)
                .rounded(px(16.))
                .border_1()
                .border_color(if focused { theme.accent } else { theme.border })
                .shadow(soft_shadow(theme))
                .cursor_text()
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    window.focus(&this.search.focus_handle(cx));
                }))
                .child(icon("search", 19., if focused { theme.accent } else { theme.text_faint }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .text_size(px(15.5))
                        .line_height(px(24.))
                        .child(self.search.clone()),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(kbd("Ctrl", theme))
                        .child(kbd("K", theme)),
                ),
        )
    }

    // -----------------------------------------------------------------------
    // Dashboard

    fn section_header(&self, icon_name: &'static str, icon_color: Hsla, title: &'static str, theme: &Theme) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(icon(icon_name, 18., icon_color))
            .child(
                div()
                    .text_size(px(15.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(title),
            )
            .child(div().flex_1())
    }

    fn render_dashboard(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let m = self.metrics;
        let favorites: Vec<crate::config::Tool> = self.config.favorites().into_iter().cloned().collect();

        // Favourites: big cards while they fit in one row, square tiles beyond that.
        let mut fav_cards = Vec::new();
        let mut fav_tiles = Vec::new();
        for (ix, tool) in favorites.iter().enumerate() {
            if m.fav_tiles {
                fav_tiles.push(self.tool_tile(ix, tool, TileIn::Favorites, theme, cx));
                continue;
            }
            let dragged = DraggedTool {
                id: tool.id.clone(),
                name: tool.name.clone().into(),
            };
            let target = tool.id.clone();
            let visual = Self::tool_visual(tool);
            let tile = self.tile(&visual, m.fav_tile, m.fav_tile / 2., theme, cx);
            let (subtitle, progress) = self.tool_subtitle(tool);
            let pending = progress.is_none() && Self::needs_download(tool);
            let id = tool.id.clone();
            let menu_target = MenuTarget::Tool(tool.id.clone());
            fav_cards.push(
                panel(theme)
                    .id(("fav", ix))
                    .relative()
                    .h(px(m.fav_h))
                    .p(px(m.fav_pad))
                    .flex()
                    .flex_col()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.panel_hover).border_color(theme.border_strong))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.activate(Target::Tool(id.clone()), false, window, cx)
                    }))
                    .on_mouse_down(MouseButton::Right, Self::open_menu_on_right_click(menu_target.clone(), cx))
                    .on_drag(dragged, |dragged, _, _, cx| {
                        cx.new(|_| DragPreview {
                            name: dragged.name.clone(),
                        })
                    })
                    .drag_over::<DraggedTool>(|s, _, _, cx| {
                        let t = Theme::get(cx);
                        s.bg(t.accent_soft).border_color(t.accent)
                    })
                    .on_drop(cx.listener(move |this, dragged: &DraggedTool, _, cx| {
                        this.drop_on_favorites(&dragged.id, Some(&target), cx)
                    }))
                    .child(tile)
                    .child(
                        div()
                            .mt(px(m.fav_gap))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(tool.name.clone()),
                            )
                            .when(pending, |d| d.child(Self::download_pill(theme, true))),
                    )
                    .child(
                        div()
                            .mt(px(3.))
                            .text_size(px(12.5))
                            .line_height(px(17.))
                            .text_color(if progress.is_some() { theme.accent } else { theme.text_muted })
                            .line_clamp(m.fav_lines)
                            .child(subtitle),
                    )
                    .when_some(progress, |d, f| d.child(Self::progress_bar(f, theme)))
                    .child(
                        div()
                            .absolute()
                            .top(px(10.))
                            .right(px(10.))
                            .child(self.more_button(("fav-more", ix), menu_target, theme, cx)),
                    )
                    .into_any_element(),
            );
        }

        let fav_total = favorites.len();
        let favorites_section = div()
            .id("favorites")
            .flex()
            .flex_col()
            .gap(px(m.header_gap))
            // Drop a tile anywhere here to make it a favourite (it goes last).
            .on_drop(cx.listener(|this, dragged: &DraggedTool, _, cx| this.drop_on_favorites(&dragged.id, None, cx)))
            .child(
                self.section_header("star-fill", rgb(0xf5b83d).into(), "收藏工具", theme)
                    .when(fav_total > 0, |d| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.text_faint)
                                .mr(px(8.))
                                .child(format!("{fav_total} 个 · 拖动可排序，从下方拖进来即可收藏")),
                        )
                    })
                    .child(link("add-tool-top", "+ 添加工具", theme).on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| this.open_tool_editor(None, None, window, cx),
                    ))),
            )
            .child(if m.fav_tiles {
                div()
                    .id("fav-grid")
                    .h(px(m.fav_grid_h))
                    .overflow_y_scroll()
                    .track_scroll(&self.fav_scroll)
                    .child(div().grid().grid_cols(m.tile_cols).gap(px(m.grid_gap)).children(fav_tiles))
                    .into_any_element()
            } else if fav_cards.is_empty() {
                panel(theme)
                    .id("fav-empty")
                    .drag_over::<DraggedTool>(|s, _, _, cx| {
                        let t = Theme::get(cx);
                        s.bg(t.accent_soft).border_color(t.accent).text_color(t.accent)
                    })
                    .h(px(76.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.text_muted)
                    .text_size(px(13.))
                    .child("把下方的工具拖到这里，或在 ⋯ 菜单中选择“收藏”")
                    .into_any_element()
            } else {
                div()
                    .grid()
                    .grid_cols(m.cols)
                    .gap(px(m.grid_gap))
                    .children(fav_cards)
                    .into_any_element()
            });

        let tools_section = self.render_tools_section(theme, cx);

        let bottom = div()
            .grid()
            .grid_cols(m.bottom_cols)
            .gap(px(m.grid_gap))
            .child(self.render_recent_panel(theme, cx))
            .child(self.render_projects_panel(theme, cx))
            .child(self.render_actions_panel(theme, cx));

        div()
            .id("dashboard")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.body_scroll)
            .px(px(24.))
            .pb(px(m.section_gap))
            .flex()
            .flex_col()
            .gap(px(m.section_gap))
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(Theme::get(cx).accent_soft))
            .child(favorites_section.flex_none())
            .child(tools_section)
            .child(bottom.flex_1().min_h(px(m.bottom_min)))
    }

    /// A square tool tile (icon over name). Dropping another tile on it moves that
    /// one into its spot: in 全部工具, or among the favourites.
    fn tool_tile(&mut self, ix: usize, tool: &crate::config::Tool, place: TileIn, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let m = self.metrics;
        let visual = Self::tool_visual(tool);
        let tile = self.tile(&visual, m.tile_icon, m.tile_icon / 2., theme, cx);
        let (_, progress) = self.tool_subtitle(tool);
        let pending = progress.is_none() && Self::needs_download(tool);
        let id = tool.id.clone();
        let target = tool.id.clone();
        let menu_target = MenuTarget::Tool(tool.id.clone());
        let (key, more_key) = match place {
            TileIn::Grid => ("tool", "tool-more"),
            TileIn::Favorites => ("fav-tile", "fav-tile-more"),
        };
        let group: SharedString = format!("{key}-{ix}").into();
        let dragged = DraggedTool {
            id: tool.id.clone(),
            name: tool.name.clone().into(),
        };
        div()
            .id((key, ix))
            .group(group.clone())
            .relative()
            .h(px(m.tile_h))
            .rounded(px(14.))
            .border_1()
            .border_color(gpui::transparent_black())
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .px(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(theme.panel).border_color(theme.border))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.activate(Target::Tool(id.clone()), false, window, cx)
            }))
            .on_mouse_down(MouseButton::Right, Self::open_menu_on_right_click(menu_target.clone(), cx))
            .on_drag(dragged, |dragged, _, _, cx| {
                cx.new(|_| DragPreview {
                    name: dragged.name.clone(),
                })
            })
            // Drop another tile here to move it into this spot.
            .drag_over::<DraggedTool>(|s, _, _, cx| {
                let t = Theme::get(cx);
                s.bg(t.accent_soft).border_color(t.accent)
            })
            .on_drop(cx.listener(move |this, dragged: &DraggedTool, _, cx| match place {
                TileIn::Grid => this.move_tool(&dragged.id, Some(&target), cx),
                TileIn::Favorites => this.drop_on_favorites(&dragged.id, Some(&target), cx),
            }))
            .child(tile)
            .child(
                div().w_full().flex().justify_center().child(
                    div()
                        .max_w_full()
                        .text_size(px(12.5))
                        .truncate()
                        .text_color(if progress.is_some() { theme.accent } else { theme.text })
                        .child(match progress {
                            Some(f) => format!("{:.0}%", f * 100.),
                            None => tool.name.clone(),
                        }),
                ),
            )
            .when(pending, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(6.))
                        .left(px(6.))
                        .child(Self::download_pill(theme, false)),
                )
            })
            .child(
                div()
                    .absolute()
                    .top(px(4.))
                    .right(px(4.))
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .child(self.more_button((more_key, ix), menu_target, theme, cx)),
            )
            .when_some(progress, |d, f| d.child(Self::progress_bar(f, theme)))
            .into_any_element()
    }

    fn render_tools_section(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let m = self.metrics;
        let tools = self.filtered_tools();
        let categories = self.config.categories.clone();
        let all = &self.config.tools;
        let total = all.len();
        let uncategorized = all
            .iter()
            .filter(|t| t.category.is_empty() || !categories.contains(&t.category))
            .count();
        let category_counts: Vec<usize> = categories
            .iter()
            .map(|c| all.iter().filter(|t| &t.category == c).count())
            .collect();
        let current = self.tool_filter.clone();

        // --- Category tabs -------------------------------------------------
        let mut tabs: Vec<AnyElement> = Vec::new();
        let tab = |ix: usize, label: String, count: String, filter: ToolFilter, drop: Option<TabDrop>, cx: &mut Context<Self>| {
            let selected = current == filter;
            let menu = match &filter {
                ToolFilter::Category(c) => Some(MenuTarget::Category(c.clone())),
                _ => None,
            };
            let category = match &filter {
                ToolFilter::Category(c) => Some(c.clone()),
                _ => None,
            };
            let el = div()
                .id(("category-tab", ix))
                .flex_none()
                .h(px(28.))
                .px(px(12.))
                .rounded_full()
                .border_1()
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(12.5))
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(theme.accent).border_color(theme.accent).text_color(theme.accent_text)
                })
                .when(!selected, |d| {
                    d.border_color(theme.border)
                        .bg(theme.panel)
                        .text_color(theme.text_muted)
                        .hover(|s| s.border_color(theme.border_strong).text_color(theme.text))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_filter(filter.clone(), cx)))
                .when_some(menu, |d, menu| {
                    d.on_mouse_down(MouseButton::Right, Self::open_menu_on_right_click(menu, cx))
                })
                // User categories can be dragged into a new order.
                .when_some(category.clone(), |d, name| {
                    let target = name.clone();
                    d.on_drag(DraggedCategory { name: name.into() }, |dragged, _, _, cx| {
                        cx.new(|_| DragPreview {
                            name: dragged.name.clone(),
                        })
                    })
                    .drag_over::<DraggedCategory>(|s, _, _, cx| {
                        let t = Theme::get(cx);
                        s.border_color(t.accent).bg(t.accent_soft).text_color(t.accent)
                    })
                    .on_drop(cx.listener(move |this, dragged: &DraggedCategory, _, cx| {
                        this.move_category(&dragged.name, &target, cx)
                    }))
                })
                .when_some(drop, |d, drop| {
                    d.drag_over::<DraggedTool>(|s, _, _, cx| {
                        let t = Theme::get(cx);
                        s.border_color(t.accent).bg(t.accent_soft).text_color(t.accent)
                    })
                    .on_drop(cx.listener(move |this, dragged: &DraggedTool, _, cx| match &drop {
                        TabDrop::Category(category) => this.set_tool_category(&dragged.id, category, cx),
                        TabDrop::Ring => this.add_to_ring(&dragged.id, cx),
                    }))
                })
                .child(label)
                .child(
                    div()
                        .text_size(px(11.))
                        .opacity(0.75)
                        .child(count),
                );
            el.into_any_element()
        };
        tabs.push(tab(0, "全部".into(), total.to_string(), ToolFilter::All, None, cx));
        let ring_count = all.iter().filter(|t| t.ring).count();
        tabs.push(tab(
            9999,
            "圆环".into(),
            format!("{ring_count}/{}", crate::ring::MAX_ITEMS),
            ToolFilter::Ring,
            Some(TabDrop::Ring),
            cx,
        ));
        for (ix, (category, n)) in categories.iter().zip(&category_counts).enumerate() {
            tabs.push(tab(
                ix + 1,
                category.clone(),
                n.to_string(),
                ToolFilter::Category(category.clone()),
                Some(TabDrop::Category(category.clone())),
                cx,
            ));
        }
        if uncategorized > 0 || current == ToolFilter::Uncategorized {
            tabs.push(tab(
                categories.len() + 1,
                "未分类".into(),
                uncategorized.to_string(),
                ToolFilter::Uncategorized,
                Some(TabDrop::Category(String::new())),
                cx,
            ));
        }
        tabs.push(
            div()
                .id("add-category")
                .flex_none()
                .h(px(28.))
                .px(px(10.))
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(12.5))
                .text_color(theme.text_muted)
                .cursor_pointer()
                .hover(|s| s.border_color(theme.accent).text_color(theme.accent))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_category_editor(None, window, cx)
                }))
                .child(icon("plus", 13., theme.text_muted))
                .child("分类")
                .into_any_element(),
        );

        // --- Square tiles --------------------------------------------------
        let mut tiles = Vec::new();
        for (ix, tool) in tools.iter().enumerate() {
            tiles.push(self.tool_tile(ix, tool, TileIn::Grid, theme, cx));
        }
        let prefill_category = match &self.tool_filter {
            ToolFilter::Category(c) => c.clone(),
            _ => String::new(),
        };
        let add_category = prefill_category.clone();
        let add_to_ring = self.tool_filter == ToolFilter::Ring && self.config.tools.iter().filter(|t| t.ring).count() < crate::ring::MAX_ITEMS;
        tiles.push(
            div()
                .id("add-tool-card")
                .group("add-tool-card")
                .h(px(m.tile_h))
                .rounded(px(14.))
                .border_1()
                .border_color(theme.border_strong)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .text_color(theme.text_muted)
                .cursor_pointer()
                .hover(|s| s.bg(theme.accent_soft).text_color(theme.accent).border_color(theme.accent))
                // Dropping a tile on the add card moves it to the end.
                .drag_over::<DraggedTool>(|s, _, _, cx| {
                    let t = Theme::get(cx);
                    s.bg(t.accent_soft).border_color(t.accent).text_color(t.accent)
                })
                .on_drop(cx.listener(|this, dragged: &DraggedTool, _, cx| this.move_tool(&dragged.id, None, cx)))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    let prefill = crate::config::Tool {
                        color: "pink".into(),
                        category: add_category.clone(),
                        ring: add_to_ring,
                        ..Default::default()
                    };
                    this.open_tool_editor(None, Some(prefill), window, cx)
                }))
                .child(icon_hover("plus", 20., theme.text_muted, "add-tool-card", theme.accent))
                .child(div().text_size(px(12.5)).child("添加工具"))
                .into_any_element(),
        );

        let header = self
            .section_header("grid", theme.text_muted, "全部工具", theme)
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text_faint)
                    .mr(px(8.))
                    .child(if self.tool_filter == ToolFilter::Ring {
                        if self.config.tools.iter().any(|t| t.ring) {
                            "按住 Alt 弹出的圆环会显示这里的工具（最多 8 个）"
                        } else {
                            "把图标拖到“圆环”标签加入；没选时圆环显示最常用的 8 个"
                        }
                    } else {
                        "拖动图标或分类标签可调整顺序，图标拖到分类标签即可归类"
                    }),
            );

        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(m.header_gap))
            .child(header)
            .child(
                div()
                    .id("category-tabs")
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(
                div()
                    .id("tool-grid")
                    .h(px(m.grid_h))
                    .overflow_y_scroll()
                    .track_scroll(&self.tools_scroll)
                    .child(
                        div()
                            .grid()
                            .grid_cols(m.tile_cols)
                            .gap(px(m.grid_gap))
                            .children(tiles),
                    ),
            )
    }

    fn panel_header(&self, icon_name: &'static str, title: &'static str, theme: &Theme) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(6.))
            .pb(px(8.))
            .child(icon(icon_name, 16., theme.text_muted))
            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(title))
            .child(div().flex_1())
    }

    fn empty_hint(text: &'static str, theme: &Theme) -> Div {
        div()
            .flex_1()
            .min_h(px(90.))
            .flex()
            .items_center()
            .justify_center()
            .px(px(12.))
            .text_size(px(12.5))
            .text_color(theme.text_faint)
            .child(text)
    }

    fn render_recent_panel(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let m = self.metrics;
        let recent = self.recent_items(m.recent_rows);
        let mut header = self.panel_header("clock", "最近使用", theme);
        if !recent.is_empty() {
            header = header.child(link("clear-history", "清空", theme).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| this.clear_history(cx),
            )));
        }
        let mut rows = Vec::new();
        for (ix, item) in recent.iter().enumerate() {
            let tile = self.tile(&item.visual, 26., 14., theme, cx);
            let target = item.target.clone();
            rows.push(
                div()
                    .id(("recent", ix))
                    .h(px(m.row_h))
                    .px(px(6.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.row_hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.activate(target.clone(), false, window, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        Self::open_menu_on_right_click(MenuTarget::History(item.key.clone()), cx),
                    )
                    .child(tile)
                    .child(div().flex_1().min_w_0().text_size(px(13.5)).truncate().child(item.name.clone()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(theme.text_faint)
                            .child(item.when.clone()),
                    ),
            );
        }
        panel(theme)
            .p(px(12.))
            .flex()
            .flex_col()
            .child(header)
            .when(rows.is_empty(), |d| d.child(Self::empty_hint("还没有使用记录，启动一个工具试试吧", theme)))
            .children(rows)
    }

    fn render_projects_panel(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let m = self.metrics;
        let projects: Vec<_> = self.config.projects.iter().take(m.recent_rows).cloned().collect();
        let header = self
            .panel_header("folder", "最近项目", theme)
            .child(link("open-project", "打开…", theme).on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| this.prompt_open_project(window, cx),
            )));
        let mut rows = Vec::new();
        for (ix, project) in projects.iter().enumerate() {
            let path = project.path.clone();
            rows.push(
                div()
                    .id(("project", ix))
                    .h(px(m.row_h + 8.))
                    .px(px(6.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.row_hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.activate(Target::Project(path.clone()), false, window, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        Self::open_menu_on_right_click(MenuTarget::Project(project.path.clone()), cx),
                    )
                    .child(icon("folder", 20., theme.folder))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(13.5)).truncate().child(project.name.clone()))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(theme.text_faint)
                                    .truncate()
                                    .child(project.path.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(theme.text_faint)
                            .child(relative_time(project.last_opened)),
                    ),
            );
        }
        panel(theme)
            .p(px(12.))
            .flex()
            .flex_col()
            .child(header)
            .when(rows.is_empty(), |d| {
                d.child(Self::empty_hint("把文件夹拖进窗口，或按 Ctrl+O 打开项目", theme))
            })
            .children(rows)
    }

    fn render_actions_panel(&mut self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let m = self.metrics;
        let mut rows = Vec::new();
        for (ix, action) in QuickAction::ALL.into_iter().enumerate() {
            rows.push(
                div()
                    .id(("action", ix))
                    .h(px(m.action_h))
                    .px(px(10.))
                    .rounded(px(9.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.input_bg)
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .hover(|s| s.border_color(theme.accent).bg(theme.accent_soft))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.quick_action(action, window, cx)
                    }))
                    .child(icon(action.icon(), 16., theme.text_muted))
                    .child(div().flex_1().text_size(px(13.5)).child(action.label()))
                    .child(
                        div()
                            .flex()
                            .gap(px(3.))
                            .children(action.keys().iter().map(|k| kbd(k, theme))),
                    ),
            );
        }
        panel(theme)
            .p(px(12.))
            .flex()
            .flex_col()
            .child(self.panel_header("zap", "快捷操作", theme))
            .child(div().flex().flex_col().gap(px(m.action_gap)).children(rows))
    }

    // -----------------------------------------------------------------------
    // Search results

    fn render_results(&mut self, theme: &Theme, mini: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let results = self.results.clone();
        let mut rows = Vec::new();
        for (ix, item) in results.iter().enumerate() {
            let selected = ix == self.selected;
            let tile = self.tile(&item.visual, 34., 18., theme, cx);
            let target = item.target.clone();
            let menu_target = match &item.target {
                Target::Tool(id) => Some(MenuTarget::Tool(id.clone())),
                Target::App(path) => Some(MenuTarget::App(path.clone())),
                Target::Project(path) => Some(MenuTarget::Project(path.clone())),
                Target::File { path, is_dir } => Some(MenuTarget::File {
                    path: path.clone(),
                    is_dir: *is_dir,
                }),
                _ => None,
            };
            rows.push(
                div()
                    .id(("result", ix))
                    .h(px(54.))
                    .px(px(10.))
                    .rounded(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.row_active))
                    .when(!selected, |d| d.hover(|s| s.bg(theme.row_hover)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.selected = ix;
                        this.activate(target.clone(), false, window, cx)
                    }))
                    .when_some(menu_target, |d, menu_target| {
                        d.on_mouse_down(MouseButton::Right, Self::open_menu_on_right_click(menu_target, cx))
                    })
                    .child(tile)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(item.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_muted)
                                    .truncate()
                                    .child(item.subtitle.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .px(px(8.))
                            .py(px(2.))
                            .rounded(px(6.))
                            .bg(if selected { theme.panel } else { theme.kbd_bg })
                            .text_size(px(11.5))
                            .text_color(theme.text_muted)
                            .child(item.badge),
                    )
                    .when(selected, |d| d.child(icon("enter", 15., theme.accent))),
            );
        }

        let hint = |keys: &'static str, label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(kbd(keys, theme))
                .child(label)
        };

        let container = if mini {
            div().flex_1().min_h_0().flex().flex_col()
        } else {
            div().flex_1().min_h_0().px(px(24.)).pb(px(22.)).flex().flex_col()
        };
        let surface = if mini { div() } else { panel(theme) };
        container.child(
            surface
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(
                    div()
                        .id("results")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.results_scroll)
                        .p(px(6.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .children(rows),
                )
                .child(
                    div()
                        .flex_none()
                        .h(px(40.))
                        .px(px(16.))
                        .border_t_1()
                        .border_color(theme.border)
                        .flex()
                        .items_center()
                        .gap(px(18.))
                        .text_size(px(12.))
                        .text_color(theme.text_faint)
                        .child(hint("↑↓", "选择"))
                        .child(hint("Enter", "打开"))
                        .when(!mini, |d| d.child(hint("Ctrl+Enter", "以管理员身份运行")))
                        .child(hint("Esc", "清空"))
                        .child(div().flex_1())
                        .child(format!("{} 个结果", results.len())),
                ),
        )
    }

    // -----------------------------------------------------------------------
    // @ai answer panel

    fn render_ai_panel(&mut self, theme: &Theme, mini: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some((provider, question)) = self.ai_request() else {
            return div().into_any_element();
        };
        let answer = self.current_answer().map(|a| (a.text.clone(), a.state.clone(), a.model.clone()));
        let streaming = matches!(answer, Some((_, AiState::Streaming, _)));
        let model = answer
            .as_ref()
            .map(|(_, _, m)| m.clone())
            .unwrap_or_else(|| provider.model.clone());
        let configured = !provider.api_key.trim().is_empty() && !provider.model.trim().is_empty();

        let small_button = |id: &'static str, icon_name: &'static str, label: &'static str, theme: &Theme| {
            div()
                .id(id)
                .h(px(26.))
                .px(px(10.))
                .rounded(px(7.))
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(12.))
                .text_color(theme.text_muted)
                .cursor_pointer()
                .hover(|s| s.bg(theme.row_hover).text_color(theme.accent))
                .child(icon(icon_name, 13., theme.text_muted))
                .child(label)
        };

        let header = div()
            .flex_none()
            .h(px(44.))
            .px(px(16.))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(8.))
            .child(icon("zap", 16., theme.accent))
            .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child(provider.name.clone()))
            .when(!model.is_empty(), |d| {
                d.child(
                    div()
                        .px(px(8.))
                        .py(px(1.))
                        .rounded(px(6.))
                        .bg(theme.kbd_bg)
                        .text_size(px(11.5))
                        .text_color(theme.text_muted)
                        .child(model.clone()),
                )
            })
            .child(div().flex_1())
            .when(streaming, |d| {
                d.child(small_button("ai-stop", "x", "停止", theme).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.stop_ai(cx);
                    },
                )))
            })
            .when_some(
                answer.as_ref().filter(|(text, _, _)| !streaming && !text.is_empty()).map(|(t, _, _)| t.clone()),
                |d, text| {
                    d.child(small_button("ai-copy", "copy", "复制回答", theme).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| this.copy_path(&text, cx),
                    )))
                },
            )
            .child(small_button("ai-settings", "settings", "设置", theme).on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| this.open_ai_settings(window, cx),
            )));

        let body: AnyElement = match &answer {
            None => {
                let (title, detail) = if !configured {
                    (
                        format!("还没有配置 {}", provider.name),
                        "点右上角“设置”，填写 API Key 并选择模型".to_string(),
                    )
                } else if question.is_empty() {
                    ("在 @ai 后面输入你的问题".to_string(), "例如：@ai 用 PowerShell 查看端口占用".to_string())
                } else {
                    (format!("按 Enter 向 {} 提问", provider.name), question.clone())
                };
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(4.))
                    .py(px(10.))
                    .child(div().text_size(px(14.)).text_color(theme.text).child(title))
                    .child(div().text_size(px(12.)).text_color(theme.text_faint).truncate().child(detail))
                    .into_any_element()
            }
            Some((text, state, _)) => {
                let mut blocks = crate::markdown::render(text, None, theme);
                if text.is_empty() && streaming {
                    blocks.push(
                        div()
                            .text_size(px(13.))
                            .text_color(theme.text_faint)
                            .child("正在思考…")
                            .into_any_element(),
                    );
                }
                if let AiState::Failed(err) = state {
                    blocks.push(
                        div()
                            .mt(px(6.))
                            .p(px(10.))
                            .rounded(px(8.))
                            .bg(theme.danger.opacity(0.1))
                            .text_size(px(12.5))
                            .text_color(theme.danger)
                            .child(err.clone())
                            .into_any_element(),
                    );
                }
                div()
                    .id("ai-answer")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.ai_scroll)
                    .px(px(18.))
                    .py(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .mb(px(6.))
                            .text_size(px(12.5))
                            .text_color(theme.text_faint)
                            .child(format!("问：{question}")),
                    )
                    .children(blocks)
                    .into_any_element()
            }
        };

        let hint = |keys: &'static str, label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(kbd(keys, theme))
                .child(label)
        };
        let footer = div()
            .flex_none()
            .h(px(40.))
            .px(px(16.))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(18.))
            .text_size(px(12.))
            .text_color(theme.text_faint)
            .child(hint("Enter", if answer.is_some() { "重新提问" } else { "提问" }))
            .child(hint("Esc", if streaming { "停止" } else { "清空" }))
            .child(div().flex_1())
            .child("@deepseek / @claude / @kimi… 可直接指定服务商");

        let container = if mini {
            div().flex_1().min_h_0().flex().flex_col()
        } else {
            div().flex_1().min_h_0().px(px(24.)).pb(px(22.)).flex().flex_col()
        };
        let surface = if mini { div() } else { panel(theme) };
        container
            .child(
                surface
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(header)
                    .child(div().flex_1().min_h_0().child(body))
                    .child(footer),
            )
            .into_any_element()
    }

    // -----------------------------------------------------------------------
    // Mini search bar

    fn render_mini(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let has_results = !self.search_text.trim().is_empty();
        let results = if self.ai_request().is_some() {
            Some(self.render_ai_panel(theme, true, cx))
        } else {
            has_results.then(|| self.render_results(theme, true, cx).into_any_element())
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("mini-bar")
                    .h(px(Self::MINI_BAR))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .pl(px(16.))
                    .pr(px(12.))
                    .window_control_area(WindowControlArea::Drag)
                    .child(img(self.logo.clone()).size(px(32.)).flex_none())
                    .child(
                        div()
                            .id("mini-input")
                            .occlude()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .text_size(px(18.))
                            .line_height(px(26.))
                            .child(self.search.clone()),
                    )
                    .child(
                        div()
                            .id("mini-restore")
                            .occlude()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .h(px(32.))
                            .px(px(10.))
                            .rounded(px(9.))
                            .text_size(px(12.5))
                            .text_color(theme.text_muted)
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.row_hover).text_color(theme.accent))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.restore_full(cx);
                                this.show(window, cx);
                            }))
                            .child(icon("maximize", 15., theme.text_muted))
                            .child("还原窗口"),
                    ),
            )
            .when_some(results, |d, results| {
                d.child(div().flex_none().h(px(1.)).bg(theme.border)).child(results)
            })
            .into_any_element()
    }

    // -----------------------------------------------------------------------
    // Context menu

    fn menu_entries(&self, target: &MenuTarget) -> Vec<(&'static str, String, MenuCommand, bool)> {
        match target {
            MenuTarget::File { path, is_dir } => {
                let shown = path.display().to_string();
                let mut entries = vec![
                    (
                        "rocket",
                        "打开".to_string(),
                        MenuCommand::Open(Target::File {
                            path: path.clone(),
                            is_dir: *is_dir,
                        }),
                        false,
                    ),
                    ("eye", "快速预览".into(), MenuCommand::Preview(path.clone()), false),
                    ("folder-open", "打开所在文件夹".into(), MenuCommand::RevealPath(shown.clone()), false),
                    ("copy", "复制路径".into(), MenuCommand::CopyPath(shown), false),
                ];
                entries.push(if *is_dir {
                    ("folder", "添加为项目".into(), MenuCommand::AddProject(path.clone()), false)
                } else {
                    ("plus", "添加到工具".into(), MenuCommand::AddAppAsTool(path.clone()), false)
                });
                entries
            }
            MenuTarget::Category(name) => vec![
                ("edit", "重命名…".into(), MenuCommand::RenameCategory(name.clone()), false),
                ("trash", "删除分类".into(), MenuCommand::DeleteCategory(name.clone()), true),
            ],
            MenuTarget::Tool(id) => {
                let tool = self.config.tools.iter().find(|t| &t.id == id);
                let favorite = tool.is_some_and(|t| t.favorite);
                let in_ring = tool.is_some_and(|t| t.ring);
                let download = tool.filter(|t| !t.download_url.is_empty()).map(|t| {
                    if self.downloads.contains_key(id) {
                        ("x", "取消下载".to_string(), MenuCommand::CancelDownload(id.clone()), false)
                    } else if Self::needs_download(t) {
                        ("download", "下载".to_string(), MenuCommand::Download(id.clone()), false)
                    } else {
                        ("refresh", "重新下载（更新）".to_string(), MenuCommand::Redownload(id.clone()), false)
                    }
                });
                let mut entries = vec![
                    ("rocket", "打开".into(), MenuCommand::Open(Target::Tool(id.clone())), false),
                    ("shield", "以管理员身份运行".into(), MenuCommand::OpenAdmin(Target::Tool(id.clone())), false),
                    (
                        if favorite { "star" } else { "star-fill" },
                        if favorite { "取消收藏" } else { "收藏" }.into(),
                        MenuCommand::ToggleFavorite(id.clone()),
                        false,
                    ),
                    (
                        "circle",
                        if in_ring { "移出圆环" } else { "加入圆环" }.into(),
                        MenuCommand::ToggleRing(id.clone()),
                        false,
                    ),
                    ("edit", "编辑…".into(), MenuCommand::EditTool(id.clone()), false),
                    ("folder-open", "打开文件位置".into(), MenuCommand::RevealTool(id.clone()), false),
                ];
                entries.extend(download);
                entries.push(("trash", "删除".into(), MenuCommand::RemoveTool(id.clone()), true));
                entries
            }
            MenuTarget::App(path) => vec![
                ("rocket", "打开".into(), MenuCommand::Open(Target::App(path.clone())), false),
                ("shield", "以管理员身份运行".into(), MenuCommand::OpenAdmin(Target::App(path.clone())), false),
                ("plus", "添加到工具".into(), MenuCommand::AddAppAsTool(path.clone()), false),
                ("folder-open", "打开文件位置".into(), MenuCommand::RevealPath(path.display().to_string()), false),
            ],
            MenuTarget::Project(path) => {
                let mut entries = vec![
                    ("folder-open", "在资源管理器中打开".into(), MenuCommand::Project(path.clone(), ProjectOpener::Explorer), false),
                    ("terminal", "在终端中打开".into(), MenuCommand::Project(path.clone(), ProjectOpener::Terminal), false),
                ];
                if self.code_available {
                    entries.push(("code", "用 VS Code 打开".into(), MenuCommand::Project(path.clone(), ProjectOpener::VsCode), false));
                }
                entries.push(("trash", "从列表中移除".into(), MenuCommand::RemoveProject(path.clone()), true));
                entries
            }
            MenuTarget::History(key) => vec![("trash", "从最近使用中移除".into(), MenuCommand::RemoveHistory(key.clone()), true)],
        }
    }

    fn render_menu(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let menu = self.menu.as_ref()?;
        let position = menu.position;
        let entries = self.menu_entries(&menu.target);
        let items = entries.into_iter().enumerate().map(|(ix, (icon_name, label, command, danger))| {
            let color = if danger { theme.danger } else { theme.text };
            div()
                .id(("menu-item", ix))
                .h(px(32.))
                .px(px(10.))
                .rounded(px(7.))
                .flex()
                .items_center()
                .gap(px(10.))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(color)
                .hover(|s| s.bg(theme.row_hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.run_menu_command(command.clone(), window, cx)
                }))
                .child(icon(icon_name, 15., if danger { theme.danger } else { theme.text_muted }))
                .child(label)
        });
        let surface = if theme.dark { rgb(0x2e272e) } else { rgb(0xffffff) };
        Some(
            deferred(
                anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .id("context-menu")
                            .occlude()
                            .min_w(px(196.))
                            .p(px(5.))
                            .bg(surface)
                            .rounded(px(11.))
                            .border_1()
                            .border_color(theme.border_strong)
                            .shadow(vec![BoxShadow {
                                color: gpui::hsla(0., 0., 0., if theme.dark { 0.45 } else { 0.14 }),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(24.),
                                spread_radius: px(0.),
                            }])
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.menu = None;
                                cx.notify();
                            }))
                            .children(items),
                    ),
            )
            .with_priority(1),
        )
    }

    fn run_menu_command(&mut self, command: MenuCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        match command {
            MenuCommand::Open(target) => self.activate(target, false, window, cx),
            MenuCommand::OpenAdmin(target) => self.activate(target, true, window, cx),
            MenuCommand::ToggleFavorite(id) => self.toggle_favorite(&id, cx),
            MenuCommand::ToggleRing(id) => self.toggle_ring(&id, cx),
            MenuCommand::EditTool(id) => self.open_tool_editor(Some(id), None, window, cx),
            MenuCommand::RevealTool(id) => {
                if let Some(target) = self.config.tools.iter().find(|t| t.id == id).map(|t| t.target.clone()) {
                    self.reveal(&target, cx);
                }
            }
            MenuCommand::RemoveTool(id) => self.remove_tool(&id, cx),
            MenuCommand::RenameCategory(name) => self.open_category_editor(Some(name), window, cx),
            MenuCommand::DeleteCategory(name) => self.delete_category(&name, cx),
            MenuCommand::CopyPath(path) => self.copy_path(&path, cx),
            MenuCommand::Preview(path) => self.open_preview(path, None, cx),
            MenuCommand::AddProject(path) => self.add_project(&path, cx),
            MenuCommand::Download(id) => self.start_download(&id, false, cx),
            MenuCommand::Redownload(id) => self.redownload(&id, cx),
            MenuCommand::CancelDownload(id) => self.cancel_download(&id),
            MenuCommand::RevealPath(path) => self.reveal(&path, cx),
            MenuCommand::AddAppAsTool(path) => self.add_app_as_tool(&path, cx),
            MenuCommand::Project(path, opener) => self.open_project_in(path, opener, cx),
            MenuCommand::RemoveProject(path) => self.remove_project(&path, cx),
            MenuCommand::RemoveHistory(key) => self.remove_history(&key, cx),
        }
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Toast

    fn render_toast(&self, theme: &Theme) -> Option<impl IntoElement + use<>> {
        let toast = self.toast.as_ref()?;
        let (bg, fg): (Hsla, Hsla) = if toast.error {
            (theme.danger, gpui::white())
        } else if theme.dark {
            (rgb(0xf3ebf1).into(), rgb(0x2b2230).into())
        } else {
            (rgb(0x2b2230).into(), gpui::white())
        };
        Some(
            div()
                .absolute()
                .bottom(px(22.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .max_w(px(640.))
                        .px(px(16.))
                        .py(px(9.))
                        .rounded(px(11.))
                        .bg(bg)
                        .text_color(fg)
                        .text_size(px(13.))
                        .shadow(soft_shadow(theme))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon(if toast.error { "info" } else { "check" }, 15., fg))
                        .child(toast.message.clone()),
                ),
        )
    }
}

/// Minimal Markdown for AI answers: headings, bullets, fenced code blocks

/// Icon that switches color while its `group` ancestor is hovered.
pub fn icon_hover(name: &str, size: f32, color: Hsla, group: &'static str, hover: Hsla) -> gpui::Svg {
    icon(name, size, color).group_hover(group, move |s| s.text_color(hover))
}

#[derive(Clone, Debug)]
pub enum MenuCommand {
    Open(Target),
    OpenAdmin(Target),
    ToggleFavorite(String),
    ToggleRing(String),
    EditTool(String),
    RevealTool(String),
    RemoveTool(String),
    RenameCategory(String),
    DeleteCategory(String),
    CopyPath(String),
    Preview(std::path::PathBuf),
    AddProject(std::path::PathBuf),
    Download(String),
    Redownload(String),
    CancelDownload(String),
    RevealPath(String),
    AddAppAsTool(std::path::PathBuf),
    Project(String, ProjectOpener),
    RemoveProject(String),
    RemoveHistory(String),
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let viewport = window.viewport_size();
        let favorites = self.config.tools.iter().filter(|t| t.favorite).count();
        self.metrics = Metrics::for_viewport(viewport.width.into(), viewport.height.into(), favorites);
        let searching = !self.search_text.trim().is_empty();

        let content = if self.mini {
            self.render_mini(&theme, cx)
        } else {
            let body = if self.ai_request().is_some() {
                self.render_ai_panel(&theme, false, cx)
            } else if searching {
                self.render_results(&theme, false, cx).into_any_element()
            } else {
                self.render_dashboard(&theme, cx).into_any_element()
            };
            let titlebar = self.render_titlebar(&theme, cx);
            let search = self.render_search(&theme, window, cx);
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(titlebar)
                .child(search)
                .child(body)
                .into_any_element()
        };
        let dialog = self.render_dialog(&theme, window, cx);
        let menu = self.render_menu(&theme, cx);
        let toast = self.render_toast(&theme);
        // The wallpaper, under a veil of the window colour so the content stays readable.
        let wallpaper = self.wallpaper_image.clone().map(|image| {
            let settings = &self.config.settings.wallpaper;
            let fit = match settings.fit {
                crate::config::WallpaperFit::Cover => gpui::ObjectFit::Cover,
                crate::config::WallpaperFit::Contain => gpui::ObjectFit::Contain,
                crate::config::WallpaperFit::Stretch => gpui::ObjectFit::Fill,
            };
            let veil = gpui::Hsla { a: settings.veil.min(95) as f32 / 100., ..theme.window_bg };
            div()
                .absolute()
                .inset_0()
                .overflow_hidden()
                .child(img(image).size_full().object_fit(fit))
                .child(div().absolute().inset_0().bg(veil))
        });

        div()
            .id("launcher-root")
            .key_context("Launcher")
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::confirm_admin))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::focus_next_field))
            .on_action(cx.listener(Self::focus_prev_field))
            .on_action(cx.listener(|this, _: &NewTerminal, window, cx| {
                this.quick_action(QuickAction::NewTerminal, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenProject, window, cx| {
                this.quick_action(QuickAction::OpenProject, window, cx)
            }))
            .on_action(cx.listener(|this, _: &TakeScreenshot, window, cx| {
                this.quick_action(QuickAction::Screenshot, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ScanCode, window, cx| {
                this.quick_action(QuickAction::Scan, window, cx)
            }))
            .on_action(cx.listener(|this, _: &QuickNote, window, cx| {
                this.quick_action(QuickAction::Note, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                if this.dialog.is_none() {
                    this.open_settings(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &AddTool, window, cx| {
                if this.dialog.is_none() {
                    this.open_tool_editor(None, None, window, cx)
                }
            }))
            .on_drop(cx.listener(Self::on_drop_paths))
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.window_bg)
            .text_color(theme.text)
            .text_size(px(14.))
            .children(wallpaper)
            .child(content)
            .children(dialog)
            .children(menu)
            .children(toast)
    }
}
