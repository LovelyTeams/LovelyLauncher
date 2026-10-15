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
    /// Where it was found: "剪贴板图片", "屏幕 1", …
    pub source: String,
    /// A grey picture of the code (width, height, BGRA), so results can be told apart.
    pub thumb: Option<(u32, u32, Vec<u8>)>,
}

impl Code {
    pub fn is_link(&self) -> bool {
        let t = self.text.trim().to_ascii_lowercase();
        t.starts_with("http://") || t.starts_with("https://")
    }
}

/// Scan the clipboard image (if any) and every monitor; duplicates are dropped.
pub fn scan_all() -> Vec<Code> {
    let mut codes = Vec::new();
    if let Some((luma, w, h)) = clipboard_image() {
        codes.extend(decode(luma, w, h, "剪贴板图片"));
    }
    let screens = capture_monitors();
    let several = screens.len() > 1;
    for (i, (luma, w, h)) in screens.into_iter().enumerate() {
        let source = if several { format!("屏幕 {}", i + 1) } else { "屏幕".to_string() };
        codes.extend(decode(luma, w, h, &source));
    }
    // The same code twice (say on screen and copied too) is listed once.
    let mut unique: Vec<Code> = Vec::new();
    for code in codes {
        if !unique.iter().any(|u| u.kind == code.kind && u.text == code.text) {
            unique.push(code);
        }
    }
    unique
}

fn decode(luma: Vec<u8>, width: u32, height: u32, source: &str) -> Vec<Code> {
    // The reader takes the pixels; keep a copy to cut the thumbnails from.
    let pixels = luma.clone();
    detect_multiple_in_luma(luma, width, height)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| !r.getText().trim().is_empty())
        .map(|r| Code {
            kind: kind_name(r.getBarcodeFormat()),
            text: r.getText().to_string(),
            source: source.to_string(),
            thumb: thumbnail(&pixels, width, height, r.getPoints()),
        })
        .collect()
}

/// Cut the code out of the image around the points the reader reported (finder
/// pattern centres for 2D codes, the scan line for barcodes) and shrink it.
fn thumbnail(luma: &[u8], width: u32, height: u32, points: &[Point]) -> Option<(u32, u32, Vec<u8>)> {
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
        // A barcode is reported as one scan line: follow its bars up and down.
        let (top, bottom) = bar_extent(luma, width, height, x0, x1, (y0 + y1) / 2.);
        let pad = 10. + w * 0.04;
        (x0 - pad, top - 8., x1 + pad, bottom + 8.)
    } else {
        // Finder pattern centres sit inside the code; widen by a share of its size.
        let pad = w.max(h) * 0.25 + 6.;
        (x0 - pad, y0 - pad, x1 + pad, y1 + pad)
    };
    let x0 = x0.max(0.) as u32;
    let y0 = y0.max(0.) as u32;
    let x1 = (x1.min(width as f32) as u32).max(x0 + 1);
    let y1 = (y1.min(height as f32) as u32).max(y0 + 1);
    let (cw, ch) = (x1 - x0, y1 - y0);
    let mut crop = image::GrayImage::new(cw, ch);
    for y in 0..ch {
        let row = ((y0 + y) * width + x0) as usize;
        for x in 0..cw {
            crop.put_pixel(x, y, image::Luma([*luma.get(row + x as usize)?]));
        }
    }
    // Fit within 120 x 88.
    let scale = (120. / cw as f32).min(88. / ch as f32).min(1.);
    let (tw, th) = (((cw as f32 * scale) as u32).max(1), ((ch as f32 * scale) as u32).max(1));
    let small = image::imageops::resize(&crop, tw, th, image::imageops::FilterType::Triangle);
    let bgra = small.pixels().flat_map(|p| [p.0[0], p.0[0], p.0[0], 255]).collect();
    Some((tw, th, bgra))
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

/// Each monitor as a grey-scale image (physical pixels).
fn capture_monitors() -> Vec<(Vec<u8>, u32, u32)> {
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

fn capture_rect(r: RECT) -> Option<(Vec<u8>, u32, u32)> {
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
                let luma = copied.then(|| bgra_to_luma(pixels));
                SelectObject(mem, old);
                let _ = DeleteObject(bitmap.into());
                luma.map(|l| (l, w as u32, h as u32))
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
fn clipboard_image() -> Option<(Vec<u8>, u32, u32)> {
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
    let grey = image.to_luma8();
    let (w, h) = grey.dimensions();
    Some((grey.into_raw(), w, h))
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

    /// Draw `contents` as `format` (with a white margin) and decode it again.
    fn round_trip(contents: &str, format: BarcodeFormat, w: i32, h: i32) -> Vec<Code> {
        use rxing::Writer;
        let matrix = rxing::MultiFormatWriter.encode(contents, &format, w, h).unwrap();
        let (mw, mh) = (matrix.getWidth(), matrix.getHeight());
        let pad = 20u32;
        let (iw, ih) = (mw + pad * 2, mh + pad * 2);
        let mut luma = vec![255u8; (iw * ih) as usize];
        for y in 0..mh {
            for x in 0..mw {
                if matrix.get(x, y) {
                    luma[((y + pad) * iw + x + pad) as usize] = 0;
                }
            }
        }
        decode(luma, iw, ih, "测试")
    }

    #[test]
    fn decodes_qr_codes_and_barcodes() {
        let qr = round_trip("https://github.com/LovelyTeams/LovelyLauncher 你好", BarcodeFormat::QR_CODE, 240, 240);
        assert_eq!(qr.len(), 1);
        assert_eq!(qr[0].kind, "二维码");
        // The thumbnail is the code, not the whole picture: mostly black and white.
        let (tw, th, px) = qr[0].thumb.clone().unwrap();
        assert!(tw > 20 && th > 20 && tw <= 120 && th <= 88);
        let dark = px.chunks(4).filter(|p| p[0] < 128).count();
        assert!(dark * 5 > px.len() / 4, "thumbnail should contain the code");
        assert_eq!(qr[0].text, "https://github.com/LovelyTeams/LovelyLauncher 你好");
        assert!(qr[0].is_link());

        let ean = round_trip("5901234123457", BarcodeFormat::EAN_13, 300, 120);
        assert_eq!(ean.len(), 1);
        assert_eq!(ean[0].text, "5901234123457");
        assert_eq!(ean[0].kind, "条形码（EAN-13）");
        let (tw, th, _) = ean[0].thumb.clone().unwrap();
        assert!(tw > th, "a barcode thumbnail is wide: {tw}x{th}");
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
