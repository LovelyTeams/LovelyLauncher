//! Hold Alt: a springy ring of favourite tools pops up in the bottom-right corner.
//! Point at an icon and let go of Alt (or click it) to open it.
//!
//! Everything moves on damped springs, stepped once per frame: the glassy ring pops
//! past full size and settles, squashes and stretches like jelly, icons pop in one
//! after another, and a gooey blob oozes to whichever icon is pointed at (its head
//! is quick, its tail slow, so it stretches thin and snaps back), with the icon
//! itself pulled a little towards the pointer.

use std::{
    f32::consts::{FRAC_PI_2, TAU},
    time::Instant,
};

use std::sync::Arc;

use gpui::{
    AnyElement, BoxShadow, Context, FontWeight, Hsla, IntoElement, MouseButton, MouseDownEvent, Render,
    RenderImage, Rgba, SharedString, WeakEntity, Window, div, hsla, img, linear_color_stop, linear_gradient,
    point, prelude::*, px,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{
    launcher::{Launcher, Visual},
    theme::Theme,
    win,
};

/// Window size (logical px); the ring sits in the middle of it.
pub const SIZE: f32 = 400.;
/// Gap between the window and the corner of the work area.
pub const MARGIN: f32 = 12.;
pub const MAX_ITEMS: usize = 8;
const CENTER: f32 = SIZE / 2.;
const OUTER: f32 = 150.;
const BAND: f32 = 68.;
const MID: f32 = OUTER - BAND / 2.;
/// The centre disc slightly overlaps the band, so no gap shows the desktop through.
const INNER: f32 = OUTER - BAND + 3.;
const TILE: f32 = 44.;
/// Pointer this close to the centre selects nothing (a way to cancel).
const DEAD_ZONE: f32 = 38.;
/// Pointer further away than this selects nothing either.
const REACH: f32 = OUTER + 90.;
/// Delay between icons popping in.
const STAGGER: f32 = 0.035;

pub struct RingItem {
    pub id: String,
    pub name: SharedString,
    pub visual: Visual,
}

/// A damped spring, stepped in small fixed increments so it stays stable at any frame rate.
#[derive(Clone, Copy)]
struct Spring {
    x: f32,
    v: f32,
    target: f32,
    stiffness: f32,
    damping: f32,
}

impl Spring {
    const fn new(x: f32, stiffness: f32, damping: f32) -> Self {
        Self { x, v: 0., target: x, stiffness, damping }
    }

    /// A spring by its damping ratio: 1 settles without overshoot, lower bounces more.
    fn bouncy(x: f32, stiffness: f32, zeta: f32) -> Self {
        Self::new(x, stiffness, 2. * zeta * stiffness.sqrt())
    }

    fn step(&mut self, dt: f32) {
        let mut left = dt;
        while left > 0. {
            let h = left.min(1. / 240.);
            let a = -self.stiffness * (self.x - self.target) - self.damping * self.v;
            self.v += a * h;
            self.x += self.v * h;
            left -= h;
        }
    }
}

/// How bouncy the ring is: 0 off, 1 gentle, 2 standard, 3 wild (Settings → 通用).
#[derive(Clone, Copy)]
struct Bounce {
    /// Damping ratios for the pop-in, the icons, and the goo's head.
    pop: f32,
    icon: f32,
    head: f32,
    /// How hard the jelly is kicked into squash and stretch, and how long it wobbles.
    kick: f32,
    wobble: f32,
}

impl Bounce {
    fn level(level: u8) -> Self {
        match level {
            0 => Self { pop: 1., icon: 1., head: 1., kick: 0., wobble: 1. },
            1 => Self { pop: 0.6, icon: 0.6, head: 0.75, kick: 0.5, wobble: 0.4 },
            3 => Self { pop: 0.25, icon: 0.24, head: 0.4, kick: 1.7, wobble: 0.14 },
            _ => Self { pop: 0.4, icon: 0.37, head: 0.59, kick: 1., wobble: 0.25 },
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Hidden,
    Open,
    Closing,
}

pub struct Ring {
    launcher: WeakEntity<Launcher>,
    hwnd: Option<isize>,
    items: Vec<RingItem>,
    phase: Phase,
    opened: Instant,
    last_frame: Instant,
    /// Overall size: pops past 1 and settles.
    scale: Spring,
    /// Squash and stretch: above 0 is wider than tall.
    squash: Spring,
    /// Each icon's size.
    icons: Vec<Spring>,
    /// The gooey highlight: where its head and its lagging tail are on the ring
    /// (radians), and how big it is (0–1).
    head: Spring,
    tail: Spring,
    bubble: Spring,
    /// The pointed-at icon leans towards the pointer by this much (logical px).
    pull_x: Spring,
    pull_y: Spring,
    /// The centre's picture pops when the selection changes.
    centre_pop: Spring,
    selected: Option<usize>,
    bounce: Bounce,
    /// The app's mascot, shown in the centre while nothing is pointed at.
    logo: Option<Arc<RenderImage>>,
}

/// `a` blended towards `b` by `t`, as an opaque colour.
fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    Hsla::from(Rgba { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: 1. })
}

/// Quick, slightly bouncy.
const HEAD: (f32, f32) = (420., 24.);
/// Slow and heavily damped: the viscous part.
const TAIL: (f32, f32) = (95., 18.);
/// How strongly the pointed-at icon sticks to the pointer, and how far it may lean.
const PULL: f32 = 0.22;
const MAX_PULL: f32 = 14.;

impl Ring {
    pub fn new(launcher: WeakEntity<Launcher>, window: &mut Window) -> Self {
        let hwnd = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
            _ => None,
        };
        Self {
            launcher,
            hwnd,
            items: Vec::new(),
            phase: Phase::Hidden,
            opened: Instant::now(),
            last_frame: Instant::now(),
            scale: Spring::new(0., 260., 13.),
            squash: Spring::new(0., 200., 7.),
            icons: Vec::new(),
            head: Spring::new(-FRAC_PI_2, HEAD.0, HEAD.1),
            tail: Spring::new(-FRAC_PI_2, TAIL.0, TAIL.1),
            bubble: Spring::new(0., 380., 20.),
            pull_x: Spring::new(0., 300., 18.),
            pull_y: Spring::new(0., 300., 18.),
            centre_pop: Spring::new(1., 500., 16.),
            selected: None,
            bounce: Bounce::level(2),
            logo: None,
        }
    }

    /// Pop the ring up in the corner of the monitor under the cursor.
    pub fn open(&mut self, items: Vec<RingItem>, logo: Arc<RenderImage>, bounce: u8, cx: &mut Context<Self>) {
        let b = Bounce::level(bounce);
        self.bounce = b;
        let now = Instant::now();
        self.logo = Some(logo);
        self.icons = items.iter().map(|_| Spring::bouncy(0., 420., b.icon)).collect();
        self.items = items;
        self.selected = None;
        self.phase = Phase::Open;
        self.opened = now;
        self.last_frame = now;
        self.scale = Spring { target: 1., ..Spring::bouncy(0.3, 260., b.pop) };
        // A kick sideways so it wobbles while it grows.
        self.squash = Spring { v: 2.6 * b.kick, ..Spring::bouncy(0., 200., b.wobble) };
        self.head = Spring { x: self.head.x, ..Spring::bouncy(self.head.x, HEAD.0, b.head) };
        self.centre_pop = Spring::bouncy(1., 500., b.icon);
        self.bubble = Spring::new(0., 380., 20.);
        self.pull_x = Spring::new(0., 300., 18.);
        self.pull_y = Spring::new(0., 300., 18.);
        if let Some(raw) = self.hwnd {
            // Not synchronously: moving the window re-enters GPUI.
            cx.spawn(async move |_, _| win::place_corner_popup(raw, SIZE, MARGIN)).detach();
        }
        cx.notify();
    }

    /// Alt was released: close, and hand back the tool that was pointed at.
    pub fn release(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if self.phase != Phase::Open {
            return None;
        }
        let chosen = self.selected.and_then(|i| self.items.get(i)).map(|item| item.id.clone());
        self.close(cx);
        chosen
    }

    /// Shrink away (and hide once small enough).
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.phase != Phase::Open {
            return;
        }
        self.phase = Phase::Closing;
        self.scale.target = 0.;
        self.scale.stiffness = 520.;
        self.scale.damping = 34.;
        self.squash.v -= 1.2 * self.bounce.kick;
        cx.notify();
    }

    fn item_angle(&self, i: usize) -> f32 {
        -FRAC_PI_2 + i as f32 * TAU / self.items.len().max(1) as f32
    }

    /// Follow the pointer: the icon in its direction from the centre.
    fn update_selection(&mut self) {
        if self.phase != Phase::Open || self.items.is_empty() {
            return;
        }
        let Some((x, y)) = self.hwnd.and_then(win::cursor_in_window) else { return };
        self.stick_to(x, y);
        let (dx, dy) = (x - CENTER, y - CENTER);
        let dist = (dx * dx + dy * dy).sqrt();
        let n = self.items.len();
        let selected = (DEAD_ZONE..REACH).contains(&dist).then(|| {
            let step = TAU / n as f32;
            let from_top = (dy.atan2(dx) + FRAC_PI_2).rem_euclid(TAU);
            ((from_top / step).round() as usize) % n
        });
        if selected == self.selected {
            return;
        }
        if let Some(i) = selected {
            // Take the short way round; the tail follows the same way.
            let target = self.item_angle(i);
            let turns = ((self.head.x - target) / TAU).round();
            self.head.target = target + turns * TAU;
            self.tail.target = self.head.target;
            if self.bubble.x < 0.05 {
                for end in [&mut self.head, &mut self.tail] {
                    end.x = end.target;
                    end.v = 0.;
                }
            }
            // Every change of selection jiggles the jelly a little.
            let kick = 0.9 * self.bounce.kick;
            self.squash.v += if self.squash.x > 0. { -kick } else { kick };
        }
        self.bubble.target = if selected.is_some() { 1. } else { 0. };
        self.selected = selected;
        // The centre's new picture pops in.
        self.centre_pop.x = 0.6;
        self.centre_pop.v = 0.;
    }

    /// Lean the pointed-at icon towards the pointer at (`x`, `y`).
    fn stick_to(&mut self, x: f32, y: f32) {
        let (tx, ty) = match self.selected {
            Some(i) => {
                let a = self.item_angle(i);
                let (dx, dy) = (x - (CENTER + MID * a.cos()), y - (CENTER + MID * a.sin()));
                let (lx, ly) = (dx * PULL, dy * PULL);
                let len = (lx * lx + ly * ly).sqrt();
                let k = if len > MAX_PULL { MAX_PULL / len } else { 1. };
                (lx * k, ly * k)
            }
            None => (0., 0.),
        };
        self.pull_x.target = tx;
        self.pull_y.target = ty;
    }

    fn step(&mut self, dt: f32) {
        let since = self.opened.elapsed().as_secs_f32();
        for (i, icon) in self.icons.iter_mut().enumerate() {
            icon.target = match self.phase {
                Phase::Open if since >= i as f32 * STAGGER => {
                    if self.selected == Some(i) {
                        1.3
                    } else {
                        1.
                    }
                }
                _ => 0.,
            };
            icon.step(dt);
        }
        for spring in [
            &mut self.scale,
            &mut self.squash,
            &mut self.head,
            &mut self.tail,
            &mut self.bubble,
            &mut self.pull_x,
            &mut self.pull_y,
            &mut self.centre_pop,
        ] {
            spring.step(dt);
        }
    }

    fn launch(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.and_then(|i| self.items.get(i)).map(|item| item.id.clone()) else {
            return;
        };
        self.close(cx);
        self.launcher.update(cx, |launcher, cx| launcher.launch_tool(&id, cx)).ok();
    }

    fn tile(&self, visual: &Visual, size: f32, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.launcher.update(cx, |launcher, cx| launcher.tile(visual, size, size * 0.56, theme, cx)).ok()
    }
}

/// `el` (a `size`-px square) absolutely positioned with its centre on (`x`, `y`).
fn place(el: AnyElement, x: f32, y: f32, size: f32) -> AnyElement {
    div().absolute().left(px(x - size / 2.)).top(px(y - size / 2.)).size(px(size)).child(el).into_any_element()
}

/// An absolutely positioned circle centred on (`x`, `y`) with radii `rx` x `ry`.
fn disc(x: f32, y: f32, rx: f32, ry: f32) -> gpui::Div {
    div()
        .absolute()
        .left(px(x - rx))
        .top(px(y - ry))
        .w(px(rx * 2.))
        .h(px(ry * 2.))
        .rounded_full()
}

impl Render for Ring {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(1. / 20.);
        self.last_frame = now;
        self.update_selection();
        self.step(dt);
        if self.phase == Phase::Closing && self.scale.x <= 0.03 {
            self.phase = Phase::Hidden;
            if let Some(raw) = self.hwnd {
                cx.spawn(async move |_, _| win::hide_window(raw)).detach();
            }
        }
        // Keep animating (and following the pointer) while the ring is up.
        if self.phase != Phase::Hidden {
            window.request_animation_frame();
        }

        let root = div()
            .id("ring")
            .size_full()
            .relative()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.update_selection();
                    this.launch(cx);
                }),
            );
        let s = self.scale.x.max(0.);
        if self.phase == Phase::Hidden || s < 0.02 {
            return root;
        }
        let theme = Theme::get(cx).clone();
        let (sx, sy) = (s * (1. + self.squash.x), s * (1. - self.squash.x));
        let c = CENTER;
        let accent = theme.accent;
        let mut layers: Vec<AnyElement> = Vec::new();

        let white = |a: f32| hsla(0., 0., 1., a);
        let point_on = |angle: f32, radius: f32| (c + radius * sx * angle.cos(), c + radius * sy * angle.sin());

        // The glass band: a faint tint with crisp rims, so the desktop shows through.
        layers.push(disc(c, c, (OUTER + 6.) * sx, (OUTER + 6.) * sy).border(px(6. * s)).border_color(accent.opacity(0.08)).into_any_element());
        layers.push(disc(c, c, OUTER * sx, OUTER * sy).border(px(BAND * s)).border_color(accent.opacity(0.2)).into_any_element());
        layers.push(disc(c, c, OUTER * sx, OUTER * sy).border(px(1.5 * s)).border_color(accent.opacity(0.6)).into_any_element());
        layers.push(disc(c, c, (OUTER - 1.5) * sx, (OUTER - 1.5) * sy).border(px(1.5 * s)).border_color(white(0.6)).into_any_element());
        layers.push(disc(c, c, (OUTER - BAND) * sx, (OUTER - BAND) * sy).border(px(1.5 * s)).border_color(accent.opacity(0.45)).into_any_element());
        // Glints on the upper left. Kept apart: stacked translucent shapes come out
        // grey in a transparent window (their alpha adds up).
        for (angle, r, alpha) in [(-2.42, 4.6, 0.75), (-2.12, 2.4, 0.6), (-1.95, 1.4, 0.5)] {
            let (x, y) = point_on(angle, OUTER - 12.);
            layers.push(disc(x, y, r * s, r * s).bg(white(alpha)).into_any_element());
        }

        // The goo: a chain of overlapping discs from the lagging tail to the head,
        // thinning in the middle the further it is stretched.
        let b = self.bubble.x.max(0.);
        if b > 0.02 {
            let (head, tail) = (self.head.x, self.tail.x);
            let span = (head - tail).abs() * MID;
            let steps = ((span / 4.).ceil() as usize).clamp(1, 60);
            let (r_head, r_tail) = (34. * s * b, 25. * s * b);
            let neck = (span / 150.).clamp(0., 0.62);
            let goo = hsla(accent.h, accent.s, (accent.l + 0.1).min(0.9), 1.);
            let (px_, py_) = (self.pull_x.x * s, self.pull_y.x * s);
            let (hx, hy) = point_on(head, MID);
            layers.push(
                disc(hx + px_, hy + py_, r_head * 0.9, r_head * 0.9)
                    .shadow(vec![BoxShadow {
                        color: hsla(accent.h, accent.s, accent.l * 0.7, 0.45),
                        offset: point(px(0.), px(5. * s)),
                        blur_radius: px(16. * s),
                        spread_radius: px(0.),
                    }])
                    .into_any_element(),
            );
            for k in 0..=steps {
                let t = k as f32 / steps as f32;
                let (x, y) = point_on(tail + (head - tail) * t, MID);
                let r = (r_tail + (r_head - r_tail) * t) * (1. - neck * (std::f32::consts::PI * t).sin());
                let r = r.max(3. * s);
                // The head sticks to the pointer; the tail stays put.
                let (x, y) = (x + px_ * t * t, y + py_ * t * t);
                let (rx, ry) = (r * (1. + self.squash.x), r * (1. - self.squash.x));
                layers.push(disc(x, y, rx, ry).bg(goo).into_any_element());
            }
            // A glint on the head.
            layers.push(disc(hx + px_ - r_head * 0.42, hy + py_ - r_head * 0.48, r_head * 0.22, r_head * 0.13).bg(white(0.7)).into_any_element());
        }

        // Centre: an opaque glossy bubble (translucent layers would turn grey here)
        // showing the pointed-at app, or the mascot when nothing is pointed at.
        let base = Hsla { a: 1., ..theme.surface };
        let (top, bottom, rim) = (base, mix(base, accent, 0.16), mix(base, accent, 0.45));
        let (rx, ry) = (INNER * sx, INNER * sy);
        layers.push(
            disc(c, c, rx, ry)
                .bg(linear_gradient(180., linear_color_stop(top, 0.25), linear_color_stop(bottom, 1.)))
                .border(px(2. * s))
                .border_color(rim)
                .into_any_element(),
        );
        // Gloss across the top of the bubble.
        layers.push(disc(c, c - ry * 0.62, rx * 0.46, ry * 0.12).bg(mix(top, hsla(0., 0., 1., 1.), 0.85)).into_any_element());
        // The picture grows and pops with the ring; the text keeps a fixed size and
        // place (scaling it every frame made it shimmer) and just fades in and out.
        let pop = self.centre_pop.x.max(0.);
        let selected = self.selected.and_then(|i| self.items.get(i)).map(|item| (item.visual.clone(), item.name.clone()));
        let (picture, title, note): (Option<AnyElement>, SharedString, SharedString) = match selected {
            Some((visual, name)) => {
                let size = 62. * s * pop;
                let tile = self.tile(&visual, size, &theme, cx).map(|t| place(t, c, c - 20., size));
                (tile, name, "松开 Alt 打开".into())
            }
            None => {
                let size = 76. * s * pop;
                let logo = self.logo.clone().map(|logo| place(img(logo).size(px(size)).into_any_element(), c, c - 14., size));
                let (title, note) = if self.items.is_empty() {
                    ("还没有工具", "先在主界面添加工具")
                } else {
                    ("", "指向图标 · 松开 Alt")
                };
                (logo, title.into(), note.into())
            }
        };
        layers.extend(picture);
        let fade = ((s - 0.8) / 0.2).clamp(0., 1.);
        if fade > 0. {
            let hint = hsla(accent.h, accent.s * 0.7, if theme.dark { 0.72 } else { 0.42 }, 1.);
            let line = |top: f32, height: f32| div().absolute().left(px(c - 72.)).top(px(top)).w(px(144.)).h(px(height)).flex().justify_center().opacity(fade);
            let (title_y, note_y) = if title.is_empty() { (0., c + 36.) } else { (c + 18., c + 40.) };
            if !title.is_empty() {
                layers.push(
                    line(title_y, 20.)
                        .child(div().max_w_full().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(theme.text).child(title))
                        .into_any_element(),
                );
            }
            layers.push(line(note_y, 16.).child(div().text_size(px(10.5)).text_color(hint).child(note)).into_any_element());
        }

        // Icons around the band.
        for i in 0..self.items.len() {
            let grow = self.icons[i].x.max(0.);
            let size = TILE * s * grow;
            if size < 2. {
                continue;
            }
            let (mut x, mut y) = point_on(self.item_angle(i), MID);
            if self.selected == Some(i) {
                x += self.pull_x.x * s;
                y += self.pull_y.x * s;
            }
            let visual = self.items[i].visual.clone();
            if let Some(tile) = self.tile(&visual, size, &theme, cx) {
                layers.push(div().absolute().left(px(x - size / 2.)).top(px(y - size / 2.)).child(tile).into_any_element());
            }
        }

        root.children(layers)
    }
}
