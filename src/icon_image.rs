//! Custom tool icons: decode user images (or extract an exe/lnk icon) and fit
//! them into a small square so they stay cheap to keep on the GPU.

use std::path::{Path, PathBuf};

use image::{RgbaImage, imageops::FilterType};

use crate::{config::Config, win};

pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "ico", "bmp", "gif", "webp"];
pub const PROGRAM_EXTENSIONS: &[&str] = &["exe", "lnk", "dll", "msc", "url"];

fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Decode an image file and fit it (aspect preserved) into a `size`² RGBA square.
pub fn load_square(path: &Path, size: u32) -> Option<RgbaImage> {
    let img = image::open(path).ok()?.into_rgba8();
    Some(fit_square(&img, size))
}

pub fn fit_square(img: &RgbaImage, size: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    if w == size && h == size {
        return img.clone();
    }
    let scale = size as f32 / w.max(h) as f32;
    let (nw, nh) = (
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    );
    let resized = image::imageops::resize(img, nw, nh, FilterType::Lanczos3);
    let mut canvas = RgbaImage::new(size, size);
    image::imageops::overlay(&mut canvas, &resized, ((size - nw) / 2) as i64, ((size - nh) / 2) as i64);
    canvas
}

pub fn rgba_to_bgra(img: &mut RgbaImage) {
    for px in img.pixels_mut() {
        px.0.swap(0, 2);
    }
}

/// Turn a picked file into a 128px PNG inside our icons folder.
/// Images are resized; programs / shortcuts contribute their shell icon.
/// Blocking; call from a background thread.
pub fn import(source: &Path) -> Result<PathBuf, String> {
    let ext = ext(source);
    let square = if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        load_square(source, 128).ok_or("无法读取这张图片，请换一张 PNG / JPG / ICO")?
    } else if PROGRAM_EXTENSIONS.contains(&ext.as_str()) {
        let bmp = win::file_icon(source).ok_or("无法从该文件中提取图标")?;
        let mut img = RgbaImage::from_raw(bmp.width, bmp.height, bmp.bgra).ok_or("图标数据无效")?;
        rgba_to_bgra(&mut img);
        fit_square(&img, 128)
    } else {
        return Err("不支持的文件类型，请选择图片或程序".into());
    };

    let dir = Config::icons_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建图标目录：{e}"))?;
    // A fresh name each time so cached renders of an older icon never linger.
    let dest = dir.join(format!("{}.png", crate::config::new_id()));
    square
        .save_with_format(&dest, image::ImageFormat::Png)
        .map_err(|e| format!("保存图标失败：{e}"))?;
    Ok(dest)
}
