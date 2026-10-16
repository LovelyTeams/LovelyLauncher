//! Markdown → GPUI elements (CommonMark + tables, strikethrough, task lists).
//! Used by the file preview and for `@ai` answers.

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use gpui::{
    AnyElement, ClipboardItem, FontStyle, FontWeight, HighlightStyle, Image, ImageSource, IntoElement,
    ObjectFit, StrikethroughStyle, StyledImage, StyledText, UnderlineStyle, div, img, prelude::*, px,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::{select::Selection, theme::Theme};

/// Element ids for copy buttons must be unique per frame across renders.
static COPY_IDS: AtomicUsize = AtomicUsize::new(0);

/// With `sel`, the text can be selected with the mouse (see [`Selection`]).
pub fn render(source: &str, base_dir: Option<&Path>, sel: Option<&Selection>, theme: &Theme) -> Vec<AnyElement> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    render_events(Parser::new_ext(source, options), base_dir, &[], sel, theme)
}

/// Draw events that were produced elsewhere (the HTML preview builds its own).
/// An image whose destination is `inline-image:N` is `images[N]`.
pub fn render_events<'e>(
    events: impl IntoIterator<Item = Event<'e>>,
    base_dir: Option<&Path>,
    images: &[Arc<Image>],
    sel: Option<&Selection>,
    theme: &Theme,
) -> Vec<AnyElement> {
    let mut r = Renderer {
        theme,
        sel,
        base_dir,
        inline_images: images,
        stack: vec![Frame::new(Container::Root)],
        inline: None,
        styles: Vec::new(),
        heading: None,
        code: None,
        code_lang: None,
        image: None,
        pending_images: Vec::new(),
        table: None,
    };
    for event in events {
        r.event(event);
    }
    // Unterminated constructs while an answer is still streaming.
    if let Some(code) = r.code.take() {
        let el = r.code_block(code);
        r.add_block(el);
    }
    r.flush_inline();
    while r.stack.len() > 1 {
        r.close_container();
    }
    r.stack.pop().map(|f| f.children).unwrap_or_default()
}

#[derive(Default)]
struct Inline {
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

enum Container {
    Root,
    Quote,
    List { ordered: bool, next: u64 },
    Item { marker: String },
}

struct Frame {
    kind: Container,
    children: Vec<AnyElement>,
}

impl Frame {
    fn new(kind: Container) -> Self {
        Self { kind, children: Vec::new() }
    }
}

struct TableState {
    rows: Vec<Vec<AnyElement>>,
    header_rows: usize,
    cols: usize,
}

struct Renderer<'a> {
    theme: &'a Theme,
    sel: Option<&'a Selection>,
    base_dir: Option<&'a Path>,
    inline_images: &'a [Arc<Image>],
    stack: Vec<Frame>,
    inline: Option<Inline>,
    styles: Vec<HighlightStyle>,
    heading: Option<u8>,
    code: Option<String>,
    /// Fence language of the current code block ("rust", "json", …).
    code_lang: Option<String>,
    /// (destination, alt text) while inside an image tag.
    image: Option<(String, String)>,
    pending_images: Vec<AnyElement>,
    table: Option<TableState>,
}

fn merge(styles: &[HighlightStyle]) -> HighlightStyle {
    let mut out = HighlightStyle::default();
    for s in styles {
        out.color = s.color.or(out.color);
        out.font_weight = s.font_weight.or(out.font_weight);
        out.font_style = s.font_style.or(out.font_style);
        out.background_color = s.background_color.or(out.background_color);
        out.underline = s.underline.or(out.underline);
        out.strikethrough = s.strikethrough.or(out.strikethrough);
    }
    out
}

impl Renderer<'_> {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if let Some(code) = self.code.as_mut() {
                    code.push_str(&text);
                } else if let Some((_, alt)) = self.image.as_mut() {
                    alt.push_str(&text);
                } else {
                    self.push_text(&text, None);
                }
            }
            Event::Code(code) => {
                let style = HighlightStyle {
                    color: Some(self.theme.accent),
                    background_color: Some(self.theme.kbd_bg),
                    ..Default::default()
                };
                self.push_text(&format!("\u{2009}{code}\u{2009}"), Some(style));
            }
            Event::SoftBreak => self.push_text(" ", None),
            Event::HardBreak => self.push_text("\n", None),
            Event::Rule => {
                self.flush_inline();
                let hr = div().my(px(8.)).h(px(1.)).bg(self.theme.border_strong).into_any_element();
                self.add_block(hr);
            }
            Event::TaskListMarker(done) => self.push_text(if done { "☑ " } else { "☐ " }, None),
            Event::Html(html) | Event::InlineHtml(html) => {
                if html.trim().eq_ignore_ascii_case("<br>") || html.trim().eq_ignore_ascii_case("<br/>") {
                    self.push_text("\n", None);
                }
            }
            Event::InlineMath(m) | Event::DisplayMath(m) => self.push_text(&m, None),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        let theme = self.theme;
        match tag {
            Tag::Paragraph => self.begin_inline(),
            Tag::Heading { level, .. } => {
                self.flush_inline();
                self.begin_inline();
                self.heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
            }
            Tag::BlockQuote(_) => {
                self.flush_inline();
                self.stack.push(Frame::new(Container::Quote));
            }
            Tag::CodeBlock(kind) => {
                self.flush_inline();
                self.code = Some(String::new());
                self.code_lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().map(str::to_string),
                    CodeBlockKind::Indented => None,
                };
            }
            Tag::List(start) => {
                self.flush_inline();
                self.stack.push(Frame::new(Container::List {
                    ordered: start.is_some(),
                    next: start.unwrap_or(1),
                }));
            }
            Tag::Item => {
                self.flush_inline();
                let marker = match self.stack.last_mut().map(|f| &mut f.kind) {
                    Some(Container::List { ordered: true, next }) => {
                        let m = format!("{next}.");
                        *next += 1;
                        m
                    }
                    _ => "•".to_string(),
                };
                self.stack.push(Frame::new(Container::Item { marker }));
            }
            Tag::Table(aligns) => {
                self.flush_inline();
                self.table = Some(TableState {
                    rows: Vec::new(),
                    header_rows: 0,
                    cols: aligns.len().max(1),
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.rows.push(Vec::new());
                }
            }
            Tag::TableCell => self.begin_inline(),
            Tag::Emphasis => self.styles.push(HighlightStyle {
                font_style: Some(FontStyle::Italic),
                ..Default::default()
            }),
            Tag::Strong => self.styles.push(HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            }),
            Tag::Strikethrough => self.styles.push(HighlightStyle {
                strikethrough: Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(theme.text_muted),
                }),
                ..Default::default()
            }),
            Tag::Link { .. } => self.styles.push(HighlightStyle {
                color: Some(theme.accent),
                underline: Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(theme.accent),
                    wavy: false,
                }),
                ..Default::default()
            }),
            Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) => self.flush_inline(),
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item => {
                self.flush_inline();
                self.close_container();
            }
            TagEnd::CodeBlock => {
                if let Some(code) = self.code.take() {
                    let el = self.code_block(code);
                    self.add_block(el);
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.header_rows = t.rows.len();
                }
            }
            TagEnd::TableCell => {
                let inline = self.inline.take().unwrap_or_default();
                // Cells of a row copy tab-separated, ready to paste into a spreadsheet.
                let first = self.table.as_ref().and_then(|t| t.rows.last()).is_none_or(|row| row.is_empty());
                let cell = self.text(inline.text, inline.highlights, if first { "\n" } else { "\t" });
                if let Some(row) = self.table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(cell);
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    let el = self.table_element(table);
                    self.add_block(el);
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.styles.pop();
            }
            TagEnd::Image => {
                if let Some((dest, alt)) = self.image.take() {
                    let source: Option<ImageSource> = match dest.strip_prefix("inline-image:") {
                        Some(n) => n.parse::<usize>().ok().and_then(|n| self.inline_images.get(n)).cloned().map(Into::into),
                        None => self.local_image(&dest).map(Into::into),
                    };
                    match source {
                        Some(source) => self.pending_images.push(
                            div()
                                .my(px(6.))
                                .max_w_full()
                                .child(img(source).max_w_full().max_h(px(420.)).object_fit(ObjectFit::Contain))
                                .into_any_element(),
                        ),
                        None => {
                            let label = if alt.is_empty() { dest } else { alt };
                            let style = HighlightStyle {
                                color: Some(self.theme.text_faint),
                                ..Default::default()
                            };
                            self.push_text(&format!("[图片：{label}]"), Some(style));
                        }
                    }
                    if self.inline.is_none() {
                        let images = std::mem::take(&mut self.pending_images);
                        for el in images {
                            self.add_block(el);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn local_image(&self, dest: &str) -> Option<PathBuf> {
        if dest.contains("://") || dest.starts_with("data:") {
            return None;
        }
        let dest = dest.split(['?', '#']).next().unwrap_or(dest).replace("%20", " ");
        let path = PathBuf::from(&dest);
        let path = if path.is_absolute() {
            path
        } else {
            self.base_dir?.join(path)
        };
        path.is_file().then_some(path)
    }

    fn begin_inline(&mut self) {
        if self.inline.is_none() {
            self.inline = Some(Inline::default());
        }
    }

    fn push_text(&mut self, text: &str, extra: Option<HighlightStyle>) {
        self.begin_inline();
        let mut styles = self.styles.clone();
        styles.extend(extra);
        let style = merge(&styles);
        let inline = self.inline.as_mut().unwrap();
        let start = inline.text.len();
        inline.text.push_str(text);
        if style != HighlightStyle::default() {
            let end = inline.text.len();
            match inline.highlights.last_mut() {
                Some((range, s)) if *s == style && range.end == start => range.end = end,
                _ => inline.highlights.push((start..end, style)),
            }
        }
    }

    /// A run of text: selectable when there is a selection to take part in.
    fn text(&self, text: String, highlights: Vec<(Range<usize>, HighlightStyle)>, sep: &'static str) -> AnyElement {
        match self.sel {
            Some(sel) => sel.piece(text, highlights, sep, self.theme.selection),
            None => StyledText::new(text).with_highlights(highlights).into_any_element(),
        }
    }

    /// Emit the current inline buffer (paragraph / heading / tight list text).
    fn flush_inline(&mut self) {
        let heading = self.heading.take();
        if let Some(inline) = self.inline.take() {
            if !inline.text.trim().is_empty() {
                let text = self.text(inline.text, inline.highlights, "\n");
                let el = match heading {
                    Some(level) => {
                        let (size, top) = match level {
                            1 => (22., 10.),
                            2 => (19., 8.),
                            3 => (16.5, 6.),
                            _ => (15., 4.),
                        };
                        div()
                            .mt(px(top))
                            .mb(px(2.))
                            .text_size(px(size))
                            .line_height(px(size * 1.45))
                            .font_weight(FontWeight::BOLD)
                            .child(text)
                    }
                    None => div().text_size(px(14.)).line_height(px(22.)).child(text),
                };
                self.add_block(el.into_any_element());
            }
        }
        let images = std::mem::take(&mut self.pending_images);
        for el in images {
            self.add_block(el);
        }
    }

    fn add_block(&mut self, el: AnyElement) {
        if let Some(frame) = self.stack.last_mut() {
            frame.children.push(el);
        }
    }

    fn close_container(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let frame = self.stack.pop().unwrap();
        let theme = self.theme;
        let el = match frame.kind {
            Container::Root => return,
            Container::Quote => div()
                .my(px(4.))
                .pl(px(12.))
                .border_l_2()
                .border_color(theme.accent.opacity(0.5))
                .text_color(theme.text_muted)
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(frame.children)
                .into_any_element(),
            Container::List { .. } => div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .children(frame.children)
                .into_any_element(),
            Container::Item { marker } => div()
                .flex()
                .gap(px(8.))
                .pl(px(4.))
                .child(
                    div()
                        .flex_none()
                        .min_w(px(14.))
                        .text_size(px(14.))
                        .line_height(px(22.))
                        .text_color(theme.accent)
                        .child(marker),
                )
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).children(frame.children))
                .into_any_element(),
        };
        self.add_block(el);
    }

    fn code_block(&mut self, code: String) -> AnyElement {
        let theme = self.theme;
        let code = code.trim_end_matches('\n').replace('\t', "    ");
        let lang = self
            .code_lang
            .take()
            .and_then(|name| crate::highlight::lang_for_name(&name));
        let highlights = lang.map(|l| crate::highlight::by_line(&code, &crate::highlight::highlight(&code, l)));
        let lines: Vec<AnyElement> = code
            .split('\n')
            .enumerate()
            .map(|(i, l)| {
                let spans = highlights.as_ref().and_then(|h| h.get(i)).filter(|s| !s.is_empty());
                let styles = spans.map(|spans| crate::highlight::styled(spans, theme.dark)).unwrap_or_default();
                match self.sel {
                    Some(sel) => div().child(sel.piece(l.to_string(), styles, "\n", theme.selection)).into_any_element(),
                    None if styles.is_empty() => {
                        div().child(if l.is_empty() { " ".to_string() } else { l.to_string() }).into_any_element()
                    }
                    None => div().child(StyledText::new(l.to_string()).with_highlights(styles)).into_any_element(),
                }
            })
            .collect();
        let copy = code.clone();
        let id = COPY_IDS.fetch_add(1, Ordering::Relaxed);
        div()
            .my(px(4.))
            .relative()
            .rounded(px(8.))
            .bg(theme.kbd_bg)
            .border_1()
            .border_color(theme.border)
            .p(px(10.))
            .pr(px(64.))
            .font_family("Consolas")
            .text_size(px(12.5))
            .line_height(px(19.))
            .children(lines)
            .child(
                div()
                    .id(("md-copy", id))
                    .absolute()
                    .top(px(6.))
                    .right(px(6.))
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(6.))
                    .font_family(".SystemUIFont")
                    .text_size(px(11.5))
                    .text_color(theme.text_muted)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.row_hover).text_color(theme.accent))
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())))
                    .child("复制"),
            )
            .into_any_element()
    }

    fn table_element(&self, table: TableState) -> AnyElement {
        let theme = self.theme;
        let cols = table.cols;
        let mut cells = Vec::new();
        for (r, row) in table.rows.into_iter().enumerate() {
            let header = r < table.header_rows;
            let len = row.len();
            for cell in row.into_iter().chain((len..cols).map(|_| div().into_any_element())).take(cols) {
                cells.push(
                    div()
                        .px(px(10.))
                        .py(px(6.))
                        .border_b_1()
                        .border_r_1()
                        .border_color(theme.border)
                        .text_size(px(13.))
                        .line_height(px(19.))
                        .when(header, |d| d.bg(theme.kbd_bg).font_weight(FontWeight::SEMIBOLD))
                        .child(cell)
                        .into_any_element(),
                );
            }
        }
        div()
            .my(px(6.))
            .grid()
            .grid_cols(cols as u16)
            .border_t_1()
            .border_l_1()
            .border_color(theme.border)
            .rounded(px(6.))
            .overflow_hidden()
            .children(cells)
            .into_any_element()
    }
}
