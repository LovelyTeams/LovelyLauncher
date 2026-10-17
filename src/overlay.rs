//! 取色器 and 屏幕标尺: a still picture of the screen under the mouse, laid over
//! everything, to pick a colour from or to measure distances in screen pixels.

use std::{sync::Arc, time::Duration};

use gpui::{
    AnyElement, App, ClipboardItem, Context, CursorStyle, FocusHandle, Focusable, FontWeight, Hsla, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, RenderImage, WeakEntity,
    Window, actions, div, img, prelude::*, px, rgb,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{launcher::Launcher, win};

actions!(overlay, [CloseOverlay, NudgeLeft, NudgeRight, NudgeUp, NudgeDown]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Overlay");
    cx.bind_keys([
        KeyBinding::new("escape", CloseOverlay, ctx),
        KeyBinding::new("left", NudgeLeft, ctx),
        KeyBinding::new("right", NudgeRight, ctx),
        KeyBinding::new("up", NudgeUp, ctx),
        KeyBinding::new("down", NudgeDown, ctx),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// 取色器
    Color,
    /// 屏幕标尺
    Ruler,
}

/// What was captured: the monitor's rectangle on the virtual screen and its pixels.
pub struct Shot {
    pub rect: (i32, i32, i32, i32),
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Shot {
    fn pixel(&self, x: i64, y: i64) -> Option<[u8; 3]> {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let p = self.bgra.get(i..i + 3)?;
        Some([p[2], p[1], p[0]])
    }
}

/// Side of the magnified square, in screen pixels.
const LOUPE: i64 = 11;
/// Size of one magnified pixel.
const CELL: f32 = 11.;
/// Colours closer than this (per channel) count as the same surface when measuring.
const TOLERANCE: u8 = 18;

pub struct Overlay {
    mode: Mode,
    shot: Arc<Shot>,
    image: Arc<RenderImage>,
    /// Mouse position in screen pixels relative to the monitor.
    mouse: Option<(i64, i64)>,
    /// Ruler: where a drag started / the rectangle last measured (screen pixels).
    drag: Option<((i64, i64), (i64, i64))>,
    dragging: bool,
    /// Shown just before closing (colour picker) or after copying (ruler).
    copied: Option<String>,
    copied_seq: u64,
    focus: FocusHandle,
    #[allow(dead_code)]
    launcher: WeakEntity<Launcher>,
}

impl Overlay {
    pub fn new(mode: Mode, shot: Shot, launcher: WeakEntity<Launcher>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hwnd = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
            _ => None,
        };
        let image = image::RgbaImage::from_raw(shot.width, shot.height, shot.bgra.clone())
            .map(|img| Arc::new(RenderImage::new(vec![image::Frame::new(img)])))
            .unwrap_or_else(|| Arc::new(RenderImage::new(Vec::new())));
        if let Some(raw) = hwnd {
            let rect = shot.rect;
            // Not synchronously: moving the window re-enters GPUI.
            cx.spawn(async move |_, _| win::cover_rect(raw, rect)).detach();
        }
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mouse = win::cursor_position().map(|(x, y)| ((x - shot.rect.0) as i64, (y - shot.rect.1) as i64));
        Self {
            mode,
            shot: Arc::new(shot),
            image,
            mouse,
            drag: None,
            dragging: false,
            copied: None,
            copied_seq: 0,
            focus,
            launcher,
        }
    }

    fn close(&mut self, window: &mut Window) {
        window.remove_window();
    }

    fn to_screen(&self, p: Point<Pixels>, window: &Window) -> (i64, i64) {
        let scale = window.scale_factor();
        ((f32::from(p.x) * scale).floor() as i64, (f32::from(p.y) * scale).floor() as i64)
    }

    fn nudge(&mut self, dx: i32, dy: i32) {
        if let Some((x, y)) = win::cursor_position() {
            win::set_cursor_position(x + dx, y + dy);
        }
    }

    /// Put `text` on the clipboard and say so for a moment; the colour picker then closes.
    fn copy(&mut self, text: String, close: bool, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.copied = Some(text);
        self.copied_seq += 1;
        let seq = self.copied_seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(if close { 550 } else { 1400 })).await;
            this.update(cx, |this, cx| {
                if this.copied_seq == seq {
                    this.copied = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn on_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = self.to_screen(e.position, window);
        self.mouse = Some((x, y));
        match self.mode {
            Mode::Color => {
                if let Some([r, g, b]) = self.shot.pixel(x, y) {
                    let text = if e.modifiers.shift {
                        format!("rgb({r}, {g}, {b})")
                    } else {
                        format!("#{r:02X}{g:02X}{b:02X}")
                    };
                    self.copy(text, true, cx);
                    // Closed for good shortly after; ignore further clicks.
                    let seq = self.copied_seq;
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(Duration::from_millis(600)).await;
                        this.update_in(cx, |this, window, _| {
                            if this.copied_seq == seq {
                                this.close(window);
                            }
                        })
                        .ok();
                    })
                    .detach();
                }
            }
            Mode::Ruler => {
                self.drag = Some(((x, y), (x, y)));
                self.dragging = true;
            }
        }
        cx.notify();
    }

    fn on_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = self.to_screen(e.position, window);
        self.mouse = Some((x, y));
        if self.dragging
            && let Some((start, _)) = self.drag
        {
            self.drag = Some((start, (x, y)));
        }
        cx.notify();
    }

    fn on_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::Ruler || !self.dragging {
            return;
        }
        self.dragging = false;
        let text = match self.drag {
            // A drag: the rectangle.
            Some((a, b)) if a != b => {
                let (w, h) = ((a.0 - b.0).abs() + 1, (a.1 - b.1).abs() + 1);
                format!("{w} × {h}")
            }
            // A click: the gaps measured around the pointer.
            _ => {
                self.drag = None;
                match self.mouse.and_then(|(x, y)| self.spacing(x, y)) {
                    Some(s) => format!("{} × {}", s.right - s.left + 1, s.bottom - s.top + 1),
                    None => return,
                }
            }
        };
        self.copy(text, false, cx);
    }

    /// How far the surface under (`x`, `y`) reaches in each direction.
    fn spacing(&self, x: i64, y: i64) -> Option<Spacing> {
        let base = self.shot.pixel(x, y)?;
        let same = |p: Option<[u8; 3]>| p.is_some_and(|p| (0..3).all(|i| p[i].abs_diff(base[i]) <= TOLERANCE));
        let (mut left, mut right, mut top, mut bottom) = (x, x, y, y);
        while same(self.shot.pixel(left - 1, y)) {
            left -= 1;
        }
        while same(self.shot.pixel(right + 1, y)) {
            right += 1;
        }
        while same(self.shot.pixel(x, top - 1)) {
            top -= 1;
        }
        while same(self.shot.pixel(x, bottom + 1)) {
            bottom += 1;
        }
        Some(Spacing { left, right, top, bottom })
    }

    fn render_loupe(&self, x: i64, y: i64, scale: f32, size: (f32, f32)) -> AnyElement {
        let half = LOUPE / 2;
        let grid = LOUPE as f32 * CELL;
        let mut cells = Vec::new();
        for dy in -half..=half {
            for dx in -half..=half {
                let color = match self.shot.pixel(x + dx, y + dy) {
                    Some([r, g, b]) => rgb(u32::from_be_bytes([0, r, g, b])).into(),
                    None => gpui::hsla(0., 0., 0.12, 1.),
                };
                cells.push(
                    div()
                        .absolute()
                        .left(px((dx + half) as f32 * CELL))
                        .top(px((dy + half) as f32 * CELL))
                        .size(px(CELL))
                        .bg(color),
                );
            }
        }
        let [r, g, b] = self.shot.pixel(x, y).unwrap_or([0, 0, 0]);
        let center: Hsla = rgb(u32::from_be_bytes([0, r, g, b])).into();
        let hex = format!("#{r:02X}{g:02X}{b:02X}");
        // Beside the pointer, flipped near the right / bottom edge.
        let (px_, py_) = (x as f32 / scale, y as f32 / scale);
        // Wide enough for the rgb() line under the magnified pixels.
        let (w, h) = (grid + 56., grid + 74.);
        let left = if px_ + 24. + w > size.0 { px_ - 24. - w } else { px_ + 24. };
        let top = if py_ + 24. + h > size.1 { py_ - 24. - h } else { py_ + 24. };
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(w))
            .p(px(8.))
            .rounded(px(12.))
            .bg(gpui::hsla(0., 0., 0.1, 0.92))
            .border_1()
            .border_color(gpui::hsla(0., 0., 1., 0.25))
            .shadow_lg()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .relative()
                    .size(px(grid))
                    .rounded(px(6.))
                    .overflow_hidden()
                    .children(cells)
                    .child(
                        div()
                            .absolute()
                            .left(px(half as f32 * CELL - 1.))
                            .top(px(half as f32 * CELL - 1.))
                            .size(px(CELL + 2.))
                            .border_2()
                            .border_color(gpui::white()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_none().size(px(28.)).rounded(px(6.)).bg(center).border_1().border_color(gpui::hsla(0., 0., 1., 0.4)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .text_color(gpui::white())
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).font_family("Consolas").child(hex))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .font_family("Consolas")
                                    .text_color(gpui::hsla(0., 0., 1., 0.7))
                                    .child(format!("rgb({r}, {g}, {b})")),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_ruler(&self, scale: f32) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let ink: Hsla = rgb(0x00d8ff).into();
        let line = |l: f32, t: f32, w: f32, h: f32| {
            // A dark halo under a bright line, readable on any background.
            div()
                .absolute()
                .left(px(l - 1.))
                .top(px(t - 1.))
                .w(px(w + 2.))
                .h(px(h + 2.))
                .bg(gpui::hsla(0., 0., 0., 0.45))
                .child(div().absolute().left(px(1.)).top(px(1.)).w(px(w)).h(px(h)).bg(ink))
                .into_any_element()
        };
        let label = |l: f32, t: f32, text: String| {
            div()
                .absolute()
                .left(px(l))
                .top(px(t))
                .px(px(7.))
                .py(px(2.))
                .rounded(px(6.))
                .bg(gpui::hsla(0., 0., 0.1, 0.9))
                .text_color(gpui::white())
                .text_size(px(12.5))
                .font_family("Consolas")
                .whitespace_nowrap()
                .child(text)
                .into_any_element()
        };
        let to = |v: i64| v as f32 / scale;
        match self.drag.filter(|(a, b)| a != b) {
            Some((a, b)) => {
                let (l, r) = (a.0.min(b.0), a.0.max(b.0));
                let (t, bt) = (a.1.min(b.1), a.1.max(b.1));
                let (w, h) = (r - l + 1, bt - t + 1);
                out.push(
                    div()
                        .absolute()
                        .left(px(to(l)))
                        .top(px(to(t)))
                        .w(px(to(w)))
                        .h(px(to(h)))
                        .border_1()
                        .border_color(ink)
                        .bg(ink.opacity(0.14))
                        .into_any_element(),
                );
                out.push(label(to(l), to(bt + 1) + 6., format!("{w} × {h}")));
            }
            None => {
                if let Some(s) = self.mouse.and_then(|(x, y)| self.spacing(x, y)) {
                    let (x, y) = self.mouse.unwrap_or_default();
                    let (w, h) = (s.right - s.left + 1, s.bottom - s.top + 1);
                    // Horizontal and vertical extent, with end caps.
                    out.push(line(to(s.left), to(y), to(w), 1.));
                    out.push(line(to(x), to(s.top), 1., to(h)));
                    out.push(line(to(s.left), to(y) - 5., 1., 11.));
                    out.push(line(to(s.right + 1) - 1., to(y) - 5., 1., 11.));
                    out.push(line(to(x) - 5., to(s.top), 11., 1.));
                    out.push(line(to(x) - 5., to(s.bottom + 1) - 1., 11., 1.));
                    out.push(label(to(x) + 12., to(y) + 12., format!("{w} × {h}")));
                }
            }
        }
        out
    }
}

struct Spacing {
    left: i64,
    right: i64,
    top: i64,
    bottom: i64,
}

impl Focusable for Overlay {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = window.scale_factor();
        let viewport = window.viewport_size();
        let size = (f32::from(viewport.width), f32::from(viewport.height));
        let accent: Hsla = crate::theme::Theme::get(cx).accent;
        let hint = match self.mode {
            Mode::Color => "单击复制颜色（HEX）· Shift + 单击复制 RGB · 方向键微调 · Esc 退出",
            Mode::Ruler => "移动鼠标量间距 · 拖动量矩形 · 松开鼠标复制尺寸 · 方向键微调 · Esc 退出",
        };
        let overlay = match (self.mode, self.mouse) {
            (Mode::Color, Some((x, y))) => vec![self.render_loupe(x, y, scale, size)],
            (Mode::Ruler, _) => self.render_ruler(scale),
            _ => Vec::new(),
        };
        div()
            .key_context("Overlay")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CloseOverlay, window, _| this.close(window)))
            .on_action(cx.listener(|this, _: &NudgeLeft, _, _| this.nudge(-1, 0)))
            .on_action(cx.listener(|this, _: &NudgeRight, _, _| this.nudge(1, 0)))
            .on_action(cx.listener(|this, _: &NudgeUp, _, _| this.nudge(0, -1)))
            .on_action(cx.listener(|this, _: &NudgeDown, _, _| this.nudge(0, 1)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, _: &MouseDownEvent, window, _| this.close(window)))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(gpui::black())
            .cursor(CursorStyle::Crosshair)
            .child(
                img(self.image.clone())
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(px(self.shot.width as f32 / scale))
                    .h(px(self.shot.height as f32 / scale)),
            )
            .children(overlay)
            .child(
                div().absolute().top(px(18.)).left_0().right_0().flex().justify_center().child(
                    div()
                        .px(px(14.))
                        .py(px(7.))
                        .rounded_full()
                        .bg(gpui::hsla(0., 0., 0.1, 0.85))
                        .text_color(gpui::white())
                        .text_size(px(12.5))
                        .child(hint),
                ),
            )
            .when_some(self.copied.clone(), |d, text| {
                d.child(
                    div().absolute().bottom(px(48.)).left_0().right_0().flex().justify_center().child(
                        div()
                            .px(px(16.))
                            .py(px(9.))
                            .rounded(px(10.))
                            .bg(accent)
                            .text_color(gpui::white())
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("已复制 {text}")),
                    ),
                )
            })
    }
}
