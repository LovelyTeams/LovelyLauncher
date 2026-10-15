//! PDF pages, rendered by Windows itself (`Windows.Data.Pdf`, available since Windows 10).
//!
//! The PDF engine keeps a large thread pool and caches alive for the life of the
//! process, so rendering happens in a helper process (this same exe started with
//! `--pdf-worker <file>`). It exits when the preview moves on, taking all of that
//! memory with it.
//!
//! Protocol over the helper's stdin / stdout:
//! - helper → `OK <pages>\n` then one `<width> <height>\n` per page, or `ERR <message>\n`
//! - app → `<page> <width>\n` for each page wanted
//! - helper → `PAGE <page> <width> <height> <bytes>\n` + BGRA pixels, or `FAIL <page> <message>\n`

use std::{
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::RenderImage;

use super::ffmpeg::CREATE_NO_WINDOW;

/// The widest a page is rendered, in pixels.
pub const MAX_WIDTH: u32 = 2400;
pub const WORKER_FLAG: &str = "--pdf-worker";

pub struct Rendered {
    pub page: usize,
    pub image: Result<Arc<RenderImage>, String>,
}

pub struct PdfDoc {
    /// Page sizes in DIPs (1/96 inch).
    pub pages: Vec<(f32, f32)>,
    input: Mutex<ChildStdin>,
    child: Mutex<Child>,
}

impl PdfDoc {
    /// Ask for `page` at `width` pixels; the result arrives on the channel from [`open`].
    pub fn request(&self, page: usize, width: u32) {
        if let Ok(mut input) = self.input.lock() {
            let _ = writeln!(input, "{page} {}", width.clamp(200, MAX_WIDTH));
            let _ = input.flush();
        }
    }
}

impl Drop for PdfDoc {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Start a helper for `path`. Rendered pages arrive on the returned receiver.
pub fn open(path: &Path) -> Result<(PdfDoc, UnboundedReceiver<Rendered>), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .arg(WORKER_FLAG)
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("无法启动 PDF 渲染：{e}"))?;
    let input = child.stdin.take().ok_or("无法启动 PDF 渲染")?;
    let mut output = BufReader::new(child.stdout.take().ok_or("无法启动 PDF 渲染")?);

    let mut line = String::new();
    output.read_line(&mut line).map_err(|e| e.to_string())?;
    let pages = match line.trim_end().split_once(' ') {
        Some(("OK", count)) => {
            let count: usize = count.parse().map_err(|_| "PDF 渲染进程返回了无法识别的数据")?;
            let mut pages = Vec::with_capacity(count);
            for _ in 0..count {
                line.clear();
                output.read_line(&mut line).map_err(|e| e.to_string())?;
                let mut it = line.split_whitespace().map(|v| v.parse::<f32>().unwrap_or(0.));
                pages.push((it.next().unwrap_or(612.).max(1.), it.next().unwrap_or(792.).max(1.)));
            }
            pages
        }
        Some(("ERR", message)) => {
            let _ = child.kill();
            return Err(message.to_string());
        }
        _ => {
            let _ = child.kill();
            return Err("无法打开 PDF".into());
        }
    };
    if pages.is_empty() {
        let _ = child.kill();
        return Err("这个 PDF 没有页面".into());
    }

    let (tx, rx) = unbounded();
    std::thread::Builder::new()
        .name("pdf-pages".into())
        .spawn(move || read_pages(output, tx))
        .map_err(|e| e.to_string())?;
    Ok((PdfDoc { pages, input: Mutex::new(input), child: Mutex::new(child) }, rx))
}

/// Turn the helper's page messages into images until it exits.
fn read_pages(mut output: BufReader<std::process::ChildStdout>, tx: UnboundedSender<Rendered>) {
    let mut line = String::new();
    loop {
        line.clear();
        if output.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let parts: Vec<&str> = line.trim_end().splitn(5, ' ').collect();
        let rendered = match parts.as_slice() {
            ["PAGE", page, w, h, len] => {
                let (Ok(page), Ok(w), Ok(h), Ok(len)) =
                    (page.parse::<usize>(), w.parse::<u32>(), h.parse::<u32>(), len.parse::<usize>())
                else {
                    return;
                };
                let mut bgra = vec![0u8; len];
                if output.read_exact(&mut bgra).is_err() {
                    return;
                }
                let image = image::RgbaImage::from_raw(w, h, bgra)
                    .map(|img| Arc::new(RenderImage::new(vec![image::Frame::new(img)])))
                    .ok_or_else(|| format!("第 {} 页渲染失败", page + 1));
                Rendered { page, image }
            }
            ["FAIL", page, rest @ ..] => match page.parse() {
                Ok(page) => Rendered { page, image: Err(rest.join(" ")) },
                Err(_) => return,
            },
            _ => return,
        };
        if tx.unbounded_send(rendered).is_err() {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// The helper process

/// Entry point of `LovelyLauncher.exe --pdf-worker <file>`.
pub fn run_worker(path: &Path) {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let doc = match load(path) {
        Ok(doc) => doc,
        Err(err) => {
            let _ = writeln!(out, "ERR {err}");
            let _ = out.flush();
            return;
        }
    };
    let count = doc.PageCount().unwrap_or(0);
    let _ = writeln!(out, "OK {count}");
    for i in 0..count {
        let (w, h) = doc.GetPage(i).and_then(|p| p.Size()).map(|s| (s.Width, s.Height)).unwrap_or((612., 792.));
        let _ = writeln!(out, "{w} {h}");
    }
    if out.flush().is_err() {
        return;
    }

    // Requests are read on the side so that a burst (fast scrolling) can be coalesced.
    let (req_tx, req_rx) = mpsc::channel::<(usize, u32)>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let mut it = line.split_whitespace().map(|v| v.parse::<u32>());
            if let (Some(Ok(page)), Some(Ok(width))) = (it.next(), it.next()) {
                if req_tx.send((page as usize, width)).is_err() {
                    break;
                }
            }
        }
    });
    while let Ok(first) = req_rx.recv() {
        let mut queue = vec![first];
        queue.extend(req_rx.try_iter());
        // Newest first: those are the pages on screen now.
        queue.reverse();
        let mut done = std::collections::HashSet::new();
        for (page, width) in queue {
            if !done.insert(page) {
                continue;
            }
            let ok = match render(&doc, page as u32, width) {
                Ok((w, h, bgra)) => writeln!(out, "PAGE {page} {w} {h} {}", bgra.len()).and_then(|_| out.write_all(&bgra)),
                Err(err) => writeln!(out, "FAIL {page} 第 {} 页渲染失败：{err}", page + 1),
            };
            if ok.and_then(|_| out.flush()).is_err() {
                return;
            }
        }
    }
}

fn load(path: &Path) -> Result<windows::Data::Pdf::PdfDocument, String> {
    use windows::{Data::Pdf::PdfDocument, Storage::StorageFile, core::HSTRING};
    let open = || -> windows::core::Result<PdfDocument> {
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?.get()?;
        PdfDocument::LoadFromFileAsync(&file)?.get()
    };
    open().map_err(|e| {
        // ERROR_WRONG_PASSWORD / E_ACCESSDENIED for encrypted documents.
        if matches!(e.code().0 as u32, 0x8007_052B | 0x8007_0005) {
            "PDF 有密码保护，无法预览".to_string()
        } else {
            format!("无法打开 PDF：{}", e.message()).replace('\n', " ")
        }
    })
}

/// One page as (width, height, BGRA pixels).
fn render(doc: &windows::Data::Pdf::PdfDocument, index: u32, width: u32) -> Result<(u32, u32, Vec<u8>), String> {
    use windows::{
        Data::Pdf::PdfPageRenderOptions,
        Graphics::Imaging::BitmapEncoder,
        Storage::Streams::{DataReader, InMemoryRandomAccessStream},
    };
    let bytes = (|| -> windows::core::Result<Vec<u8>> {
        let page = doc.GetPage(index)?;
        let stream = InMemoryRandomAccessStream::new()?;
        let options = PdfPageRenderOptions::new()?;
        options.SetDestinationWidth(width)?;
        // BMP: no compression on either side.
        options.SetBitmapEncoderId(BitmapEncoder::BmpEncoderId()?)?;
        page.RenderWithOptionsToStreamAsync(&stream, &options)?.get()?;
        let size = stream.Size()? as u32;
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        reader.LoadAsync(size)?.get()?;
        let mut bytes = vec![0u8; size as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(bytes)
    })()
    .map_err(|e| e.message().to_string().replace('\n', " "))?;
    let mut rgba = image::load_from_memory_with_format(&bytes, image::ImageFormat::Bmp)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    // GPUI wants BGRA.
    for px in rgba.pixels_mut() {
        px.0.swap(0, 2);
    }
    let (w, h) = rgba.dimensions();
    Ok((w, h, rgba.into_raw()))
}
