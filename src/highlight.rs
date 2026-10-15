//! Lightweight syntax highlighting and pretty-printing for previews.
//!
//! A small table-driven lexer instead of syntect / tree-sitter keeps the binary
//! small: it recognises comments, strings, numbers, keywords, types, function
//! calls and a few language-specific forms (decorators, `$vars`, JSON keys,
//! markup tags, YAML / INI keys, diff lines). Good enough to make code readable.

use std::ops::Range;

use gpui::{FontStyle, HighlightStyle, Hsla, rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    Type,
    Literal,
    String,
    Number,
    Comment,
    Function,
    Meta,
    Key,
    Tag,
    Attr,
    Inserted,
    Deleted,
}

pub type Spans = Vec<(Range<usize>, Token)>;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Code,
    Markup,
    Yaml,
    Ini,
    Diff,
}

pub struct Lang {
    pub name: &'static str,
    mode: Mode,
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    quotes: &'static [u8],
    /// Quotes whose strings may span lines (JS template literals).
    multiline_quotes: &'static [u8],
    triple_quotes: bool,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    literals: &'static [&'static str],
    ignore_case: bool,
    /// Capitalised identifiers are types (C-family convention).
    caps_types: bool,
    decorators: bool,
    preprocessor: bool,
    dollar_vars: bool,
    /// `"string":` is an object key.
    json_keys: bool,
    /// `foo!(` is a macro call (Rust).
    bang_macros: bool,
    /// Identifiers may contain '-' (CSS); `name:` inside braces is a property.
    css: bool,
}

const BASE: Lang = Lang {
    name: "",
    mode: Mode::Code,
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: b"\"'",
    multiline_quotes: b"",
    triple_quotes: false,
    keywords: &[],
    types: &[],
    literals: &["true", "false", "null"],
    ignore_case: false,
    caps_types: true,
    decorators: false,
    preprocessor: false,
    dollar_vars: false,
    json_keys: false,
    bang_macros: false,
    css: false,
};

const RUST: Lang = Lang {
    name: "Rust",
    quotes: b"\"",
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for",
        "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "static",
        "struct", "super", "trait", "type", "unsafe", "use", "where", "while", "yield",
    ],
    types: &[
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool",
        "char", "str", "Self",
    ],
    literals: &["true", "false", "None", "Some", "Ok", "Err"],
    bang_macros: true,
    ..BASE
};

const JS: Lang = Lang {
    name: "JavaScript / TypeScript",
    quotes: b"\"'",
    multiline_quotes: b"`",
    keywords: &[
        "abstract", "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger",
        "declare", "default", "delete", "do", "else", "enum", "export", "extends", "finally", "for", "from",
        "function", "get", "if", "implements", "import", "in", "instanceof", "interface", "keyof", "let", "namespace",
        "new", "of", "private", "protected", "public", "readonly", "return", "set", "static", "super", "switch",
        "this", "throw", "try", "type", "typeof", "var", "void", "while", "with", "yield",
    ],
    types: &["string", "number", "boolean", "any", "unknown", "never", "object", "symbol", "bigint"],
    literals: &["true", "false", "null", "undefined", "NaN", "Infinity"],
    decorators: true,
    ..BASE
};

const PYTHON: Lang = Lang {
    name: "Python",
    line_comments: &["#"],
    block_comment: None,
    triple_quotes: true,
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def", "del", "elif", "else",
        "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "match", "nonlocal",
        "not", "or", "pass", "raise", "return", "self", "try", "while", "with", "yield",
    ],
    types: &["int", "str", "float", "bool", "list", "dict", "set", "tuple", "bytes", "object"],
    literals: &["True", "False", "None"],
    decorators: true,
    ..BASE
};

const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "class", "const", "constexpr", "continue", "default", "delete", "do", "else", "enum",
    "explicit", "extern", "for", "friend", "goto", "if", "inline", "namespace", "new", "noexcept", "operator",
    "override", "private", "protected", "public", "register", "return", "sizeof", "static", "struct", "switch",
    "template", "this", "throw", "try", "catch", "typedef", "typename", "union", "using", "virtual", "volatile",
    "while", "nullptr",
];

const C: Lang = Lang {
    name: "C / C++",
    keywords: C_KEYWORDS,
    types: &[
        "void", "int", "char", "short", "long", "float", "double", "bool", "signed", "unsigned", "size_t", "int8_t",
        "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "wchar_t",
    ],
    literals: &["true", "false", "NULL", "nullptr"],
    preprocessor: true,
    ..BASE
};

const JAVA: Lang = Lang {
    name: "Java / Kotlin / Scala",
    keywords: &[
        "abstract", "assert", "break", "case", "catch", "class", "companion", "const", "continue", "data", "default",
        "do", "else", "enum", "extends", "final", "finally", "for", "fun", "if", "implements", "import", "in",
        "instanceof", "interface", "internal", "is", "lateinit", "new", "object", "open", "override", "package",
        "private", "protected", "public", "return", "sealed", "static", "super", "switch", "synchronized", "this",
        "throw", "throws", "try", "val", "var", "when", "while", "def", "trait", "lazy", "implicit",
    ],
    types: &["void", "int", "long", "short", "byte", "char", "float", "double", "boolean", "String", "Unit", "Int", "Any"],
    decorators: true,
    ..BASE
};

const CSHARP: Lang = Lang {
    name: "C#",
    keywords: &[
        "abstract", "as", "async", "await", "base", "break", "case", "catch", "class", "const", "continue", "default",
        "delegate", "do", "else", "enum", "event", "explicit", "extern", "finally", "fixed", "for", "foreach", "get",
        "goto", "if", "implicit", "in", "init", "interface", "internal", "is", "lock", "namespace", "new", "operator",
        "out", "override", "params", "partial", "private", "protected", "public", "readonly", "record", "ref",
        "return", "sealed", "set", "sizeof", "static", "struct", "switch", "this", "throw", "try", "typeof", "using",
        "var", "virtual", "void", "volatile", "when", "where", "while", "yield",
    ],
    types: &["int", "long", "short", "byte", "char", "float", "double", "decimal", "bool", "string", "object", "dynamic"],
    preprocessor: true,
    ..BASE
};

const GO: Lang = Lang {
    name: "Go",
    multiline_quotes: b"`",
    keywords: &[
        "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go",
        "goto", "if", "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type",
        "var",
    ],
    types: &[
        "bool", "byte", "complex64", "complex128", "error", "float32", "float64", "int", "int8", "int16", "int32",
        "int64", "rune", "string", "uint", "uint8", "uint16", "uint32", "uint64", "uintptr", "any",
    ],
    literals: &["true", "false", "nil", "iota"],
    ..BASE
};

const SWIFT: Lang = Lang {
    name: "Swift / Dart",
    keywords: &[
        "as", "async", "await", "break", "case", "catch", "class", "continue", "default", "defer", "do", "else", "enum",
        "extension", "fallthrough", "final", "for", "func", "guard", "if", "import", "in", "init", "is", "let",
        "mutating", "private", "protocol", "public", "required", "return", "self", "static", "struct", "super",
        "switch", "throw", "throws", "try", "var", "where", "while", "late", "dynamic", "void", "extends", "with",
        "factory", "new", "this",
    ],
    literals: &["true", "false", "nil", "null"],
    decorators: true,
    ..BASE
};

const PHP: Lang = Lang {
    name: "PHP",
    line_comments: &["//", "#"],
    keywords: &[
        "abstract", "and", "as", "break", "case", "catch", "class", "clone", "const", "continue", "declare", "default",
        "do", "echo", "else", "elseif", "enum", "extends", "final", "finally", "fn", "for", "foreach", "function",
        "global", "if", "implements", "include", "instanceof", "interface", "match", "namespace", "new", "or",
        "print", "private", "protected", "public", "readonly", "require", "return", "static", "switch", "throw",
        "trait", "try", "use", "while", "yield",
    ],
    literals: &["true", "false", "null", "TRUE", "FALSE", "NULL"],
    dollar_vars: true,
    ..BASE
};

const RUBY: Lang = Lang {
    name: "Ruby",
    line_comments: &["#"],
    block_comment: None,
    keywords: &[
        "alias", "and", "begin", "break", "case", "class", "def", "do", "else", "elsif", "end", "ensure", "for", "if",
        "in", "module", "next", "not", "or", "redo", "rescue", "retry", "return", "self", "super", "then", "unless",
        "until", "when", "while", "yield", "require", "attr_accessor", "puts",
    ],
    literals: &["true", "false", "nil"],
    ..BASE
};

const LUA: Lang = Lang {
    name: "Lua",
    line_comments: &["--"],
    block_comment: Some(("--[[", "]]")),
    keywords: &[
        "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in", "local", "not", "or",
        "repeat", "return", "then", "until", "while",
    ],
    literals: &["true", "false", "nil"],
    caps_types: false,
    ..BASE
};

const SHELL: Lang = Lang {
    name: "Shell",
    line_comments: &["#"],
    block_comment: None,
    keywords: &[
        "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function",
        "return", "local", "export", "readonly", "echo", "exit", "source", "set", "unset", "shift", "cd", "sudo",
    ],
    literals: &["true", "false"],
    caps_types: false,
    dollar_vars: true,
    ..BASE
};

const POWERSHELL: Lang = Lang {
    name: "PowerShell",
    line_comments: &["#"],
    block_comment: Some(("<#", "#>")),
    keywords: &[
        "begin", "break", "catch", "class", "continue", "data", "do", "dynamicparam", "else", "elseif", "end", "exit",
        "filter", "finally", "for", "foreach", "from", "function", "if", "in", "param", "process", "return", "switch",
        "throw", "trap", "try", "until", "using", "while", "-eq", "-ne", "-gt", "-lt", "-and", "-or", "-not",
    ],
    literals: &["$true", "$false", "$null"],
    ignore_case: true,
    caps_types: false,
    dollar_vars: true,
    ..BASE
};

const BATCH: Lang = Lang {
    name: "Batch",
    line_comments: &["::", "rem ", "@rem "],
    block_comment: None,
    quotes: b"\"",
    keywords: &[
        "echo", "set", "if", "else", "goto", "call", "exit", "for", "in", "do", "not", "exist", "errorlevel",
        "setlocal", "endlocal", "pause", "shift", "start", "cd", "pushd", "popd", "equ", "neq", "defined",
    ],
    literals: &["on", "off"],
    ignore_case: true,
    caps_types: false,
    ..BASE
};

const SQL: Lang = Lang {
    name: "SQL",
    line_comments: &["--"],
    keywords: &[
        "add", "all", "alter", "and", "as", "asc", "begin", "between", "by", "case", "commit", "create", "cross",
        "database", "declare", "default", "delete", "desc", "distinct", "drop", "else", "end", "exists", "foreign",
        "from", "full", "function", "group", "having", "if", "in", "index", "inner", "insert", "into", "is", "join",
        "key", "left", "like", "limit", "not", "offset", "on", "or", "order", "outer", "primary", "procedure",
        "references", "returns", "right", "rollback", "select", "set", "table", "then", "trigger", "union", "unique",
        "update", "values", "view", "when", "where", "with",
    ],
    types: &["int", "integer", "bigint", "varchar", "char", "text", "date", "datetime", "timestamp", "decimal", "float", "boolean"],
    literals: &["null", "true", "false"],
    ignore_case: true,
    caps_types: false,
    ..BASE
};

const JSON: Lang = Lang {
    name: "JSON",
    keywords: &[],
    caps_types: false,
    json_keys: true,
    ..BASE
};

const CSS: Lang = Lang {
    name: "CSS",
    line_comments: &[],
    keywords: &["important"],
    literals: &[],
    caps_types: false,
    css: true,
    ..BASE
};

const MARKUP: Lang = Lang { name: "XML / HTML", mode: Mode::Markup, ..BASE };
const YAML: Lang = Lang { name: "YAML", mode: Mode::Yaml, ..BASE };
const INI: Lang = Lang { name: "INI / TOML", mode: Mode::Ini, ..BASE };
const DIFF: Lang = Lang { name: "Diff", mode: Mode::Diff, ..BASE };

pub fn lang_for_ext(ext: &str) -> Option<&'static Lang> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "rs" => &RUST,
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "mts" | "cts" => &JS,
        "py" | "pyw" | "pyi" => &PYTHON,
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "ino" => &C,
        "java" | "kt" | "kts" | "scala" | "gradle" | "groovy" => &JAVA,
        "cs" => &CSHARP,
        "go" => &GO,
        "swift" | "dart" => &SWIFT,
        "php" => &PHP,
        "rb" => &RUBY,
        "lua" => &LUA,
        "sh" | "bash" | "zsh" | "fish" => &SHELL,
        "ps1" | "psm1" | "psd1" => &POWERSHELL,
        "bat" | "cmd" => &BATCH,
        "sql" => &SQL,
        "json" | "jsonc" | "json5" | "geojson" | "ipynb" => &JSON,
        "css" | "scss" | "less" => &CSS,
        "xml" | "html" | "htm" | "xhtml" | "xaml" | "csproj" | "vbproj" | "props" | "targets" | "config" | "vue"
        | "svelte" | "plist" | "resx" | "wsdl" | "xsd" => &MARKUP,
        "yaml" | "yml" => &YAML,
        "toml" | "ini" | "cfg" | "conf" | "properties" | "env" | "editorconfig" | "gitconfig" | "inf" | "reg" => &INI,
        "diff" | "patch" => &DIFF,
        _ => return None,
    })
}

/// Language from a Markdown fence tag such as "rust" or "py".
pub fn lang_for_name(name: &str) -> Option<&'static Lang> {
    let name = name.trim().to_ascii_lowercase();
    match name.as_str() {
        "rust" => Some(&RUST),
        "javascript" | "typescript" | "node" => Some(&JS),
        "python" | "python3" => Some(&PYTHON),
        "c++" => Some(&C),
        "csharp" | "c#" => Some(&CSHARP),
        "golang" => Some(&GO),
        "kotlin" => Some(&JAVA),
        "ruby" => Some(&RUBY),
        "shell" | "console" | "terminal" => Some(&SHELL),
        "powershell" | "pwsh" => Some(&POWERSHELL),
        "batch" | "dos" => Some(&BATCH),
        "html" | "xml" | "svg" => Some(&MARKUP),
        _ => lang_for_ext(&name),
    }
}

/// Guess from a `#!` line when there's no useful extension.
pub fn lang_for_shebang(first_line: &str) -> Option<&'static Lang> {
    let line = first_line.strip_prefix("#!")?;
    if line.contains("python") {
        Some(&PYTHON)
    } else if line.contains("node") || line.contains("deno") {
        Some(&JS)
    } else if line.contains("pwsh") {
        Some(&POWERSHELL)
    } else if line.contains("sh") {
        Some(&SHELL)
    } else {
        None
    }
}

pub fn is_json(lang: &Lang) -> bool {
    lang.json_keys
}

pub fn is_markup(lang: &Lang) -> bool {
    lang.mode == Mode::Markup
}

// ---------------------------------------------------------------------------
// Lexing

pub fn highlight(text: &str, lang: &Lang) -> Spans {
    let mut out = Vec::new();
    match lang.mode {
        Mode::Code => scan_code(text, lang, &mut out),
        Mode::Markup => scan_markup(text, &mut out),
        Mode::Yaml => scan_lines(text, &mut out, yaml_line),
        Mode::Ini => scan_lines(text, &mut out, ini_line),
        Mode::Diff => scan_lines(text, &mut out, diff_line),
    }
    out
}

fn starts_with(b: &[u8], i: usize, pat: &str, ignore_case: bool) -> bool {
    let p = pat.as_bytes();
    b.len() >= i + p.len()
        && if ignore_case {
            b[i..i + p.len()].eq_ignore_ascii_case(p)
        } else {
            &b[i..i + p.len()] == p
        }
}

fn find(b: &[u8], from: usize, pat: &str) -> Option<usize> {
    let p = pat.as_bytes();
    (from..=b.len().saturating_sub(p.len())).find(|&i| &b[i..i + p.len()] == p)
}

fn line_end(b: &[u8], from: usize) -> usize {
    b[from..].iter().position(|&c| c == b'\n').map(|p| from + p).unwrap_or(b.len())
}

fn next_non_space(b: &[u8], from: usize) -> Option<u8> {
    b[from..].iter().copied().find(|c| !matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
}

/// Non-ASCII bytes count as identifier characters, so multi-byte characters
/// are never split and token boundaries always land on char boundaries.
fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_ident(c: u8, css: bool) -> bool {
    is_ident_start(c) || c.is_ascii_digit() || (css && c == b'-')
}

fn contains(list: &[&str], word: &str, ignore_case: bool) -> bool {
    if ignore_case {
        list.iter().any(|k| k.eq_ignore_ascii_case(word))
    } else {
        list.contains(&word)
    }
}

fn scan_code(text: &str, l: &Lang, out: &mut Spans) {
    let b = text.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut line_start = true;
    let mut depth = 0i32; // brace depth, for CSS properties
    while i < n {
        let c = b[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if matches!(c, b' ' | b'\t' | b'\r') {
            i += 1;
            continue;
        }
        if let Some((open, close)) = l.block_comment {
            if starts_with(b, i, open, false) {
                let end = find(b, i + open.len(), close).map(|e| e + close.len()).unwrap_or(n);
                out.push((i..end, Token::Comment));
                i = end;
                continue;
            }
        }
        // Word-like markers (batch `rem`) only count at the start of a line.
        let word_marker = |p: &&str| p.as_bytes()[0].is_ascii_alphabetic() || p.starts_with('@');
        if l
            .line_comments
            .iter()
            .any(|p| starts_with(b, i, p, l.ignore_case) && (line_start || !word_marker(p)))
        {
            let end = line_end(b, i);
            out.push((i..end, Token::Comment));
            i = end;
            continue;
        }
        if l.preprocessor && line_start && c == b'#' {
            let end = line_end(b, i);
            out.push((i..end, Token::Meta));
            i = end;
            continue;
        }
        line_start = false;
        if l.triple_quotes && (starts_with(b, i, "\"\"\"", false) || starts_with(b, i, "'''", false)) {
            let quote = &text[i..i + 3];
            let end = find(b, i + 3, quote).map(|e| e + 3).unwrap_or(n);
            out.push((i..end, Token::String));
            i = end;
            continue;
        }
        if l.quotes.contains(&c) || l.multiline_quotes.contains(&c) {
            let multiline = l.multiline_quotes.contains(&c);
            let mut j = i + 1;
            while j < n {
                match b[j] {
                    b'\\' => j += 2,
                    q if q == c => {
                        j += 1;
                        break;
                    }
                    b'\n' if !multiline => break,
                    _ => j += 1,
                }
            }
            let j = j.min(n);
            let kind = if l.json_keys && next_non_space(b, j) == Some(b':') { Token::Key } else { Token::String };
            out.push((i..j, kind));
            i = j;
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && i + 1 < n && b[i + 1].is_ascii_digit()) {
            let mut j = i + 1;
            while j < n {
                let d = b[j];
                if d.is_ascii_alphanumeric() || d == b'.' || d == b'_' || d == b'%' {
                    j += 1;
                } else if matches!(d, b'+' | b'-') && matches!(b[j - 1], b'e' | b'E') && !text[i..j].starts_with("0x") {
                    j += 1;
                } else {
                    break;
                }
            }
            out.push((i..j, Token::Number));
            i = j;
            continue;
        }
        if l.css && c == b'#' && i + 1 < n && b[i + 1].is_ascii_hexdigit() && depth > 0 {
            let mut j = i + 1;
            while j < n && b[j].is_ascii_alphanumeric() {
                j += 1;
            }
            out.push((i..j, Token::Number));
            i = j;
            continue;
        }
        if (l.decorators || l.css) && c == b'@' || l.dollar_vars && c == b'$' {
            let mut j = i + 1;
            if c == b'$' && j < n && b[j] == b'{' {
                j = b[j..].iter().position(|&x| x == b'}').map(|p| j + p + 1).unwrap_or(n);
            } else {
                while j < n && (is_ident(b[j], l.css) || b[j] == b'.') {
                    j += 1;
                }
            }
            if j > i + 1 {
                let word = &text[i..j];
                let kind = if contains(l.literals, word, l.ignore_case) {
                    Token::Literal
                } else if l.css {
                    Token::Keyword
                } else {
                    Token::Meta
                };
                out.push((i..j, kind));
                i = j;
                continue;
            }
        }
        if is_ident_start(c) || (l.ignore_case && c == b'-' && i + 1 < n && b[i + 1].is_ascii_alphabetic()) {
            let mut j = i + 1;
            while j < n && is_ident(b[j], l.css || (l.ignore_case && c == b'-')) {
                j += 1;
            }
            let word = &text[i..j];
            let next = b.get(j).copied();
            let kind = if contains(l.keywords, word, l.ignore_case) {
                Some(Token::Keyword)
            } else if contains(l.literals, word, l.ignore_case) {
                Some(Token::Literal)
            } else if contains(l.types, word, l.ignore_case) {
                Some(Token::Type)
            } else if l.css && depth > 0 && next_non_space(b, j) == Some(b':') {
                Some(Token::Attr)
            } else if next == Some(b'(') {
                Some(Token::Function)
            } else if l.bang_macros && next == Some(b'!') && matches!(b.get(j + 1), Some(b'(' | b'[' | b'{')) {
                Some(Token::Function)
            } else if l.caps_types && c.is_ascii_uppercase() && word.bytes().any(|x| x.is_ascii_lowercase()) {
                Some(Token::Type)
            } else {
                None
            };
            if let Some(kind) = kind {
                out.push((i..j, kind));
            }
            i = j;
            continue;
        }
        match c {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
}

fn scan_markup(text: &str, out: &mut Spans) {
    let b = text.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n {
        if starts_with(b, i, "<!--", false) {
            let end = find(b, i + 4, "-->").map(|e| e + 3).unwrap_or(n);
            out.push((i..end, Token::Comment));
            i = end;
            continue;
        }
        if b[i] == b'<' && i + 1 < n && (b[i + 1].is_ascii_alphabetic() || matches!(b[i + 1], b'/' | b'?' | b'!')) {
            // Tag name
            let mut j = i + 1;
            while j < n && matches!(b[j], b'/' | b'?' | b'!') {
                j += 1;
            }
            while j < n && (is_ident(b[j], true) || b[j] == b':' || b[j] == b'.') {
                j += 1;
            }
            out.push((i..j, Token::Tag));
            // Attributes until '>'
            while j < n && b[j] != b'>' {
                match b[j] {
                    b'"' | b'\'' => {
                        let q = b[j];
                        let end = b[j + 1..].iter().position(|&c| c == q).map(|p| j + 1 + p + 1).unwrap_or(n);
                        out.push((j..end, Token::String));
                        j = end;
                    }
                    c if is_ident_start(c) => {
                        let s = j;
                        while j < n && (is_ident(b[j], true) || b[j] == b':') {
                            j += 1;
                        }
                        out.push((s..j, Token::Attr));
                    }
                    _ => j += 1,
                }
            }
            let end = (j + 1).min(n);
            if j < n {
                let close_start = if j > 0 && matches!(b[j - 1], b'/' | b'?') { j - 1 } else { j };
                out.push((close_start..end, Token::Tag));
            }
            i = end;
            continue;
        }
        if b[i] == b'&' {
            if let Some(p) = b[i..(i + 12).min(n)].iter().position(|&c| c == b';') {
                out.push((i..i + p + 1, Token::Literal));
                i += p + 1;
                continue;
            }
        }
        i += 1;
    }
}

/// Run a per-line lexer; `f` gets the line text and its byte offset.
fn scan_lines(text: &str, out: &mut Spans, f: fn(&str, usize, &mut Spans)) {
    let mut offset = 0;
    for line in text.split('\n') {
        f(line, offset, out);
        offset += line.len() + 1;
    }
}

fn scalar(value: &str, at: usize, out: &mut Spans) {
    let v = value.trim();
    if v.is_empty() {
        return;
    }
    let start = at + (value.len() - value.trim_start().len());
    let range = start..start + v.len();
    let lower = v.to_ascii_lowercase();
    if v.starts_with(['"', '\'']) {
        out.push((range, Token::String));
    } else if matches!(lower.as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~") {
        out.push((range, Token::Literal));
    } else if v.parse::<f64>().is_ok() || v.starts_with("0x") {
        out.push((range, Token::Number));
    } else if v.starts_with(['&', '*', '!']) {
        out.push((range, Token::Meta));
    }
}

/// Split off a trailing ` # comment` (outside quotes).
fn split_comment(line: &str, markers: &[char]) -> (usize, Option<usize>) {
    let mut quote: Option<char> = None;
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if markers.contains(&c) && (i == 0 || line[..i].ends_with([' ', '\t'])) => return (i, Some(i)),
            None => {}
        }
    }
    (line.len(), None)
}

fn yaml_line(line: &str, at: usize, out: &mut Spans) {
    let (code_end, comment) = split_comment(line, &['#']);
    let code = &line[..code_end];
    let trimmed = code.trim_start();
    let indent = code.len() - trimmed.len();
    if trimmed == "---" || trimmed == "..." {
        out.push((at + indent..at + code_end, Token::Meta));
    } else {
        let mut rest_start = indent;
        let mut body = trimmed;
        if let Some(stripped) = body.strip_prefix("- ") {
            rest_start += 2;
            body = stripped;
        }
        let key_end = body
            .char_indices()
            .find(|&(i, c)| c == ':' && (i + 1 == body.len() || body[i + 1..].starts_with([' ', '\t'])))
            .map(|(i, _)| i);
        match key_end {
            Some(k) if !body.starts_with(['"', '\'', '{', '[']) || body[..k].ends_with(['"', '\'']) => {
                out.push((at + rest_start..at + rest_start + k, Token::Key));
                scalar(&body[k + 1..], at + rest_start + k + 1, out);
            }
            _ => scalar(body, at + rest_start, out),
        }
    }
    if let Some(c) = comment {
        out.push((at + c..at + line.len(), Token::Comment));
    }
}

fn ini_line(line: &str, at: usize, out: &mut Spans) {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if trimmed.starts_with(['#', ';']) {
        out.push((at + indent..at + line.len(), Token::Comment));
        return;
    }
    if trimmed.starts_with('[') {
        let end = trimmed.find(']').map(|e| e + 1).unwrap_or(trimmed.len());
        out.push((at + indent..at + indent + end, Token::Tag));
        return;
    }
    let (code_end, comment) = split_comment(line, &['#', ';']);
    let code = &line[..code_end];
    if let Some(eq) = code.find(['=', ':']) {
        let key = code[..eq].trim_end();
        if !key.trim().is_empty() {
            out.push((at + indent..at + key.len(), Token::Key));
        }
        scalar(&code[eq + 1..], at + eq + 1, out);
    }
    if let Some(c) = comment {
        out.push((at + c..at + line.len(), Token::Comment));
    }
}

fn diff_line(line: &str, at: usize, out: &mut Spans) {
    let range = at..at + line.len();
    let kind = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") || line.starts_with("index ") {
        Token::Keyword
    } else if line.starts_with("@@") {
        Token::Meta
    } else if line.starts_with('+') {
        Token::Inserted
    } else if line.starts_with('-') {
        Token::Deleted
    } else {
        return;
    };
    out.push((range, kind));
}

/// Split whole-text spans into per-line spans (offsets relative to each line).
pub fn by_line(text: &str, spans: &Spans) -> Vec<Spans> {
    let mut starts = vec![0usize];
    starts.extend(text.bytes().enumerate().filter(|&(_, b)| b == b'\n').map(|(i, _)| i + 1));
    let mut lines: Vec<Spans> = vec![Vec::new(); starts.len()];
    for (range, token) in spans {
        let mut s = range.start;
        while s < range.end {
            let li = starts.partition_point(|&x| x <= s) - 1;
            let line_end = starts.get(li + 1).map(|e| e - 1).unwrap_or(text.len());
            let e = range.end.min(line_end);
            if e > s {
                lines[li].push((s - starts[li]..e - starts[li], *token));
            }
            s = line_end + 1;
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// Colours (GitHub light / dark palettes)

pub fn style(token: Token, dark: bool) -> HighlightStyle {
    let hex = if dark {
        match token {
            Token::Keyword => 0xff7b72,
            Token::Type => 0xffa657,
            Token::Literal | Token::Number => 0x79c0ff,
            Token::String => 0xa5d6ff,
            Token::Comment => 0x8b949e,
            Token::Function => 0xd2a8ff,
            Token::Meta => 0xffa198,
            Token::Key => 0x7ee787,
            Token::Tag => 0x7ee787,
            Token::Attr => 0x79c0ff,
            Token::Inserted => 0x56d364,
            Token::Deleted => 0xf85149,
        }
    } else {
        match token {
            Token::Keyword => 0xcf222e,
            Token::Type => 0x953800,
            Token::Literal | Token::Number => 0x0550ae,
            Token::String => 0x0a3069,
            Token::Comment => 0x6e7781,
            Token::Function => 0x8250df,
            Token::Meta => 0xbc4c00,
            Token::Key => 0x116329,
            Token::Tag => 0x116329,
            Token::Attr => 0x0550ae,
            Token::Inserted => 0x1a7f37,
            Token::Deleted => 0xcf222e,
        }
    };
    HighlightStyle {
        color: Some(Hsla::from(rgb(hex))),
        font_style: (token == Token::Comment).then_some(FontStyle::Italic),
        ..Default::default()
    }
}

/// `(range, style)` pairs ready for `StyledText::with_highlights`.
pub fn styled(spans: &Spans, dark: bool) -> Vec<(Range<usize>, HighlightStyle)> {
    spans.iter().map(|(r, t)| (r.clone(), style(*t, dark))).collect()
}

// ---------------------------------------------------------------------------
// Pretty printing

/// Re-indent JSON / JSONC (2 spaces), keeping key order, comments and number
/// spellings exactly. Returns `None` if it doesn't look like JSON.
pub fn pretty_json(text: &str) -> Option<String> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if !trimmed.starts_with(['{', '[']) {
        return None;
    }
    let chars: Vec<char> = trimmed.chars().collect();
    let mut out = String::with_capacity(trimmed.len() * 2);
    let mut indent = 0usize;
    let newline = |out: &mut String, indent: usize| {
        out.push('\n');
        for _ in 0..indent {
            out.push_str("  ");
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' | '\'' => {
                let quote = c;
                out.push(c);
                i += 1;
                while i < chars.len() {
                    let d = chars[i];
                    out.push(d);
                    i += 1;
                    if d == '\\' && i < chars.len() {
                        out.push(chars[i]);
                        i += 1;
                    } else if d == quote {
                        break;
                    }
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    out.push(chars[i]);
                    i += 1;
                }
                newline(&mut out, indent);
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let start = i;
                while i < chars.len() {
                    out.push(chars[i]);
                    i += 1;
                    if i >= start + 4 && chars[i - 1] == '/' && chars[i - 2] == '*' {
                        break;
                    }
                }
                continue;
            }
            '{' | '[' => {
                let close = if c == '{' { '}' } else { ']' };
                let next = chars[i + 1..].iter().position(|ch| !ch.is_whitespace()).map(|p| i + 1 + p);
                if next.is_some_and(|n| chars[n] == close) {
                    out.push(c);
                    out.push(close);
                    i = next.unwrap() + 1;
                    continue;
                }
                out.push(c);
                indent += 1;
                newline(&mut out, indent);
            }
            '}' | ']' => {
                indent = indent.saturating_sub(1);
                // Drop the indentation a trailing comment's newline may have left.
                while out.ends_with(' ') {
                    out.pop();
                }
                if !out.ends_with('\n') {
                    newline(&mut out, indent);
                } else {
                    for _ in 0..indent {
                        out.push_str("  ");
                    }
                }
                out.push(c);
            }
            ',' => {
                out.push(',');
                newline(&mut out, indent);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
        i += 1;
    }
    Some(out)
}

/// Re-indent well-formed XML (2 spaces). `None` if it doesn't parse.
pub fn pretty_xml(text: &str) -> Option<String> {
    use quick_xml::{Reader, Writer, events::Event};
    let mut reader = Reader::from_str(text.trim_start_matches('\u{feff}'));
    reader.config_mut().trim_text(true);
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    let mut elements = 0usize;
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(event) => {
                if matches!(event, Event::Start(_) | Event::Empty(_)) {
                    elements += 1;
                }
                writer.write_event(event).ok()?;
            }
            Err(_) => return None,
        }
    }
    if elements == 0 {
        return None;
    }
    String::from_utf8(writer.into_inner()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str, lang: &Lang) -> Vec<(String, Token)> {
        highlight(text, lang).into_iter().map(|(r, t)| (text[r].to_string(), t)).collect()
    }

    #[test]
    fn rust_tokens() {
        let t = tokens("fn main() { let s = \"hi\"; // note\n println!(\"{}\", 42); }", &RUST);
        assert!(t.contains(&("fn".into(), Token::Keyword)));
        assert!(t.contains(&("main".into(), Token::Function)));
        assert!(t.contains(&("\"hi\"".into(), Token::String)));
        assert!(t.contains(&("// note".into(), Token::Comment)));
        assert!(t.contains(&("println".into(), Token::Function)));
        assert!(t.contains(&("42".into(), Token::Number)));
    }

    #[test]
    fn json_keys_and_values() {
        let t = tokens(r#"{"name": "中文", "n": -1.5e3, "ok": true}"#, &JSON);
        assert!(t.contains(&("\"name\"".into(), Token::Key)));
        assert!(t.contains(&("\"中文\"".into(), Token::String)));
        assert!(t.contains(&("1.5e3".into(), Token::Number)));
        assert!(t.contains(&("true".into(), Token::Literal)));
    }

    #[test]
    fn multiline_comment_split_by_line() {
        let text = "a /* one\ntwo */ b";
        let spans = highlight(text, &C);
        let lines = by_line(text, &spans);
        assert_eq!(lines[0], vec![(2..8, Token::Comment)]);
        assert_eq!(lines[1], vec![(0..6, Token::Comment)]);
    }

    #[test]
    fn batch_rem_only_at_line_start() {
        let t = tokens("rem hello\necho remark", &BATCH);
        assert!(t.contains(&("rem hello".into(), Token::Comment)));
        assert!(!t.iter().any(|(s, k)| s.contains("remark") && *k == Token::Comment));
    }

    #[test]
    fn markup_and_yaml_and_ini() {
        let t = tokens(r#"<a href="x">hi &amp; <!-- c --></a>"#, &MARKUP);
        assert!(t.contains(&("<a".into(), Token::Tag)));
        assert!(t.contains(&("href".into(), Token::Attr)));
        assert!(t.contains(&("\"x\"".into(), Token::String)));
        assert!(t.contains(&("&amp;".into(), Token::Literal)));
        assert!(t.contains(&("<!-- c -->".into(), Token::Comment)));

        let t = tokens("name: demo # c\nlist:\n  - port: 8080\n", &YAML);
        assert!(t.contains(&("name".into(), Token::Key)));
        assert!(t.contains(&("# c".into(), Token::Comment)));
        assert!(t.contains(&("port".into(), Token::Key)));
        assert!(t.contains(&("8080".into(), Token::Number)));

        let t = tokens("[server]\nport = 80 ; c\n", &INI);
        assert!(t.contains(&("[server]".into(), Token::Tag)));
        assert!(t.contains(&("port".into(), Token::Key)));
        assert!(t.contains(&("80".into(), Token::Number)));
    }

    #[test]
    fn json_pretty_print() {
        let pretty = pretty_json(r#"{"a":1,"b":[1,2,{}],"c":{"d":"x,y:z"},"e":[]}"#).unwrap();
        assert_eq!(
            pretty,
            "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2,\n    {}\n  ],\n  \"c\": {\n    \"d\": \"x,y:z\"\n  },\n  \"e\": []\n}"
        );
        assert!(pretty_json("not json").is_none());
        // JSONC comments survive.
        let p = pretty_json("{/* c */\"a\":1, // tail\n\"b\":2}").unwrap();
        assert!(p.contains("/* c */") && p.contains("// tail") && p.ends_with("}"));
        // Order and number spelling are preserved.
        let p = pretty_json(r#"{"z":1.50,"a":2}"#).unwrap();
        assert!(p.find("\"z\"").unwrap() < p.find("\"a\"").unwrap());
        assert!(p.contains("1.50"));
    }

    #[test]
    fn xml_pretty_print() {
        let pretty = pretty_xml("<a><b x=\"1\">t</b><c/></a>").unwrap();
        assert_eq!(pretty, "<a>\n  <b x=\"1\">t</b>\n  <c/>\n</a>");
        assert!(pretty_xml("plain text").is_none());
    }
}
