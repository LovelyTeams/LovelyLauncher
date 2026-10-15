//! HTML → reading view. A forgiving parser builds a small tree (unclosed `<p>`/`<li>`,
//! stray end tags and the like are tolerated), which is then turned into the same
//! events the Markdown renderer draws. Scripts, styles and layout are ignored: this
//! shows the page's text, headings, lists, tables, code and images.

use std::sync::Arc;

use gpui::Image;
use pulldown_cmark::{Alignment, CodeBlockKind, CowStr, Event, HeadingLevel, LinkType, Tag, TagEnd};

/// Stop converting very long pages; the rest is summarised with a note.
const MAX_EVENTS: usize = 60_000;

pub struct Page {
    pub title: Option<String>,
    pub events: Vec<Event<'static>>,
    /// Images embedded as `data:` URIs, referenced as `inline-image:N`.
    pub images: Vec<Arc<Image>>,
    pub truncated: bool,
}

pub fn convert(html: &str) -> Page {
    let tree = parse(html);
    let mut c = Converter::default();
    c.children(&tree, 0);
    c.close_para();
    if c.truncated {
        c.events.push(Event::Start(Tag::Paragraph));
        c.events.push(Event::Text("……页面较长，后面的内容没有显示".into()));
        c.events.push(Event::End(TagEnd::Paragraph));
    }
    Page { title: tree.title, events: c.events, images: c.images, truncated: c.truncated }
}

// ---------------------------------------------------------------------------
// Parsing

struct Node {
    /// Lower-case tag name; empty for text.
    tag: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<usize>,
}

struct Tree {
    nodes: Vec<Node>,
    title: Option<String>,
}

impl Tree {
    fn attr(&self, id: usize, name: &str) -> Option<&str> {
        self.nodes[id].attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr",
];
/// Their content is not markup.
const RAW: &[&str] = &["script", "style", "title", "textarea", "xmp", "noscript", "template"];
/// Starting one of these closes an open `<p>`.
const CLOSES_P: &[&str] = &[
    "address", "article", "aside", "blockquote", "details", "div", "dl", "fieldset", "figcaption", "figure", "footer",
    "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "main", "nav", "ol", "p", "pre", "section", "table",
    "ul",
];

fn parse(html: &str) -> Tree {
    let mut tree = Tree {
        nodes: vec![Node { tag: "#root".into(), attrs: Vec::new(), text: String::new(), children: Vec::new() }],
        title: None,
    };
    let mut stack: Vec<usize> = vec![0];
    let b = html.as_bytes();
    let mut i = 0;
    let mut text_start = 0;

    let flush_text = |tree: &mut Tree, stack: &[usize], from: usize, to: usize| {
        if from < to {
            let text = decode_entities(&html[from..to]);
            let parent = *stack.last().unwrap();
            tree.nodes.push(Node { tag: String::new(), attrs: Vec::new(), text, children: Vec::new() });
            let id = tree.nodes.len() - 1;
            tree.nodes[parent].children.push(id);
        }
    };

    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let next = b.get(i + 1).copied().unwrap_or(0);
        if html[i..].starts_with("<!--") {
            flush_text(&mut tree, &stack, text_start, i);
            i = html[i + 4..].find("-->").map_or(b.len(), |e| i + 4 + e + 3);
            text_start = i;
        } else if next == b'!' || next == b'?' {
            flush_text(&mut tree, &stack, text_start, i);
            i = html[i..].find('>').map_or(b.len(), |e| i + e + 1);
            text_start = i;
        } else if next == b'/' && b.get(i + 2).is_some_and(|c| c.is_ascii_alphabetic()) {
            flush_text(&mut tree, &stack, text_start, i);
            let end = html[i..].find('>').map_or(b.len(), |e| i + e + 1);
            let name = tag_name(&html[i + 2..end]);
            i = end;
            text_start = i;
            if name == "br" {
                add_element(&mut tree, &mut stack, "br".into(), Vec::new(), true);
            } else if let Some(pos) = stack.iter().rposition(|&id| tree.nodes[id].tag == name) {
                if pos > 0 {
                    stack.truncate(pos);
                }
            }
        } else if next.is_ascii_alphabetic() {
            flush_text(&mut tree, &stack, text_start, i);
            let (name, attrs, self_closing, end) = start_tag(html, i + 1);
            i = end;
            text_start = i;
            close_implied(&tree, &mut stack, &name);
            let void = self_closing || VOID.contains(&name.as_str());
            if RAW.contains(&name.as_str()) {
                let close = format!("</{name}");
                let lower = html[i..].to_ascii_lowercase();
                let stop = lower.find(&close).map_or(b.len(), |e| i + e);
                let content = &html[i..stop];
                if name == "title" && tree.title.is_none() {
                    let t = decode_entities(content).split_whitespace().collect::<Vec<_>>().join(" ");
                    if !t.is_empty() {
                        tree.title = Some(t);
                    }
                } else if matches!(name.as_str(), "textarea" | "xmp") {
                    let id = add_element(&mut tree, &mut stack, "pre".into(), attrs, true);
                    tree.nodes.push(Node { tag: String::new(), attrs: Vec::new(), text: decode_entities(content), children: Vec::new() });
                    let text = tree.nodes.len() - 1;
                    tree.nodes[id].children.push(text);
                }
                i = html[stop..].find('>').map_or(b.len(), |e| stop + e + 1);
                text_start = i;
            } else {
                add_element(&mut tree, &mut stack, name, attrs, void);
            }
        } else {
            i += 1;
        }
    }
    flush_text(&mut tree, &stack, text_start, b.len());
    tree
}

fn add_element(tree: &mut Tree, stack: &mut Vec<usize>, tag: String, attrs: Vec<(String, String)>, void: bool) -> usize {
    let parent = *stack.last().unwrap();
    tree.nodes.push(Node { tag, attrs, text: String::new(), children: Vec::new() });
    let id = tree.nodes.len() - 1;
    tree.nodes[parent].children.push(id);
    if !void {
        stack.push(id);
    }
    id
}

/// HTML's implied end tags, roughly: a new `<li>` ends the previous one, a block ends `<p>`, …
fn close_implied(tree: &Tree, stack: &mut Vec<usize>, name: &str) {
    let close_to = |stack: &mut Vec<usize>, target: &[&str], boundary: &[&str]| {
        for pos in (1..stack.len()).rev() {
            let tag = tree.nodes[stack[pos]].tag.as_str();
            if boundary.contains(&tag) {
                return;
            }
            if target.contains(&tag) {
                stack.truncate(pos);
                return;
            }
        }
    };
    if CLOSES_P.contains(&name) || name == "li" || name == "dt" || name == "dd" {
        close_to(stack, &["p"], &["li", "td", "th", "table", "blockquote", "div", "dd", "button"]);
    }
    match name {
        "li" => close_to(stack, &["li"], &["ul", "ol", "table"]),
        "dt" | "dd" => close_to(stack, &["dt", "dd"], &["dl", "table"]),
        "tr" => close_to(stack, &["tr"], &["table"]),
        "td" | "th" => close_to(stack, &["td", "th"], &["tr", "table"]),
        "thead" | "tbody" | "tfoot" => close_to(stack, &["thead", "tbody", "tfoot"], &["table"]),
        "option" => close_to(stack, &["option"], &["select"]),
        _ => {}
    }
}

fn tag_name(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Parse `<name attr=…>` starting just after `<`; returns (name, attrs, self-closing, index after `>`).
fn start_tag(html: &str, from: usize) -> (String, Vec<(String, String)>, bool, usize) {
    let b = html.as_bytes();
    let name = tag_name(&html[from..]);
    let mut i = from + name.len();
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            return (name, attrs, self_closing, b.len());
        }
        match b[i] {
            b'>' => return (name, attrs, self_closing, i + 1),
            b'/' => {
                self_closing = true;
                i += 1;
            }
            _ => {
                let start = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
                    i += 1;
                }
                let key = html[start..i].to_ascii_lowercase();
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
                let mut value = String::new();
                if i < b.len() && b[i] == b'=' {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                        let q = b[i];
                        let s = i + 1;
                        let e = html[s..].find(q as char).map_or(b.len(), |e| s + e);
                        value = decode_entities(&html[s..e]);
                        i = (e + 1).min(b.len());
                    } else {
                        let s = i;
                        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                            i += 1;
                        }
                        value = decode_entities(&html[s..i]);
                    }
                }
                self_closing = false;
                if !key.is_empty() {
                    attrs.push((key, value));
                }
            }
        }
    }
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let end = rest[1..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '#')).map(|e| e + 1);
        let (entity, consumed) = match end {
            Some(e) if rest[e..].starts_with(';') => (&rest[1..e], e + 1),
            Some(e) => (&rest[1..e], e),
            None => (&rest[1..], rest.len()),
        };
        let ch = if let Some(num) = entity.strip_prefix('#') {
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => num.parse().ok(),
            };
            code.and_then(char::from_u32).map(|c| c.to_string())
        } else {
            named_entity(entity).map(str::to_string)
        };
        match ch {
            Some(c) if consumed > 1 => {
                out.push_str(&c);
                rest = &rest[consumed..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" | "ensp" | "emsp" | "thinsp" => " ",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "middot" => "·",
        "bull" => "•",
        "times" => "×",
        "divide" => "÷",
        "laquo" => "«",
        "raquo" => "»",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "yen" => "¥",
        "euro" => "€",
        "pound" => "£",
        "deg" => "°",
        "plusmn" => "±",
        "larr" => "←",
        "rarr" => "→",
        "uarr" => "↑",
        "darr" => "↓",
        "sect" => "§",
        "para" => "¶",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Tree → Markdown events

/// How content is laid out where the converter currently is.
#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    /// Text goes into paragraphs.
    Block,
    /// List item: text goes straight in, nested lists / code / tables stay blocks.
    Item,
    /// Heading or table cell: everything is flattened into one run of text.
    Inline,
}

struct Converter {
    events: Vec<Event<'static>>,
    images: Vec<Arc<Image>>,
    ctx: Vec<Ctx>,
    /// A paragraph is open.
    para: bool,
    /// The last thing emitted ends in whitespace (or nothing yet in this block).
    space: bool,
    truncated: bool,
}

impl Default for Converter {
    fn default() -> Self {
        Self { events: Vec::new(), images: Vec::new(), ctx: vec![Ctx::Block], para: false, space: true, truncated: false }
    }
}

const SKIP: &[&str] = &[
    "head", "script", "style", "noscript", "template", "svg", "canvas", "iframe", "object", "embed", "video",
    "audio", "select", "button", "input", "map", "math",
];
const BLOCK: &[&str] = &[
    "#root", "html", "body", "p", "div", "section", "article", "main", "header", "footer", "nav", "aside", "figure",
    "figcaption", "address", "form", "fieldset", "details", "summary", "center", "dl", "dd", "legend", "caption",
    "tbody", "thead", "tfoot", "tr", "noframes", "hgroup", "search",
];

impl Converter {
    fn mode(&self) -> Ctx {
        *self.ctx.last().unwrap_or(&Ctx::Block)
    }

    fn push(&mut self, e: Event<'static>) {
        if self.events.len() >= MAX_EVENTS {
            self.truncated = true;
            return;
        }
        self.events.push(e);
    }

    fn open_para(&mut self) {
        if self.mode() == Ctx::Block && !self.para {
            self.push(Event::Start(Tag::Paragraph));
            self.para = true;
            self.space = true;
        }
    }

    fn close_para(&mut self) {
        if self.para {
            self.push(Event::End(TagEnd::Paragraph));
            self.para = false;
        }
        self.space = true;
    }

    /// Between blocks: a new paragraph, or a line break where paragraphs don't exist.
    fn block_break(&mut self) {
        if self.mode() == Ctx::Block {
            self.close_para();
        } else if !self.space {
            self.push(Event::HardBreak);
            self.space = true;
        }
    }

    fn with_ctx(&mut self, ctx: Ctx, body: impl FnOnce(&mut Self)) {
        self.ctx.push(ctx);
        body(self);
        self.ctx.pop();
    }

    fn text(&mut self, raw: &str) {
        let mut collapsed = String::with_capacity(raw.len());
        let mut last_space = self.space;
        for c in raw.chars() {
            if c.is_whitespace() {
                if !last_space {
                    collapsed.push(' ');
                    last_space = true;
                }
            } else {
                collapsed.push(c);
                last_space = false;
            }
        }
        // Whitespace between blocks is not content.
        if collapsed.trim().is_empty() && !self.para && self.mode() == Ctx::Block {
            return;
        }
        if collapsed.is_empty() {
            return;
        }
        self.open_para();
        self.space = last_space;
        self.push(Event::Text(collapsed.into()));
    }

    fn inline(&mut self, start: Tag<'static>, body: impl FnOnce(&mut Self)) {
        self.open_para();
        let end = start.to_end();
        self.push(Event::Start(start));
        body(self);
        self.push(Event::End(end));
    }

    fn children(&mut self, tree: &Tree, id: usize) {
        for &child in &tree.nodes[id].children {
            if self.events.len() >= MAX_EVENTS {
                self.truncated = true;
                return;
            }
            self.node(tree, child);
        }
    }

    /// Content laid out as a block of its own, or as separate lines when flattened.
    fn block(&mut self, tree: &Tree, id: usize) {
        self.block_break();
        self.children(tree, id);
        self.block_break();
    }

    fn node(&mut self, tree: &Tree, id: usize) {
        let node = &tree.nodes[id];
        let tag = node.tag.as_str();
        if tag.is_empty() {
            self.text(&node.text);
            return;
        }
        if SKIP.contains(&tag) || tree.attr(id, "hidden").is_some() || tree.attr(id, "aria-hidden") == Some("true") {
            return;
        }
        let mode = self.mode();
        match tag {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if mode == Ctx::Block => {
                self.close_para();
                let level = match tag {
                    "h1" => HeadingLevel::H1,
                    "h2" => HeadingLevel::H2,
                    "h3" => HeadingLevel::H3,
                    "h4" => HeadingLevel::H4,
                    "h5" => HeadingLevel::H5,
                    _ => HeadingLevel::H6,
                };
                self.push(Event::Start(Tag::Heading { level, id: None, classes: Vec::new(), attrs: Vec::new() }));
                self.with_ctx(Ctx::Inline, |c| c.children(tree, id));
                self.push(Event::End(TagEnd::Heading(level)));
                self.space = true;
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "dt" => {
                self.block_break();
                self.inline(Tag::Strong, |c| c.children(tree, id));
                self.block_break();
            }
            "br" => {
                if self.para || mode != Ctx::Block {
                    self.push(Event::HardBreak);
                    self.space = true;
                }
            }
            "hr" if mode == Ctx::Block => {
                self.close_para();
                self.push(Event::Rule);
            }
            "ul" | "ol" | "menu" if mode != Ctx::Inline => {
                self.close_para();
                let start = (tag == "ol").then(|| tree.attr(id, "start").and_then(|s| s.trim().parse().ok()).unwrap_or(1));
                self.push(Event::Start(Tag::List(start)));
                for &child in &tree.nodes[id].children {
                    let c = &tree.nodes[child];
                    if c.tag.is_empty() && c.text.trim().is_empty() {
                        continue;
                    }
                    self.item(tree, child);
                }
                self.push(Event::End(TagEnd::List(start.is_some())));
                self.space = true;
            }
            "li" if mode != Ctx::Inline => self.item(tree, id),
            "blockquote" if mode != Ctx::Inline => {
                self.close_para();
                self.push(Event::Start(Tag::BlockQuote(None)));
                self.with_ctx(Ctx::Block, |c| {
                    c.children(tree, id);
                    c.close_para();
                });
                self.push(Event::End(TagEnd::BlockQuote(None)));
            }
            "pre" => {
                let mut code = String::new();
                collect_text(tree, id, &mut code);
                let code = code.trim_matches('\n').to_string();
                if mode == Ctx::Inline {
                    self.push(Event::Code(code.into()));
                    self.space = false;
                    return;
                }
                self.close_para();
                let lang = code_language(tree, id).unwrap_or_default();
                self.push(Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang.into()))));
                self.push(Event::Text(code.into()));
                self.push(Event::End(TagEnd::CodeBlock));
                self.space = true;
            }
            "code" | "kbd" | "samp" | "tt" | "var" => {
                let mut code = String::new();
                collect_text(tree, id, &mut code);
                let code = code.split_whitespace().collect::<Vec<_>>().join(" ");
                if !code.is_empty() {
                    self.open_para();
                    self.push(Event::Code(code.into()));
                    self.space = false;
                }
            }
            "strong" | "b" => self.inline(Tag::Strong, |c| c.children(tree, id)),
            "em" | "i" | "cite" | "dfn" => self.inline(Tag::Emphasis, |c| c.children(tree, id)),
            "s" | "del" | "strike" => self.inline(Tag::Strikethrough, |c| c.children(tree, id)),
            "a" => match tree.attr(id, "href").filter(|h| !h.is_empty() && !h.starts_with("javascript:")) {
                Some(href) => {
                    let link = Tag::Link {
                        link_type: LinkType::Inline,
                        dest_url: href.to_string().into(),
                        title: CowStr::Borrowed(""),
                        id: CowStr::Borrowed(""),
                    };
                    self.inline(link, |c| c.children(tree, id));
                }
                None => self.children(tree, id),
            },
            "img" => self.image(tree, id),
            "table" => self.table(tree, id),
            "hr" | "ul" | "ol" | "menu" | "li" | "blockquote" => self.block(tree, id),
            t if BLOCK.contains(&t) => self.block(tree, id),
            _ => self.children(tree, id),
        }
    }

    fn item(&mut self, tree: &Tree, id: usize) {
        self.close_para();
        self.push(Event::Start(Tag::Item));
        self.space = true;
        self.with_ctx(Ctx::Item, |c| {
            if tree.nodes[id].tag == "li" {
                c.children(tree, id);
            } else {
                c.node(tree, id);
            }
        });
        self.push(Event::End(TagEnd::Item));
        self.space = true;
    }

    fn image(&mut self, tree: &Tree, id: usize) {
        let Some(src) = tree.attr(id, "src").filter(|s| !s.is_empty()) else { return };
        let alt = tree.attr(id, "alt").unwrap_or("").trim().to_string();
        let dest = match src.strip_prefix("data:") {
            Some(data) => match decode_data_image(data) {
                Some(image) => {
                    self.images.push(Arc::new(image));
                    format!("inline-image:{}", self.images.len() - 1)
                }
                None => return,
            },
            None => src.to_string(),
        };
        let tag = Tag::Image {
            link_type: LinkType::Inline,
            dest_url: dest.into(),
            title: CowStr::Borrowed(""),
            id: CowStr::Borrowed(""),
        };
        self.push(Event::Start(tag));
        if !alt.is_empty() {
            self.push(Event::Text(alt.into()));
        }
        self.push(Event::End(TagEnd::Image));
        self.space = false;
    }

    fn table(&mut self, tree: &Tree, id: usize) {
        // Rows from thead/tbody/tfoot or directly under <table>.
        let mut rows = Vec::new();
        let mut caption = None;
        for &child in &tree.nodes[id].children {
            match tree.nodes[child].tag.as_str() {
                "tr" => rows.push(child),
                "thead" | "tbody" | "tfoot" => {
                    rows.extend(tree.nodes[child].children.iter().copied().filter(|&r| tree.nodes[r].tag == "tr"));
                }
                "caption" => caption = Some(child),
                _ => {}
            }
        }
        let cells = |r: usize| -> Vec<usize> {
            tree.nodes[r].children.iter().copied().filter(|&c| matches!(tree.nodes[c].tag.as_str(), "td" | "th")).collect()
        };
        let cols = rows.iter().map(|&r| cells(r).len()).max().unwrap_or(0);
        // Layout tables (a single column) and tables inside cells: just their content.
        if self.mode() == Ctx::Inline || cols <= 1 {
            self.block_break();
            for &r in &rows {
                for c in cells(r) {
                    self.block(tree, c);
                }
            }
            return;
        }
        self.close_para();
        if let Some(cap) = caption {
            self.with_ctx(Ctx::Block, |c| {
                c.inline(Tag::Strong, |c| c.children(tree, cap));
                c.close_para();
            });
        }
        self.push(Event::Start(Tag::Table(vec![Alignment::None; cols])));
        // Markdown tables have exactly one header row: the first.
        for (n, &r) in rows.iter().enumerate() {
            let row = if n == 0 { Tag::TableHead } else { Tag::TableRow };
            let end = row.to_end();
            self.push(Event::Start(row));
            for c in cells(r) {
                self.push(Event::Start(Tag::TableCell));
                self.space = true;
                self.with_ctx(Ctx::Inline, |cx| cx.children(tree, c));
                self.push(Event::End(TagEnd::TableCell));
            }
            self.push(Event::End(end));
        }
        self.push(Event::End(TagEnd::Table));
        self.space = true;
    }
}

fn collect_text(tree: &Tree, id: usize, out: &mut String) {
    for &c in &tree.nodes[id].children {
        let n = &tree.nodes[c];
        match n.tag.as_str() {
            "" => out.push_str(&n.text),
            "br" => out.push('\n'),
            t if SKIP.contains(&t) => {}
            _ => collect_text(tree, c, out),
        }
    }
}

/// `<pre><code class="language-rust">` / `class="lang-js"` / `class="hljs python"`.
fn code_language(tree: &Tree, pre: usize) -> Option<String> {
    let mut ids = vec![pre];
    ids.extend(tree.nodes[pre].children.iter().copied().filter(|&c| tree.nodes[c].tag == "code"));
    for id in ids {
        if let Some(class) = tree.attr(id, "class") {
            for word in class.split_whitespace() {
                let lang = word.strip_prefix("language-").or_else(|| word.strip_prefix("lang-")).unwrap_or(word);
                if crate::highlight::lang_for_name(lang).is_some() {
                    return Some(lang.to_string());
                }
            }
        }
        if let Some(lang) = tree.attr(id, "data-lang").or_else(|| tree.attr(id, "lang")) {
            return Some(lang.to_string());
        }
    }
    None
}

fn decode_data_image(data: &str) -> Option<Image> {
    let (meta, payload) = data.split_once(',')?;
    let format = match meta.split(';').next()?.trim().to_ascii_lowercase().as_str() {
        "image/png" => gpui::ImageFormat::Png,
        "image/jpeg" | "image/jpg" => gpui::ImageFormat::Jpeg,
        "image/gif" => gpui::ImageFormat::Gif,
        "image/webp" => gpui::ImageFormat::Webp,
        "image/bmp" => gpui::ImageFormat::Bmp,
        "image/svg+xml" => gpui::ImageFormat::Svg,
        _ => return None,
    };
    let bytes = if meta.ends_with(";base64") {
        let clean: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
        base64_decode(&clean)?
    } else {
        percent_decode(payload)
    };
    Some(Image::from_bytes(format, bytes))
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let digits = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= value(c)? << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..chunk.len().saturating_sub(1)]);
    }
    Some(out)
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(html: &str) -> String {
        let page = convert(html);
        page.events
            .iter()
            .map(|e| match e {
                Event::Start(t) => format!("<{}>", tag_label(t)),
                Event::End(t) => format!("</{}>", end_label(t)),
                Event::Text(t) => t.to_string(),
                Event::Code(t) => format!("`{t}`"),
                Event::HardBreak => "\\n".into(),
                Event::Rule => "---".into(),
                _ => "?".into(),
            })
            .collect()
    }

    fn tag_label(t: &Tag) -> String {
        match t {
            Tag::Paragraph => "p".into(),
            Tag::Heading { level, .. } => format!("h{}", *level as usize),
            Tag::List(Some(_)) => "ol".into(),
            Tag::List(None) => "ul".into(),
            Tag::Item => "li".into(),
            Tag::Strong => "b".into(),
            Tag::Emphasis => "i".into(),
            Tag::Link { dest_url, .. } => format!("a {dest_url}"),
            Tag::Image { dest_url, .. } => format!("img {dest_url}"),
            Tag::Table(_) => "table".into(),
            Tag::TableHead => "thead".into(),
            Tag::TableRow => "tr".into(),
            Tag::TableCell => "td".into(),
            Tag::CodeBlock(CodeBlockKind::Fenced(l)) => format!("pre {l}"),
            Tag::BlockQuote(_) => "quote".into(),
            _ => "x".into(),
        }
    }

    fn end_label(t: &TagEnd) -> String {
        match t {
            TagEnd::Paragraph => "p".into(),
            TagEnd::Heading(level) => format!("h{}", *level as usize),
            TagEnd::List(true) => "ol".into(),
            TagEnd::List(false) => "ul".into(),
            TagEnd::Item => "li".into(),
            TagEnd::Strong => "b".into(),
            TagEnd::Emphasis => "i".into(),
            TagEnd::Link => "a".into(),
            TagEnd::Image => "img".into(),
            TagEnd::Table => "table".into(),
            TagEnd::TableHead => "thead".into(),
            TagEnd::TableRow => "tr".into(),
            TagEnd::TableCell => "td".into(),
            TagEnd::CodeBlock => "pre".into(),
            TagEnd::BlockQuote(_) => "quote".into(),
            _ => "x".into(),
        }
    }

    #[test]
    fn paragraphs_headings_and_inline() {
        let s = summary("<html><head><title> 测试 &amp; 页面 </title><style>p{}</style></head><body><h1>标题</h1><p>一段 <b>加粗</b>\n 和 <a href='x.html'>链接</a><p>第二段<br>换行</body>");
        assert_eq!(s, "<h1>标题</h1><p>一段 <b>加粗</b> 和 <a x.html>链接</a></p><p>第二段\\n换行</p>");
        assert_eq!(convert("<title>测试 &amp; 页面</title>").title.as_deref(), Some("测试 & 页面"));
    }

    #[test]
    fn lists_without_end_tags() {
        let s = summary("<ul><li>一<li>二<ol start=3><li>三</ol></ul>");
        assert_eq!(s, "<ul><li>一</li><li>二<ol><li>三</li></ol></li></ul>");
    }

    #[test]
    fn tables_and_code() {
        let s = summary("<table><tr><th>名称<th>值<tr><td>a<td><b>1</b></table><pre><code class=\"language-rust\">fn main() {\n}</code></pre>");
        assert_eq!(s, "<table><thead><td>名称</td><td>值</td></thead><tr><td>a</td><td><b>1</b></td></tr></table><pre rust>fn main() {\n}</pre>");
    }

    #[test]
    fn scripts_comments_entities() {
        let s = summary("<!-- c --><script>var a = '<p>';</script><div>A&lt;B &#x4e2d;&#25991; &copy; &unknown; R&D</div>");
        assert_eq!(s, "<p>A<B 中文 © &unknown; R&D</p>");
    }

    #[test]
    fn data_images() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let page = convert(&format!("<p><img src=\"data:image/png;base64,{png}\" alt=\"点\"><img src=\"a.png\"></p>"));
        assert_eq!(page.images.len(), 1);
        let s = summary(&format!("<img src=\"data:image/png;base64,{png}\" alt=\"点\">"));
        assert_eq!(s, "<img inline-image:0>点</img>");
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGk").unwrap(), b"hi");
    }
}
