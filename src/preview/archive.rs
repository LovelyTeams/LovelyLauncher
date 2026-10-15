//! Archive listings for the preview: zip, 7z, rar, tar(.gz/.bz2/.xz/.zst), cab, iso…
//!
//! Windows ships bsdtar (`tar.exe`, libarchive) which reads all of these, so it does
//! the listing; zip files fall back to the `zip` crate if that fails.

use std::{
    io::{BufRead, BufReader, Read},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use super::ffmpeg::CREATE_NO_WINDOW;

/// Listing stops here; huge archives show the first part.
pub const MAX_ENTRIES: usize = 20_000;
/// A compressed tarball has to be decompressed to be listed; don't spin forever.
const TIME_LIMIT: Duration = Duration::from_secs(20);

pub const EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "tbz", "tbz2", "xz", "txz", "zst", "tzst", "lz", "lzma", "cab",
    "iso", "jar", "war", "apk", "xpi", "nupkg", "whl", "cpio", "lzh", "lha", "xar",
];
const ZIP_LIKE: &[&str] = &["zip", "jar", "war", "apk", "xpi", "nupkg", "whl"];

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Path inside the archive, `/`-separated, without a trailing slash.
    pub path: String,
    pub dir: bool,
    pub size: u64,
    /// As the archive tool prints it (already in the user's locale).
    pub modified: String,
}

pub struct Listing {
    pub entries: Vec<Entry>,
    /// Stopped early (too many entries or too slow).
    pub truncated: bool,
}

pub fn list(path: &Path) -> Result<Listing, String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match list_with_tar(path) {
        Ok(listing) => Ok(listing),
        Err(err) if ZIP_LIKE.contains(&ext.as_str()) => list_zip(path).map_err(|_| err),
        Err(err) => Err(err),
    }
}

fn tar_exe() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join("tar.exe")
}

fn list_with_tar(path: &Path) -> Result<Listing, String> {
    let exe = tar_exe();
    if !exe.is_file() {
        return Err("系统里没有 tar.exe（需要 Windows 10 1803 或更新版本）".into());
    }
    let mut child = Command::new(exe)
        .arg("-tvf")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("无法运行 tar.exe：{e}"))?;
    let stdout = child.stdout.take().ok_or("无法读取 tar.exe 输出")?;
    let mut stderr = child.stderr.take();
    // Read stderr on the side so a chatty failure can't block the pipe.
    let errors = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_end(&mut buf);
        }
        buf
    });

    let started = Instant::now();
    let mut reader = BufReader::new(stdout);
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if let Some(entry) = parse_line(&super::office::ansi_to_string(&line)) {
            entries.push(entry);
        }
        if entries.len() >= MAX_ENTRIES || started.elapsed() > TIME_LIMIT {
            truncated = true;
            let _ = child.kill();
            break;
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let stderr = super::office::ansi_to_string(&errors.join().unwrap_or_default());
    if !truncated && !status.success() && entries.is_empty() {
        let detail = stderr.lines().map(|l| l.trim_start_matches("tar.exe: ").trim()).find(|l| !l.is_empty());
        return Err(match detail {
            Some(d) if d.contains("Unrecognized archive format") => "无法识别的压缩包格式".to_string(),
            Some(d) if d.contains("encrypted") || d.contains("passphrase") => "压缩包已加密，无法查看内容".to_string(),
            Some(d) => format!("无法读取压缩包：{d}"),
            None => "无法读取压缩包".to_string(),
        });
    }
    Ok(Listing { entries, truncated })
}

/// One line of `tar -tv`: mode, links, owner, group, size, three date fields, then the name.
/// `drwxr-xr-x  0 0      0           0 10月 15 12:49 dir/`
fn parse_line(line: &str) -> Option<Entry> {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut rest = line;
    let mut fields = Vec::with_capacity(8);
    for _ in 0..8 {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace)?;
        fields.push(&rest[..end]);
        rest = &rest[end..];
    }
    // Exactly one separator space before the name; names may start with spaces themselves.
    let name = rest.strip_prefix(' ').unwrap_or(rest);
    let mode = fields[0];
    let size = fields[4].parse().ok()?;
    let name = match mode.as_bytes().first() {
        Some(b'l') => name.split(" -> ").next().unwrap_or(name),
        Some(b'h') => name.split(" link to ").next().unwrap_or(name),
        _ => name,
    };
    let dir = mode.starts_with('d') || name.ends_with('/');
    let path = normalize(name);
    (!path.is_empty()).then(|| Entry {
        path,
        dir,
        size: if dir { 0 } else { size },
        modified: fields[5..8].join(" "),
    })
}

fn normalize(name: &str) -> String {
    let name = name.replace('\\', "/");
    let mut s = name.as_str();
    while let Some(rest) = s.strip_prefix("./") {
        s = rest;
    }
    s.trim_matches('/').to_string()
}

fn list_zip(path: &Path) -> Result<Listing, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("无法打开文件：{e}"))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(|e| format!("无法读取压缩包：{e}"))?;
    let count = zip.len();
    let mut entries = Vec::new();
    for i in 0..count.min(MAX_ENTRIES) {
        let Ok(f) = zip.by_index_raw(i) else { continue };
        let raw = f.name_raw();
        // Older Chinese tools store GBK names without the UTF-8 flag.
        let name = match std::str::from_utf8(raw) {
            Ok(s) => s.to_string(),
            Err(_) => super::office::ansi_to_string(raw),
        };
        let modified = f
            .last_modified()
            .map(|t| format!("{}-{:02}-{:02} {:02}:{:02}", t.year(), t.month(), t.day(), t.hour(), t.minute()))
            .unwrap_or_default();
        let path = normalize(&name);
        if !path.is_empty() {
            entries.push(Entry { path, dir: f.is_dir(), size: if f.is_dir() { 0 } else { f.size() }, modified });
        }
    }
    Ok(Listing { entries, truncated: count > MAX_ENTRIES })
}

/// A row of the folder tree shown in the preview.
#[derive(Clone, Debug)]
pub struct Row {
    pub name: String,
    pub depth: usize,
    pub dir: bool,
    /// Files: own size. Folders: total size of everything inside.
    pub size: u64,
    pub modified: String,
    /// Number of rows below this one that belong to it (folders only).
    pub descendants: usize,
}

#[derive(Default)]
struct Node {
    name: String,
    dir: bool,
    size: u64,
    modified: String,
    children: Vec<usize>,
}

/// Arrange entries as a tree (folders first, then names), including folders that
/// only appear implicitly in file paths.
pub fn tree(entries: &[Entry]) -> Vec<Row> {
    let mut nodes = vec![Node { dir: true, ..Default::default() }];
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in entries {
        let mut parent = 0;
        let mut prefix = String::new();
        let parts: Vec<&str> = e.path.split('/').filter(|p| !p.is_empty()).collect();
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            let id = match index.get(&prefix) {
                Some(&id) => id,
                None => {
                    nodes.push(Node { name: part.to_string(), dir: !last || e.dir, ..Default::default() });
                    let id = nodes.len() - 1;
                    nodes[parent].children.push(id);
                    index.insert(prefix.clone(), id);
                    id
                }
            };
            if last {
                let node = &mut nodes[id];
                node.dir |= e.dir;
                if !e.dir {
                    node.size = e.size;
                }
                node.modified = e.modified.clone();
            }
            parent = id;
        }
    }

    fn total(nodes: &mut [Node], id: usize) -> u64 {
        if !nodes[id].dir {
            return nodes[id].size;
        }
        let children = nodes[id].children.clone();
        let sum = children.iter().map(|&c| total(nodes, c)).sum();
        nodes[id].size = sum;
        sum
    }
    total(&mut nodes, 0);

    fn walk(nodes: &[Node], id: usize, depth: usize, out: &mut Vec<Row>) {
        let mut children = nodes[id].children.clone();
        children.sort_by(|&a, &b| {
            let (a, b) = (&nodes[a], &nodes[b]);
            b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        for c in children {
            let at = out.len();
            let n = &nodes[c];
            out.push(Row {
                name: n.name.clone(),
                depth,
                dir: n.dir,
                size: n.size,
                modified: n.modified.clone(),
                descendants: 0,
            });
            walk(nodes, c, depth + 1, out);
            out[at].descendants = out.len() - at - 1;
        }
    }
    let mut rows = Vec::new();
    walk(&nodes, 0, 0, &mut rows);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tar_lines() {
        let e = parse_line("drwxrwxr-x  0 0      0           0 10月 15 12:49 LovelyLauncher 项目/\r\n").unwrap();
        assert_eq!((e.path.as_str(), e.dir, e.modified.as_str()), ("LovelyLauncher 项目", true, "10月 15 12:49"));
        let e = parse_line("-rw-r--r--  0 0      0        4500 Oct 15  2025 ./a b/c.txt").unwrap();
        assert_eq!((e.path.as_str(), e.dir, e.size), ("a b/c.txt", false, 4500));
        let e = parse_line("lrwxrwxrwx  0 1000   1000        0 Jan  1 00:00 link -> target").unwrap();
        assert_eq!(e.path, "link");
        assert!(parse_line("garbage").is_none());
    }

    #[test]
    fn builds_tree_with_implicit_folders() {
        let f = |p: &str, size| Entry { path: p.into(), dir: false, size, modified: String::new() };
        let rows = tree(&[f("b.txt", 1), f("src/z.rs", 10), f("src/a/x.rs", 5), f("A.md", 2)]);
        let names: Vec<_> = rows.iter().map(|r| (r.name.as_str(), r.depth)).collect();
        assert_eq!(names, [("src", 0), ("a", 1), ("x.rs", 2), ("z.rs", 1), ("A.md", 0), ("b.txt", 0)]);
        assert_eq!((rows[0].size, rows[0].descendants), (15, 3));
        assert_eq!(rows[1].descendants, 1);
    }
}
