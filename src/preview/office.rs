//! Lightweight readers for .xlsx / .docx (zip + XML), CSV/TSV and text decoding.
//! Only what a quick preview needs: cell values, paragraphs, basic styling, images.

use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read},
    ops::Range,
    path::Path,
};

use quick_xml::{
    Reader, XmlVersion,
    events::{BytesRef, BytesStart, Event},
};

pub const MAX_ROWS: usize = 100_000;
pub const MAX_COLS: usize = 200;

#[derive(Clone, Debug, Default)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Text decoding

/// Decode bytes as UTF-8 / UTF-16 (BOM) or, failing that, the system ANSI code
/// page (GBK on Chinese Windows). Returns the text and a label for the encoding.
pub fn decode_text(bytes: &[u8]) -> (String, &'static str) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(rest).into_owned(), "UTF-8 BOM");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return (String::from_utf16_lossy(&units), "UTF-16 LE");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return (String::from_utf16_lossy(&units), "UTF-16 BE");
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), "UTF-8"),
        // A multi-byte character cut off by a read limit is still UTF-8.
        Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 > bytes.len() => {
            (String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(), "UTF-8")
        }
        Err(_) => (ansi_to_string(bytes), "ANSI / GBK"),
    }
}

pub(crate) fn ansi_to_string(bytes: &[u8]) -> String {
    use windows::Win32::Globalization::{MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};
    unsafe {
        let len = MultiByteToWideChar(0, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None);
        if len <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut buf = vec![0u16; len as usize];
        let written = MultiByteToWideChar(0, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut buf));
        String::from_utf16_lossy(&buf[..written.max(0) as usize])
    }
}

/// Heuristic: does this look like text rather than binary?
pub fn looks_like_text(sample: &[u8]) -> bool {
    if sample.starts_with(&[0xFF, 0xFE]) || sample.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    if sample.contains(&0) {
        return false;
    }
    let control = sample
        .iter()
        .filter(|&&b| b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t' | 0x0C | 0x1B))
        .count();
    control * 100 <= sample.len().max(1)
}

/// Read up to `limit` bytes of a file.
pub fn read_prefix(path: &Path, limit: usize) -> Result<(Vec<u8>, bool), String> {
    let mut file = File::open(path).map_err(|e| format!("无法打开文件：{e}"))?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut buf = Vec::with_capacity(limit.min(total as usize));
    file.by_ref()
        .take(limit as u64)
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取文件失败：{e}"))?;
    Ok((buf, total as usize > limit))
}

// ---------------------------------------------------------------------------
// CSV / TSV

pub fn parse_delimited(text: &str, delimiter: char) -> Sheet {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    let mut truncated = false;
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => in_quotes = true,
            c if c == delimiter => {
                if row.len() < MAX_COLS {
                    row.push(std::mem::take(&mut field));
                } else {
                    field.clear();
                }
            }
            '\r' => {}
            '\n' => {
                if row.len() < MAX_COLS {
                    row.push(std::mem::take(&mut field));
                }
                field.clear();
                rows.push(std::mem::take(&mut row));
                if rows.len() >= MAX_ROWS {
                    truncated = true;
                    break;
                }
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Sheet {
        name: String::new(),
        rows,
        truncated,
    }
}

// ---------------------------------------------------------------------------
// XML helpers

fn local(name: &str) -> &str {
    match name.rfind(':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

fn attr(e: &BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.as_ref() == key || local(a.key.as_ref()) == key).then(|| {
            a.normalized_value(XmlVersion::Implicit1_0)
                .map(|v| v.into_owned())
                .unwrap_or_default()
        })
    })
}

fn entity(r: &BytesRef) -> String {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return c.to_string();
    }
    match &**r {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => "",
    }
    .to_string()
}

fn zip_text(archive: &mut zip::ZipArchive<BufReader<File>>, name: &str) -> Option<String> {
    let mut entry = archive.by_name(name).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    Some(text)
}

fn zip_bytes(archive: &mut zip::ZipArchive<BufReader<File>>, name: &str) -> Option<Vec<u8>> {
    let mut entry = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<BufReader<File>>, String> {
    let file = File::open(path).map_err(|e| format!("无法打开文件：{e}"))?;
    zip::ZipArchive::new(BufReader::new(file)).map_err(|_| "文件已损坏，或不是新版 Office 格式".to_string())
}

/// Relationship id → target path, resolved against `base_dir` (e.g. "xl/").
fn relationships(xml: &str, base_dir: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut reader = Reader::from_str(xml);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) if local(e.name().as_ref()) == "Relationship" => {
                if let (Some(id), Some(target)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    let path = match target.strip_prefix('/') {
                        Some(abs) => abs.to_string(),
                        None => format!("{base_dir}{target}"),
                    };
                    map.insert(id, normalize_zip_path(&path));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    map
}

fn normalize_zip_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

// ---------------------------------------------------------------------------
// XLSX

pub fn read_xlsx(path: &Path, max_rows: usize) -> Result<Vec<Sheet>, String> {
    let mut archive = open_zip(path)?;
    let shared = zip_text(&mut archive, "xl/sharedStrings.xml")
        .map(|xml| shared_strings(&xml))
        .unwrap_or_default();
    let workbook = zip_text(&mut archive, "xl/workbook.xml").ok_or("缺少 workbook.xml，可能不是 Excel 文件")?;
    let rels = zip_text(&mut archive, "xl/_rels/workbook.xml.rels")
        .map(|xml| relationships(&xml, "xl/"))
        .unwrap_or_default();

    let mut sheets = Vec::new();
    let mut reader = Reader::from_str(&workbook);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) if local(e.name().as_ref()) == "sheet" => {
                let name = attr(&e, "name").unwrap_or_else(|| format!("Sheet{}", sheets.len() + 1));
                let target = attr(&e, "r:id").and_then(|id| rels.get(&id).cloned());
                sheets.push((name, target));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if sheets.is_empty() {
        return Err("工作簿里没有工作表".into());
    }

    let mut out = Vec::new();
    for (i, (name, target)) in sheets.into_iter().enumerate() {
        let target = target.unwrap_or_else(|| format!("xl/worksheets/sheet{}.xml", i + 1));
        let sheet = match zip_text(&mut archive, &target) {
            Some(xml) => parse_sheet(&xml, &shared, name, max_rows),
            None => Sheet { name, ..Default::default() },
        };
        out.push(sheet);
    }
    Ok(out)
}

fn shared_strings(xml: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_t = false;
    let mut reader = Reader::from_str(xml);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => match local(e.name().as_ref()) {
                "si" => current.clear(),
                "t" => in_t = true,
                _ => {}
            },
            Event::End(e) => match local(e.name().as_ref()) {
                "si" => strings.push(std::mem::take(&mut current)),
                "t" => in_t = false,
                _ => {}
            },
            Event::Text(t) if in_t => current.push_str(&t),
            Event::GeneralRef(r) if in_t => current.push_str(&entity(&r)),
            Event::Eof => break,
            _ => {}
        }
    }
    strings
}

/// "AB12" → (col 27, row 11), zero-based.
fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let letters: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits = &r[letters.len()..];
    let col = letters
        .chars()
        .fold(0usize, |acc, c| acc * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1));
    let row: usize = digits.parse().ok()?;
    (col > 0 && row > 0).then(|| (col - 1, row - 1))
}

fn format_number(raw: &str) -> String {
    match raw.parse::<f64>() {
        Ok(n) if n.is_finite() => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", n as i64)
            } else {
                let s = format!("{n:.10}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
        _ => raw.to_string(),
    }
}

fn parse_sheet(xml: &str, shared: &[String], name: String, max_rows: usize) -> Sheet {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    let (mut cell_type, mut pos) = (String::new(), None::<(usize, usize)>);
    let mut value = String::new();
    let (mut in_value, mut row_index) = (false, 0usize);
    // Formula text, shown when the file has no cached result for it.
    let (mut formula, mut in_formula) = (String::new(), false);
    let mut next_col = 0usize;
    let mut reader = Reader::from_str(xml);
    loop {
        let Ok(event) = reader.read_event() else { break };
        match event {
            Event::Start(e) | Event::Empty(e) => match local(e.name().as_ref()) {
                "row" => {
                    row_index = attr(&e, "r")
                        .and_then(|r| r.parse::<usize>().ok())
                        .map(|r| r.saturating_sub(1))
                        .unwrap_or(rows.len());
                    next_col = 0;
                    if row_index >= max_rows {
                        truncated = true;
                        break;
                    }
                }
                "c" => {
                    cell_type = attr(&e, "t").unwrap_or_default();
                    pos = attr(&e, "r").and_then(|r| cell_ref(&r)).or(Some((next_col, row_index)));
                    value.clear();
                    formula.clear();
                }
                "v" | "t" => in_value = true,
                "f" => in_formula = true,
                _ => {}
            },
            Event::End(e) => match local(e.name().as_ref()) {
                "v" | "t" => in_value = false,
                "f" => in_formula = false,
                "c" => {
                    if let Some((col, row)) = pos.take() {
                        next_col = col + 1;
                        if value.is_empty() && !formula.is_empty() {
                            value = format!("={formula}");
                            cell_type = "str".into();
                        }
                        if col < MAX_COLS && !value.is_empty() {
                            let text = match cell_type.as_str() {
                                "s" => value.trim().parse::<usize>().ok().and_then(|i| shared.get(i).cloned()).unwrap_or_default(),
                                "b" => if value.trim() == "1" { "TRUE".into() } else { "FALSE".into() },
                                "str" | "inlineStr" | "e" => value.clone(),
                                _ => format_number(value.trim()),
                            };
                            if rows.len() <= row {
                                rows.resize_with(row + 1, Vec::new);
                            }
                            let cells = &mut rows[row];
                            if cells.len() <= col {
                                cells.resize(col + 1, String::new());
                            }
                            cells[col] = text;
                        }
                    }
                }
                _ => {}
            },
            Event::Text(t) if in_value => value.push_str(&t),
            Event::Text(t) if in_formula => formula.push_str(&t),
            Event::GeneralRef(r) if in_value => value.push_str(&entity(&r)),
            Event::Eof => break,
            _ => {}
        }
    }
    Sheet { name, rows, truncated }
}

// ---------------------------------------------------------------------------
// DOCX

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub link: bool,
}

#[derive(Clone, Debug)]
pub enum DocBlock {
    Paragraph {
        text: String,
        /// Styled ranges (byte offsets into `text`).
        runs: Vec<(Range<usize>, RunStyle)>,
        /// 1..=6 for headings, 0 for body text.
        heading: u8,
        /// Some(level) for list items.
        list_level: Option<u8>,
    },
    Table(Vec<Vec<String>>),
    Image(Vec<u8>),
}

pub fn read_docx(path: &Path) -> Result<Vec<DocBlock>, String> {
    let mut archive = open_zip(path)?;
    let document = zip_text(&mut archive, "word/document.xml").ok_or("缺少 document.xml，可能不是 Word 文档")?;
    let headings = zip_text(&mut archive, "word/styles.xml")
        .map(|xml| heading_styles(&xml))
        .unwrap_or_default();
    let rels = zip_text(&mut archive, "word/_rels/document.xml.rels")
        .map(|xml| relationships(&xml, "word/"))
        .unwrap_or_default();

    let mut blocks = Vec::new();
    let mut reader = Reader::from_str(&document);

    // Paragraph state
    let (mut text, mut runs) = (String::new(), Vec::<(Range<usize>, RunStyle)>::new());
    let (mut heading, mut list_level) = (0u8, None::<u8>);
    let mut style = RunStyle::default();
    let (mut in_rpr, mut in_ppr, mut in_text, mut in_link) = (false, false, false, false);
    let mut images: Vec<String> = Vec::new();
    // Table state (nested tables are flattened into their outer cell)
    let mut table_depth = 0usize;
    let mut table: Vec<Vec<String>> = Vec::new();
    let mut cell = String::new();

    loop {
        let Ok(event) = reader.read_event() else { break };
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name();
                match local(name.as_ref()) {
                    "tbl" => {
                        table_depth += 1;
                        if table_depth == 1 {
                            table.clear();
                        }
                    }
                    "tr" if table_depth == 1 => table.push(Vec::new()),
                    "tc" if table_depth == 1 => cell.clear(),
                    "p" => {
                        text.clear();
                        runs.clear();
                        heading = 0;
                        list_level = None;
                        images.clear();
                    }
                    "pStyle" => {
                        if let Some(id) = attr(&e, "w:val") {
                            heading = headings.get(&id).copied().unwrap_or(0);
                        }
                    }
                    "ilvl" => list_level = Some(attr(&e, "w:val").and_then(|v| v.parse().ok()).unwrap_or(0)),
                    "numPr" => list_level = list_level.or(Some(0)),
                    "hyperlink" => in_link = true,
                    "r" => style = RunStyle { link: in_link, ..Default::default() },
                    "rPr" => in_rpr = true,
                    "pPr" => in_ppr = true,
                    "b" | "bCs" if in_rpr => style.bold = attr(&e, "w:val").is_none_or(|v| v != "0" && v != "false"),
                    "i" | "iCs" if in_rpr => style.italic = attr(&e, "w:val").is_none_or(|v| v != "0" && v != "false"),
                    "u" if in_rpr => style.underline = attr(&e, "w:val").is_none_or(|v| v != "none"),
                    "strike" if in_rpr => style.strike = attr(&e, "w:val").is_none_or(|v| v != "0" && v != "false"),
                    "t" => in_text = true,
                    "tab" if !in_ppr => text.push('\t'),
                    "br" | "cr" => text.push('\n'),
                    "blip" => {
                        if let Some(target) = attr(&e, "r:embed").and_then(|id| rels.get(&id).cloned()) {
                            images.push(target);
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = e.name();
                match local(name.as_ref()) {
                    "t" => in_text = false,
                    "rPr" => in_rpr = false,
                    "pPr" => in_ppr = false,
                    "hyperlink" => in_link = false,
                    "p" => {
                        if table_depth > 0 {
                            if !cell.is_empty() {
                                cell.push('\n');
                            }
                            cell.push_str(&text);
                        } else {
                            if !text.trim().is_empty() || images.is_empty() {
                                blocks.push(DocBlock::Paragraph {
                                    text: std::mem::take(&mut text),
                                    runs: std::mem::take(&mut runs),
                                    heading,
                                    list_level,
                                });
                            }
                            for target in images.drain(..) {
                                if let Some(bytes) = zip_bytes(&mut archive, &target) {
                                    blocks.push(DocBlock::Image(bytes));
                                }
                            }
                        }
                    }
                    "tc" if table_depth == 1 => {
                        if let Some(row) = table.last_mut() {
                            row.push(std::mem::take(&mut cell));
                        }
                    }
                    "tbl" => {
                        table_depth = table_depth.saturating_sub(1);
                        if table_depth == 0 && !table.is_empty() {
                            blocks.push(DocBlock::Table(std::mem::take(&mut table)));
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(t) if in_text => push_run(&mut text, &mut runs, &t, style),
            Event::GeneralRef(r) if in_text => push_run(&mut text, &mut runs, &entity(&r), style),
            Event::Eof => break,
            _ => {}
        }
    }
    // Collapse runs of empty paragraphs into one spacer.
    blocks.dedup_by(|a, b| {
        matches!((a, b), (DocBlock::Paragraph { text: x, .. }, DocBlock::Paragraph { text: y, .. }) if x.trim().is_empty() && y.trim().is_empty())
    });
    Ok(blocks)
}

fn push_run(text: &mut String, runs: &mut Vec<(Range<usize>, RunStyle)>, piece: &str, style: RunStyle) {
    let start = text.len();
    text.push_str(piece);
    if style != RunStyle::default() {
        match runs.last_mut() {
            Some((range, s)) if *s == style && range.end == start => range.end = text.len(),
            _ => runs.push((start..text.len(), style)),
        }
    }
}

/// Style id → heading level, from styles.xml ("heading 1", "Title", …).
fn heading_styles(xml: &str) -> HashMap<String, u8> {
    let mut map = HashMap::new();
    let mut current: Option<String> = None;
    let mut reader = Reader::from_str(xml);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local(e.name().as_ref()) {
                "style" => current = attr(&e, "w:styleId"),
                "name" => {
                    if let (Some(id), Some(name)) = (current.as_ref(), attr(&e, "w:val")) {
                        let lower = name.to_lowercase();
                        let level = if lower == "title" {
                            1
                        } else if let Some(n) = lower.strip_prefix("heading ") {
                            n.trim().parse::<u8>().unwrap_or(0).min(6)
                        } else {
                            0
                        };
                        if level > 0 {
                            map.insert(id.clone(), level);
                        }
                    }
                }
                _ => {}
            },
            Event::End(e) if local(e.name().as_ref()) == "style" => current = None,
            Event::Eof => break,
            _ => {}
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_and_newlines() {
        let s = parse_delimited("a,\"b,1\",c\n\"x\"\"y\",\"multi\nline\",z\n", ',');
        assert_eq!(s.rows, vec![vec!["a", "b,1", "c"], vec!["x\"y", "multi\nline", "z"]]);
        let t = parse_delimited("a\tb\r\n1\t2", '\t');
        assert_eq!(t.rows, vec![vec!["a", "b"], vec!["1", "2"]]);
    }

    #[test]
    fn cell_refs_and_numbers() {
        assert_eq!(cell_ref("A1"), Some((0, 0)));
        assert_eq!(cell_ref("AB12"), Some((27, 11)));
        assert_eq!(format_number("3"), "3");
        assert_eq!(format_number("0.30000000000000004"), "0.3");
        assert_eq!(format_number("1.5"), "1.5");
    }

    #[test]
    fn sheet_xml() {
        let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1"><v>42</v></c></row><row r="2"><c r="B2" t="inlineStr"><is><t>a &amp; b</t></is></c></row></sheetData></worksheet>"#;
        let s = parse_sheet(xml, &["hello".to_string()], "S".into(), 100);
        assert_eq!(s.rows[0], vec!["hello", "", "42"]);
        assert_eq!(s.rows[1], vec!["", "a & b"]);

        // A formula without a cached value shows the formula itself.
        let xml = r#"<worksheet><sheetData><row r="1"><c r="A1"><f>B1*2</f></c><c r="B1"><f>1+1</f><v>2</v></c></row></sheetData></worksheet>"#;
        let s = parse_sheet(xml, &[], "S".into(), 100);
        assert_eq!(s.rows[0], vec!["=B1*2", "2"]);
    }

    #[test]
    fn decoding() {
        assert_eq!(decode_text("中文".as_bytes()).1, "UTF-8");
        assert_eq!(decode_text(&[0xEF, 0xBB, 0xBF, b'a']).0, "a");
        assert_eq!(decode_text(&[0xFF, 0xFE, b'h', 0]).0, "h");
        assert!(looks_like_text(b"hello\nworld\t1"));
        assert!(!looks_like_text(b"\x00\x01\x02binary"));
    }
}
