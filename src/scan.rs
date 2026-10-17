//! 快速扫码: find QR codes and barcodes on every monitor and in an image on the
//! clipboard, decoded with rxing (a port of ZXing).

use std::ffi::c_void;

use rxing::{BarcodeFormat, Point, helpers::detect_multiple_in_luma};
use windows::Win32::{
    Foundation::{HGLOBAL, LPARAM, RECT, TRUE},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, HDC, HMONITOR, ReleaseDC, SRCCOPY, SelectObject,
    },
    System::{
        DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW},
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        Ole::{CF_DIB, CF_DIBV5},
    },
};
use windows::core::{BOOL, w};

/// One decoded code.
#[derive(Clone, Debug)]
pub struct Code {
    /// "二维码", "条形码（EAN-13）", …
    pub kind: String,
    pub text: String,
    /// Index into [`ScanOutput::sources`].
    pub source: usize,
    /// Where the code is in its source, as fractions of its size: (x, y, width, height).
    pub rect: (f32, f32, f32, f32),
    /// The code with what's around it (labels, captions), outlined in its colour,
    /// as (width, height, BGRA).
    pub thumb: Option<(u32, u32, Vec<u8>)>,
}

impl Code {
    pub fn is_link(&self) -> bool {
        let t = self.text.trim().to_ascii_lowercase();
        t.starts_with("http://") || t.starts_with("https://")
    }
}

/// A searched picture (a monitor, or the clipboard image), shrunk for an overview
/// on which each code is marked with its number.
pub struct Source {
    pub label: String,
    pub preview: (u32, u32, Vec<u8>),
}

pub struct ScanOutput {
    /// Only the pictures in which something was found.
    pub sources: Vec<Source>,
    pub codes: Vec<Code>,
}

/// Colours that tell results apart (result `n` uses `COLORS[n % len]`), as 0xRRGGBB.
pub const COLORS: [u32; 8] = [0xE5484D, 0x3E63DD, 0x30A46C, 0xF76B15, 0x8E4EC6, 0x12A594, 0xD6409F, 0x978365];
/// Width of the overview pictures.
pub const PREVIEW_WIDTH: u32 = 516;

/// A picture to search, as BGRA.
struct Capture {
    label: String,
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

/// Scan the clipboard image (if any) and every monitor; duplicates are dropped.
pub fn scan_all() -> ScanOutput {
    let mut captures = Vec::new();
    if let Some(c) = clipboard_image() {
        captures.push(c);
    }
    let screens = capture_monitors();
    let several = screens.len() > 1;
    for (i, mut c) in screens.into_iter().enumerate() {
        c.label = if several { format!("屏幕 {}", i + 1) } else { "屏幕".to_string() };
        captures.push(c);
    }
    find(&captures)
}

fn find(captures: &[Capture]) -> ScanOutput {
    // (capture, kind, text, box in pixels)
    let mut found: Vec<(usize, String, String, (f32, f32, f32, f32))> = Vec::new();
    for (ci, cap) in captures.iter().enumerate() {
        let luma = bgra_to_luma(&cap.bgra);
        let results = detect_multiple_in_luma(luma.clone(), cap.width, cap.height).unwrap_or_default();
        for r in results {
            let text = r.getText().to_string();
            let kind = kind_name(r.getBarcodeFormat());
            // The same code twice (say on screen and copied too) is listed once.
            if text.trim().is_empty() || found.iter().any(|f| f.1 == kind && f.2 == text) {
                continue;
            }
            if let Some(b) = code_box(&luma, cap.width, cap.height, r.getPoints()) {
                found.push((ci, kind, text, b));
            }
        }
    }
    let mut used: Vec<usize> = found.iter().map(|f| f.0).collect();
    used.sort();
    used.dedup();
    let sources = used
        .iter()
        .map(|&ci| Source { label: captures[ci].label.clone(), preview: preview(&captures[ci]) })
        .collect();
    let codes = found
        .into_iter()
        .enumerate()
        .map(|(n, (ci, kind, text, (x0, y0, x1, y1)))| {
            let cap = &captures[ci];
            let (w, h) = (cap.width as f32, cap.height as f32);
            Code {
                kind,
                text,
                source: used.iter().position(|&u| u == ci).unwrap_or(0),
                rect: (x0 / w, y0 / h, (x1 - x0) / w, (y1 - y0) / h),
                thumb: context(cap, (x0, y0, x1, y1), COLORS[n % COLORS.len()]),
            }
        })
        .collect();
    ScanOutput { sources, codes }
}

/// The code's box in pixels, from the points the reader reported: finder pattern
/// centres for 2D codes (widened to the corners), a scan line for barcodes (whose
/// bars are followed up and down).
fn code_box(luma: &[u8], width: u32, height: u32, points: &[Point]) -> Option<(f32, f32, f32, f32)> {
    if points.is_empty() {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let (x0, y0, x1, y1) = if h < w * 0.2 {
        let (top, bottom) = bar_extent(luma, width, height, x0, x1, (y0 + y1) / 2.);
        let pad = 4. + w * 0.03;
        (x0 - pad, top - 4., x1 + pad, bottom + 4.)
    } else {
        let pad = w.max(h) * 0.22 + 4.;
        (x0 - pad, y0 - pad, x1 + pad, y1 + pad)
    };
    Some((x0.max(0.), y0.max(0.), x1.min(width as f32), y1.min(height as f32)))
}

/// The code and what's around it, with the code outlined in `color`, fit within 160 x 110.
fn context(cap: &Capture, (x0, y0, x1, y1): (f32, f32, f32, f32), color: u32) -> Option<(u32, u32, Vec<u8>)> {
    let margin = (x1 - x0).max(y1 - y0) * 0.9 + 16.;
    let cx0 = (x0 - margin).max(0.) as u32;
    let cy0 = (y0 - margin).max(0.) as u32;
    let cx1 = ((x1 + margin).min(cap.width as f32) as u32).max(cx0 + 1);
    let cy1 = ((y1 + margin).min(cap.height as f32) as u32).max(cy0 + 1);
    let (cw, ch) = (cx1 - cx0, cy1 - cy0);
    let mut crop = image::RgbaImage::new(cw, ch);
    for y in 0..ch {
        let start = (((cy0 + y) * cap.width + cx0) * 4) as usize;
        let row = cap.bgra.get(start..start + cw as usize * 4)?;
        for (x, px) in row.chunks_exact(4).enumerate() {
            crop.put_pixel(x as u32, y, image::Rgba([px[0], px[1], px[2], 255]));
        }
    }
    // Outline, thick enough to stay visible once shrunk.
    let scale = (160. / cw as f32).min(110. / ch as f32).min(1.);
    let t = ((2.5 / scale).ceil() as u32).max(2);
    let bgra = [(color & 0xFF) as u8, ((color >> 8) & 0xFF) as u8, ((color >> 16) & 0xFF) as u8, 255];
    let (bx0, by0) = ((x0 as u32).saturating_sub(cx0), (y0 as u32).saturating_sub(cy0));
    let (bx1, by1) = (((x1 as u32).saturating_sub(cx0)).min(cw - 1), ((y1 as u32).saturating_sub(cy0)).min(ch - 1));
    for y in by0..=by1 {
        for x in bx0..=bx1 {
            let edge = x < bx0 + t || x + t > bx1 || y < by0 + t || y + t > by1;
            if edge {
                crop.put_pixel(x, y, image::Rgba(bgra));
            }
        }
    }
    let (tw, th) = (((cw as f32 * scale) as u32).max(1), ((ch as f32 * scale) as u32).max(1));
    let small = image::imageops::resize(&crop, tw, th, image::imageops::FilterType::Triangle);
    Some((tw, th, small.into_raw()))
}

/// The whole picture, [`PREVIEW_WIDTH`] wide.
fn preview(cap: &Capture) -> (u32, u32, Vec<u8>) {
    let full = image::RgbaImage::from_raw(cap.width, cap.height, cap.bgra.clone()).unwrap_or_default();
    let w = PREVIEW_WIDTH.min(cap.width.max(1));
    let h = ((cap.height as f32 * w as f32 / cap.width.max(1) as f32) as u32).max(1);
    let small = image::imageops::resize(&full, w, h, image::imageops::FilterType::Triangle);
    (w, h, small.into_raw())
}

fn kind_name(format: &BarcodeFormat) -> String {
    match format {
        BarcodeFormat::QR_CODE | BarcodeFormat::MICRO_QR_CODE | BarcodeFormat::RECTANGULAR_MICRO_QR_CODE => "二维码".into(),
        BarcodeFormat::DATA_MATRIX => "Data Matrix 码".into(),
        BarcodeFormat::PDF_417 => "PDF417 码".into(),
        BarcodeFormat::AZTEC => "Aztec 码".into(),
        BarcodeFormat::MAXICODE => "MaxiCode".into(),
        other => format!("条形码（{}）", barcode_name(other)),
    }
}

fn barcode_name(format: &BarcodeFormat) -> String {
    match format {
        BarcodeFormat::EAN_13 => "EAN-13".into(),
        BarcodeFormat::EAN_8 => "EAN-8".into(),
        BarcodeFormat::UPC_A => "UPC-A".into(),
        BarcodeFormat::UPC_E => "UPC-E".into(),
        BarcodeFormat::CODE_128 => "Code 128".into(),
        BarcodeFormat::CODE_39 => "Code 39".into(),
        BarcodeFormat::CODE_93 => "Code 93".into(),
        BarcodeFormat::CODABAR => "Codabar".into(),
        BarcodeFormat::ITF => "ITF".into(),
        BarcodeFormat::RSS_14 => "GS1 DataBar".into(),
        BarcodeFormat::RSS_EXPANDED => "GS1 DataBar Expanded".into(),
        other => other.to_string().to_uppercase(),
    }
}

/// Rows above and below `y` that look like the same bars (between `x0` and `x1`).
fn bar_extent(luma: &[u8], width: u32, height: u32, x0: f32, x1: f32, y: f32) -> (f32, f32) {
    let (x0, x1) = (x0.max(0.) as usize, (x1.min(width as f32) as usize).max(x0.max(0.) as usize + 1));
    let row = |y: u32| -> Option<&[u8]> {
        let start = y as usize * width as usize;
        luma.get(start + x0..start + x1)
    };
    let mid = (y.max(0.) as u32).min(height.saturating_sub(1));
    let Some(reference) = row(mid) else { return (y, y) };
    let similar = |y: u32| {
        row(y).is_some_and(|r| {
            let diff: u32 = r.iter().zip(reference).map(|(a, b)| a.abs_diff(*b) as u32).sum();
            diff / (r.len() as u32).max(1) < 48
        })
    };
    let limit = (x1 - x0) as u32;
    let (mut top, mut bottom) = (mid, mid);
    while top > 0 && mid - top < limit && similar(top - 1) {
        top -= 1;
    }
    while bottom + 1 < height && bottom - mid < limit && similar(bottom + 1) {
        bottom += 1;
    }
    (top as f32, bottom as f32)
}

// ---------------------------------------------------------------------------
// Screen

/// Each monitor as a picture (physical pixels).
fn capture_monitors() -> Vec<Capture> {
    unsafe extern "system" fn collect(_: HMONITOR, _: HDC, rect: *mut RECT, out: LPARAM) -> BOOL {
        let list = unsafe { &mut *(out.0 as *mut Vec<RECT>) };
        list.push(unsafe { *rect });
        TRUE
    }
    let mut rects: Vec<RECT> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut rects as *mut Vec<RECT> as isize));
    }
    rects.into_iter().filter_map(capture_rect).collect()
}

/// Screen pixels of (left, top, right, bottom): (width, height, BGRA).
pub fn capture_area(rect: (i32, i32, i32, i32)) -> Option<(u32, u32, Vec<u8>)> {
    let (left, top, right, bottom) = rect;
    capture_rect(RECT { left, top, right, bottom }).map(|c| (c.width, c.height, c.bgra))
}

fn capture_rect(r: RECT) -> Option<Capture> {
    let (w, h) = ((r.right - r.left).max(0), (r.bottom - r.top).max(0));
    if w == 0 || h == 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative: rows top to bottom.
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let result = match CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) if !bits.is_null() => {
                let old = SelectObject(mem, bitmap.into());
                let copied = BitBlt(mem, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY | CAPTUREBLT).is_ok();
                let pixels = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                let bgra = copied.then(|| {
                    // A screen capture leaves alpha at 0: make it opaque.
                    let mut bgra = pixels.to_vec();
                    bgra.chunks_exact_mut(4).for_each(|p| p[3] = 255);
                    bgra
                });
                SelectObject(mem, old);
                let _ = DeleteObject(bitmap.into());
                bgra.map(|bgra| Capture { label: String::new(), width: w as u32, height: h as u32, bgra })
            }
            _ => None,
        };
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        result
    }
}

fn bgra_to_luma(pixels: &[u8]) -> Vec<u8> {
    pixels
        .chunks_exact(4)
        .map(|p| ((p[2] as u32 * 299 + p[1] as u32 * 587 + p[0] as u32 * 114) / 1000) as u8)
        .collect()
}

// ---------------------------------------------------------------------------
// Clipboard

/// The clipboard's image, if it has one: PNG when offered, else the bitmap (DIB).
fn clipboard_image() -> Option<Capture> {
    let png_format = unsafe { RegisterClipboardFormatW(w!("PNG")) };
    let (bytes, is_png) = unsafe {
        OpenClipboard(None).ok()?;
        let read = |format: u32| -> Option<Vec<u8>> {
            IsClipboardFormatAvailable(format).ok()?;
            let handle = GetClipboardData(format).ok()?;
            let global = HGLOBAL(handle.0);
            let size = GlobalSize(global);
            let ptr = GlobalLock(global) as *const u8;
            if ptr.is_null() || size == 0 {
                return None;
            }
            let data = std::slice::from_raw_parts(ptr, size).to_vec();
            let _ = GlobalUnlock(global);
            Some(data)
        };
        let found = match read(png_format) {
            Some(png) => Some((png, true)),
            None => read(CF_DIBV5.0 as u32).or_else(|| read(CF_DIB.0 as u32)).map(|dib| (dib, false)),
        };
        let _ = CloseClipboard();
        found?
    };
    let image = if is_png {
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?
    } else {
        image::load_from_memory_with_format(&dib_to_bmp(&bytes)?, image::ImageFormat::Bmp).ok()?
    };
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    // BGRA, with transparent parts on white (as an image viewer would show them).
    let bgra = rgba
        .pixels()
        .flat_map(|p| {
            let [r, g, b, a] = p.0;
            let over = |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32)) / 255) as u8;
            [over(b), over(g), over(r), 255]
        })
        .collect();
    Some(Capture { label: "剪贴板图片".into(), width, height, bgra })
}

/// A clipboard DIB is a BMP file without its 14-byte file header; put one in front.
fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |i: usize| dib.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let header = u32_at(0)? as usize;
    let bit_count = u16::from_le_bytes([*dib.get(14)?, *dib.get(15)?]);
    let compression = u32_at(16)?;
    let colors_used = u32_at(32)? as usize;
    // BI_BITFIELDS with a plain 40-byte header keeps its three masks after it.
    let masks = if header == 40 && compression == 3 { 12 } else { 0 };
    let palette = match colors_used {
        0 if bit_count <= 8 => (1usize << bit_count) * 4,
        n => n * 4,
    };
    let offset = 14 + header + masks + palette;
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&(offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draw `contents` as `format` (with a white margin) and scan it.
    fn round_trip(contents: &str, format: BarcodeFormat, w: i32, h: i32) -> ScanOutput {
        use rxing::Writer;
        let matrix = rxing::MultiFormatWriter.encode(contents, &format, w, h).unwrap();
        let (mw, mh) = (matrix.getWidth(), matrix.getHeight());
        let pad = 60u32;
        let (iw, ih) = (mw + pad * 2, mh + pad * 2);
        let mut bgra = vec![255u8; (iw * ih * 4) as usize];
        for y in 0..mh {
            for x in 0..mw {
                if matrix.get(x, y) {
                    let i = (((y + pad) * iw + x + pad) * 4) as usize;
                    bgra[i..i + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
        }
        find(&[Capture { label: "测试".into(), width: iw, height: ih, bgra }])
    }

    #[test]
    fn decodes_qr_codes_and_barcodes() {
        let out = round_trip("https://github.com/LovelyTeams/LovelyLauncher 你好", BarcodeFormat::QR_CODE, 240, 240);
        assert_eq!(out.codes.len(), 1);
        assert_eq!(out.sources.len(), 1);
        let qr = &out.codes[0];
        assert_eq!(qr.kind, "二维码");
        assert_eq!(qr.text, "https://github.com/LovelyTeams/LovelyLauncher 你好");
        assert!(qr.is_link());
        // Its box sits around the middle of the picture, where the code was drawn.
        let (x, y, w, h) = qr.rect;
        assert!(x > 0.05 && y > 0.05 && x + w < 0.95 && y + h < 0.95, "{:?}", qr.rect);
        // The thumbnail shows the code with its surroundings and the coloured outline.
        let (tw, th, px) = qr.thumb.clone().unwrap();
        assert!(tw <= 160 && th <= 110);
        let red = px.chunks(4).filter(|p| p[2] > 180 && p[1] < 120 && p[0] < 120).count();
        assert!(red > 0, "outline in the first colour");

        let ean = round_trip("5901234123457", BarcodeFormat::EAN_13, 300, 120);
        assert_eq!(ean.codes.len(), 1);
        assert_eq!(ean.codes[0].text, "5901234123457");
        assert_eq!(ean.codes[0].kind, "条形码（EAN-13）");
        // In pixels (the picture isn't square): wider than tall.
        let (_, _, w, h) = ean.codes[0].rect;
        let (pw, ph, _) = &ean.sources[0].preview;
        assert!(w * *pw as f32 > h * *ph as f32, "a barcode's box is wide: {:?}", ean.codes[0].rect);
    }

    #[test]
    fn dib_header_gets_file_header() {
        let mut dib = vec![0u8; 40 + 4];
        dib[0] = 40; // biSize
        dib[4] = 1; // width 1
        dib[8] = 1; // height 1
        dib[12] = 1; // planes
        dib[14] = 32; // bit count
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32::from_le_bytes([bmp[10], bmp[11], bmp[12], bmp[13]]), 54);
        let img = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp);
        assert!(img.is_ok());
    }
}
