//! Thin overlay scrollbars — GPUI doesn't draw any of its own.

use std::{cell::Cell, rc::Rc};

use gpui::{
    Bounds, DispatchPhase, Hsla, ListOffset, ListState, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollHandle, canvas, div, fill, point, prelude::*, px,
};

/// What a scrollbar drives.
#[derive(Clone)]
pub enum Track {
    /// A scrollable div or uniform list.
    Handle(ScrollHandle),
    /// A variable-height list. Only rows near the viewport are measured, so the
    /// bar works in rows, each counted as `row` pixels tall.
    List { state: ListState, row: Pixels },
}

#[derive(Clone, Copy, PartialEq)]
pub enum Axis {
    Vertical,
    Horizontal,
}

/// Where inside the thumb the pointer grabbed it, while a drag is in progress.
pub type Drag = Rc<Cell<Option<f32>>>;

pub const THICKNESS: f32 = 12.;
const MIN_THUMB: f32 = 32.;
const PAD: f32 = 2.;

/// Positions along the scroll axis, in pixels.
#[derive(Clone, Copy)]
struct Metrics {
    viewport: f32,
    content: f32,
    offset: f32,
}

impl Metrics {
    fn max(&self) -> f32 {
        (self.content - self.viewport).max(0.)
    }
}

impl Track {
    fn metrics(&self, axis: Axis) -> Option<Metrics> {
        let m = match self {
            Track::Handle(handle) => {
                let (size, max, offset) = (handle.bounds().size, handle.max_offset(), handle.offset());
                let (viewport, max, offset) = match axis {
                    Axis::Vertical => (size.height, max.height, -offset.y),
                    Axis::Horizontal => (size.width, max.width, -offset.x),
                };
                Metrics { viewport: f32::from(viewport), content: f32::from(viewport + max), offset: f32::from(offset) }
            }
            Track::List { state, row } => {
                // Count in rows rather than pixels: how many rows fit on screen right
                // now (wrapped rows are taller), and how far down the top row is.
                let row = f32::from(*row);
                let count = state.item_count();
                let view = state.viewport_bounds();
                let top = state.logical_scroll_top();
                let mut visible = 0.;
                let mut first = 0.;
                for ix in top.item_ix..count.min(top.item_ix + 500) {
                    let Some(b) = state.bounds_for_item(ix) else { break };
                    if b.top() >= view.bottom() {
                        break;
                    }
                    let height = f32::from(b.size.height).max(1.);
                    let shown = f32::from(b.bottom().min(view.bottom()) - b.top().max(view.top())) / height;
                    if ix == top.item_ix {
                        first = (f32::from(view.top() - b.top()) / height).clamp(0., 1.);
                    }
                    visible += shown.clamp(0., 1.);
                }
                let visible = if visible > 0. { visible } else { f32::from(view.size.height) / row };
                Metrics {
                    viewport: visible * row,
                    content: count as f32 * row,
                    offset: (top.item_ix as f32 + first) * row,
                }
            }
        };
        (m.viewport > 0. && m.max() > 0.5).then_some(m)
    }

    /// Scroll so the thumb sits at `fraction` (0 = start, 1 = end) of its travel.
    fn scroll_to(&self, axis: Axis, fraction: f32, m: Metrics) {
        let offset = fraction * m.max();
        match self {
            Track::Handle(handle) => {
                let mut p: Point<Pixels> = handle.offset();
                match axis {
                    Axis::Vertical => p.y = px(-offset),
                    Axis::Horizontal => p.x = px(-offset),
                }
                handle.set_offset(p);
            }
            Track::List { state, row } => {
                let last = state.item_count().saturating_sub(1);
                if fraction >= 1. {
                    // The list settles so its last row sits at the bottom.
                    state.scroll_to(ListOffset { item_ix: last, offset_in_item: px(0.) });
                    return;
                }
                let row = f32::from(*row);
                let item_ix = ((offset / row).floor().max(0.) as usize).min(last);
                state.scroll_to(ListOffset { item_ix, offset_in_item: px(offset - item_ix as f32 * row) });
            }
        }
    }
}

/// An overlay bar along the right (vertical) or bottom (horizontal) edge of a `relative()` parent.
/// `corner` leaves room for the bar on the other axis.
pub fn scrollbar(track: Track, axis: Axis, drag: Drag, color: Hsla, corner: bool) -> impl IntoElement {
    let end = px(if corner { THICKNESS } else { 0. });
    let bar = div().absolute();
    let bar = match axis {
        Axis::Vertical => bar.top_0().right_0().bottom(end).w(px(THICKNESS)),
        Axis::Horizontal => bar.left_0().bottom_0().right(end).h(px(THICKNESS)),
    };
    bar.child(
        canvas(
            {
                let track = track.clone();
                move |bounds, window, _| {
                    track
                        .metrics(axis)
                        .map(|m| (m, window.insert_hitbox(bounds, gpui::HitboxBehavior::BlockMouseExceptScroll)))
                }
            },
            move |bounds, state, window, _| {
                let Some((m, hitbox)) = state else { return };
                let along = move |p: Point<Pixels>| f32::from(match axis {
                    Axis::Vertical => p.y - bounds.origin.y,
                    Axis::Horizontal => p.x - bounds.origin.x,
                });
                let length = f32::from(match axis {
                    Axis::Vertical => bounds.size.height,
                    Axis::Horizontal => bounds.size.width,
                }) - PAD * 2.;
                let thumb = (length * m.viewport / m.content).clamp(MIN_THUMB.min(length), length);
                let travel = (length - thumb).max(1.);
                let start = PAD + travel * (m.offset / m.max()).clamp(0., 1.);

                let hovered = hitbox.is_hovered(window);
                let dragging = drag.get().is_some();
                let width = if hovered || dragging { 8. } else { 5. };
                let thumb_bounds = match axis {
                    Axis::Vertical => Bounds::new(
                        point(bounds.right() - px(width + PAD), bounds.origin.y + px(start)),
                        gpui::size(px(width), px(thumb)),
                    ),
                    Axis::Horizontal => Bounds::new(
                        point(bounds.origin.x + px(start), bounds.bottom() - px(width + PAD)),
                        gpui::size(px(thumb), px(width)),
                    ),
                };
                let alpha = if dragging { 0.85 } else if hovered { 0.7 } else { 0.5 };
                window.paint_quad(fill(thumb_bounds, color.opacity(alpha)).corner_radii(px(width / 2.)));

                // Where along its travel the thumb is when it starts at `pos`.
                let fraction_at = move |pos: f32| ((pos - PAD) / travel).clamp(0., 1.);
                window.on_mouse_event({
                    let (track, drag, hitbox) = (track.clone(), drag.clone(), hitbox.clone());
                    move |e: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble || e.button != MouseButton::Left || !hitbox.is_hovered(window) {
                            return;
                        }
                        let pos = along(e.position);
                        let grab = if (start..start + thumb).contains(&pos) {
                            pos - start
                        } else {
                            // Clicking the track jumps there, then keeps dragging.
                            track.scroll_to(axis, fraction_at(pos - thumb / 2.), m);
                            thumb / 2.
                        };
                        drag.set(Some(grab));
                        window.refresh();
                        cx.stop_propagation();
                    }
                });
                window.on_mouse_event({
                    let drag = drag.clone();
                    move |e: &MouseMoveEvent, phase, window, _| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        match drag.get() {
                            Some(grab) if e.pressed_button == Some(MouseButton::Left) => {
                                track.scroll_to(axis, fraction_at(along(e.position) - grab), m);
                                window.refresh();
                            }
                            Some(_) => {
                                drag.set(None);
                                window.refresh();
                            }
                            None if hitbox.is_hovered(window) != hovered => window.refresh(),
                            None => {}
                        }
                    }
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, _| {
                    if phase == DispatchPhase::Bubble && drag.take().is_some() {
                        window.refresh();
                    }
                });
            },
        )
        .size_full(),
    )
}

/// `Drag` handles for the vertical and horizontal bars of one view.
#[derive(Clone, Default)]
pub struct Drags {
    pub vertical: Drag,
    pub horizontal: Drag,
}

pub fn vertical(track: Track, drags: &Drags, color: Hsla, corner: bool) -> impl IntoElement {
    scrollbar(track, Axis::Vertical, drags.vertical.clone(), color, corner)
}

pub fn horizontal(handle: ScrollHandle, drags: &Drags, color: Hsla) -> impl IntoElement {
    scrollbar(Track::Handle(handle), Axis::Horizontal, drags.horizontal.clone(), color, true)
}
