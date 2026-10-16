//! Drag-to-select text that is split over many `StyledText` pieces (rows of a
//! file, paragraphs of a document), and copy it.
//!
//! Each frame the view hands its text out through [`Selection::piece`], which
//! remembers where the piece was laid out. Mouse positions are then mapped back
//! to (piece key, byte offset), and selected bytes get a background highlight.

use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
};

use gpui::{
    AnyElement, App, Bounds, CursorStyle, Element, ElementId, GlobalElementId, HighlightStyle, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Point, SharedString, StyledText,
    TextLayout, Window,
};

/// (piece key, byte offset in that piece). Keys follow reading order.
pub type Pos = (usize, usize);

struct Piece {
    key: usize,
    text: SharedString,
    /// Text that goes before this piece when copying ("\n" or "").
    sep: &'static str,
    layout: TextLayout,
    /// Set once the piece was laid out and placed, so its layout can be queried.
    placed: Rc<Cell<bool>>,
}

#[derive(Default)]
struct State {
    anchor: Option<Pos>,
    head: Option<Pos>,
    dragging: bool,
    /// Double / triple click: extend by words / whole pieces.
    unit: Unit,
    /// The word / piece first clicked, which stays selected while dragging.
    origin: (Pos, Pos),
    pieces: Vec<Piece>,
    next_key: usize,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Unit {
    #[default]
    Char,
    Word,
    Piece,
}

#[derive(Clone, Default)]
pub struct Selection(Rc<RefCell<State>>);

impl Selection {
    /// Start a new frame: forget last frame's pieces. Keys handed out by
    /// [`Selection::piece`] start at `first_key`.
    pub fn begin_frame(&self, first_key: usize) {
        let mut s = self.0.borrow_mut();
        s.pieces.clear();
        s.next_key = first_key;
    }

    pub fn clear(&self) {
        let mut s = self.0.borrow_mut();
        s.anchor = None;
        s.head = None;
        s.dragging = false;
    }

    pub fn is_dragging(&self) -> bool {
        self.0.borrow().dragging
    }

    /// The selected span in reading order, if it is not empty.
    pub fn range(&self) -> Option<(Pos, Pos)> {
        let s = self.0.borrow();
        let (a, b) = (s.anchor?, s.head?);
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        (start != end).then_some((start, end))
    }

    /// Select everything from `start` to `end`.
    pub fn set(&self, start: Pos, end: Pos) {
        let mut s = self.0.borrow_mut();
        s.anchor = Some(start);
        s.head = Some(end);
        s.dragging = false;
    }

    /// A piece with the next key in order (documents rendered all at once).
    pub fn piece(
        &self,
        text: impl Into<SharedString>,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
        sep: &'static str,
        color: Hsla,
    ) -> AnyElement {
        let key = {
            let mut s = self.0.borrow_mut();
            s.next_key += 1;
            s.next_key - 1
        };
        self.keyed_piece(key, text, highlights, sep, color)
    }

    /// A piece with a fixed key (rows of a virtual list, which come and go).
    pub fn keyed_piece(
        &self,
        key: usize,
        text: impl Into<SharedString>,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
        sep: &'static str,
        color: Hsla,
    ) -> AnyElement {
        let text: SharedString = text.into();
        let selected = self.selected_in(key, text.len());
        let highlights = match selected {
            Some(range) => with_selection(highlights, range, text.len(), color),
            None => highlights,
        };
        // An empty piece still needs a line box to be pointed at.
        let shown: SharedString = if text.is_empty() { " ".into() } else { text.clone() };
        let styled = StyledText::new(shown).with_highlights(highlights);
        let placed = Rc::new(Cell::new(false));
        self.0.borrow_mut().pieces.push(Piece {
            key,
            text,
            sep,
            layout: styled.layout().clone(),
            placed: placed.clone(),
        });
        Selectable { inner: styled, placed, hitbox: None }.into_any_element()
    }

    /// Bytes of piece `key` (of length `len`) that are selected.
    fn selected_in(&self, key: usize, len: usize) -> Option<Range<usize>> {
        let ((k1, o1), (k2, o2)) = self.range()?;
        if key < k1 || key > k2 {
            return None;
        }
        let start = if key == k1 { o1.min(len) } else { 0 };
        let end = if key == k2 { o2.min(len) } else { len };
        (start < end).then_some(start..end)
    }

    /// Select every piece of this frame (a document shown in full).
    pub fn select_all_pieces(&self) {
        let ends = {
            let s = self.0.borrow();
            let first = s.pieces.iter().min_by_key(|p| p.key).map(|p| (p.key, 0));
            let last = s.pieces.iter().max_by_key(|p| p.key).map(|p| (p.key, p.text.len()));
            first.zip(last)
        };
        if let Some((first, last)) = ends {
            self.set(first, last);
        }
    }

    /// Mouse pressed at `p`: start selecting (double click: a word, triple: the piece).
    pub fn press(&self, p: Point<Pixels>, clicks: usize) {
        let Some(pos) = self.hit(p) else {
            self.clear();
            return;
        };
        let unit = match clicks {
            0 | 1 => Unit::Char,
            2 => Unit::Word,
            _ => Unit::Piece,
        };
        let (start, end) = self.expand(pos, unit);
        let mut s = self.0.borrow_mut();
        s.unit = unit;
        s.origin = (start, end);
        s.anchor = Some(start);
        s.head = Some(end);
        s.dragging = true;
    }

    /// Mouse moved to `p` with the button held; returns whether anything changed.
    pub fn drag(&self, p: Point<Pixels>) -> bool {
        if !self.is_dragging() {
            return false;
        }
        let Some(pos) = self.hit(p) else { return false };
        let unit = self.0.borrow().unit;
        let (start, end) = self.expand(pos, unit);
        let mut s = self.0.borrow_mut();
        // The first clicked word / piece stays selected whichever way it grows.
        let (anchor, head) = if pos < s.origin.0 { (s.origin.1, start) } else { (s.origin.0, end) };
        let changed = s.anchor != Some(anchor) || s.head != Some(head);
        s.anchor = Some(anchor);
        s.head = Some(head);
        changed
    }

    pub fn release(&self) {
        self.0.borrow_mut().dragging = false;
    }

    /// Select whole words / pieces around `pos`.
    fn expand(&self, pos: Pos, unit: Unit) -> (Pos, Pos) {
        let s = self.0.borrow();
        let Some(piece) = s.pieces.iter().find(|p| p.key == pos.0) else {
            return (pos, pos);
        };
        match unit {
            Unit::Char => (pos, pos),
            Unit::Piece => ((pos.0, 0), (pos.0, piece.text.len())),
            Unit::Word => {
                let (a, b) = word_at(&piece.text, pos.1);
                ((pos.0, a), (pos.0, b))
            }
        }
    }

    /// The text position under `p`, or the nearest one.
    fn hit(&self, p: Point<Pixels>) -> Option<Pos> {
        let s = self.0.borrow();
        let placed: Vec<&Piece> = s.pieces.iter().filter(|piece| piece.placed.get()).collect();
        let bounds = |piece: &Piece| piece.layout.bounds();
        let at = |piece: &Piece| {
            let ix = match piece.layout.index_for_position(p) {
                Ok(ix) | Err(ix) => ix,
            };
            Some((piece.key, ix.min(piece.text.len())))
        };
        // Pieces on the pointer's line: the one under it, else the nearest sideways.
        let on_line = placed
            .iter()
            .filter(|piece| {
                let b = bounds(piece);
                p.y >= b.top() && p.y < b.bottom()
            })
            .min_by(|a, b| {
                let d = |piece: &Piece| {
                    let b = bounds(piece);
                    if p.x < b.left() { b.left() - p.x } else if p.x > b.right() { p.x - b.right() } else { gpui::px(0.) }
                };
                d(a).partial_cmp(&d(b)).unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some(piece) = on_line {
            return at(piece);
        }
        // Between lines: the end of the closest piece above, else the start of the first.
        let above = placed
            .iter()
            .filter(|piece| bounds(piece).bottom() <= p.y)
            .max_by(|a, b| {
                let (a, b) = (bounds(a), bounds(b));
                (a.bottom(), -a.left()).partial_cmp(&(b.bottom(), -b.left())).unwrap_or(std::cmp::Ordering::Equal)
            });
        match above {
            Some(piece) => Some((piece.key, piece.text.len())),
            None => placed.iter().min_by_key(|piece| piece.key).map(|piece| (piece.key, 0)),
        }
    }

    /// The selected text, when every selected piece was on screen this frame
    /// (documents); `None` lets the caller build it from its own data.
    pub fn text_from_pieces(&self) -> Option<String> {
        let ((k1, o1), (k2, o2)) = self.range()?;
        let s = self.0.borrow();
        let mut out = String::new();
        for key in k1..=k2 {
            let piece = s.pieces.iter().find(|p| p.key == key)?;
            let text = piece.text.as_ref();
            let start = if key == k1 { o1.min(text.len()) } else { 0 };
            let end = if key == k2 { o2.min(text.len()) } else { text.len() };
            if key != k1 {
                out.push_str(piece.sep);
            }
            out.push_str(text.get(start..end.max(start)).unwrap_or(""));
        }
        Some(out)
    }
}

/// The word around byte `at`: letters, digits and `_`, or a run of CJK characters,
/// or a single other character.
fn word_at(text: &str, at: usize) -> (usize, usize) {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Cjk,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() && (c as u32) < 0x2E80 || c == '_' {
            Class::Word
        } else if c.is_alphanumeric() {
            Class::Cjk
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let at = at.min(text.len());
    let Some((start, c)) = text[at..].char_indices().next().map(|(i, c)| (at + i, c)).or_else(|| text[..at].char_indices().last())
    else {
        return (at, at);
    };
    let kind = class(c);
    if kind == Class::Other {
        return (start, start + c.len_utf8());
    }
    let mut a = start;
    for (i, c) in text[..start].char_indices().rev() {
        if class(c) != kind {
            break;
        }
        a = i;
    }
    let rest = start + c.len_utf8();
    let mut b = rest;
    for (i, c) in text[rest..].char_indices() {
        if class(c) != kind {
            break;
        }
        b = rest + i + c.len_utf8();
    }
    (a, b)
}

/// Lay `selected` over the syntax / style highlights as a background colour.
fn with_selection(
    mut highlights: Vec<(Range<usize>, HighlightStyle)>,
    selected: Range<usize>,
    len: usize,
    color: Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    highlights.sort_by_key(|(r, _)| r.start);
    // Cover every byte, so plain gaps can be marked too.
    let mut full = Vec::new();
    let mut at = 0;
    for (range, style) in highlights {
        let range = range.start.max(at)..range.end.min(len);
        if range.is_empty() {
            continue;
        }
        if range.start > at {
            full.push((at..range.start, HighlightStyle::default()));
        }
        at = range.end;
        full.push((range, style));
    }
    if at < len {
        full.push((at..len, HighlightStyle::default()));
    }
    let mut out = Vec::new();
    for (range, style) in full {
        let cuts = [range.start, selected.start.clamp(range.start, range.end), selected.end.clamp(range.start, range.end), range.end];
        for pair in cuts.windows(2) {
            let part = pair[0]..pair[1];
            if part.is_empty() {
                continue;
            }
            let inside = part.start >= selected.start && part.end <= selected.end;
            let style = if inside { HighlightStyle { background_color: Some(color), ..style } } else { style };
            out.push((part, style));
        }
    }
    out
}

/// `StyledText` that reports when it has been placed and shows a text cursor.
struct Selectable {
    inner: StyledText,
    placed: Rc<Cell<bool>>,
    hitbox: Option<Hitbox>,
}

impl IntoElement for Selectable {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Selectable {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.prepaint(id, inspector_id, bounds, state, window, cx);
        self.hitbox = Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));
        self.placed.set(true);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(hitbox) = &self.hitbox {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }
        self.inner.paint(id, inspector_id, bounds, state, prepaint, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        let t = "let foo_bar = 中文字符;";
        assert_eq!(&t[word_at(t, 5).0..word_at(t, 5).1], "foo_bar");
        let ix = t.find('文').unwrap();
        let (a, b) = word_at(t, ix);
        assert_eq!(&t[a..b], "中文字符");
        let ix = t.find('=').unwrap();
        assert_eq!(word_at(t, ix), (ix, ix + 1));
        assert_eq!(word_at("", 0), (0, 0));
        assert_eq!(&t[word_at(t, t.len()).0..], ";");
    }

    #[test]
    fn selection_overlays_highlights() {
        let red = HighlightStyle { color: Some(gpui::red()), ..Default::default() };
        let bg = gpui::blue();
        let out = with_selection(vec![(2..6, red)], 4..8, 10, bg);
        let ranges: Vec<_> = out.iter().map(|(r, s)| (r.clone(), s.background_color.is_some(), s.color.is_some())).collect();
        assert_eq!(
            ranges,
            [(0..2, false, false), (2..4, false, true), (4..6, true, true), (6..8, true, false), (8..10, false, false)]
        );
    }
}
