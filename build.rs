//! Generates the app icon (.ico), tray icon and in-app logo from `assets/icon.png`.

use std::{env, fs, io::Cursor, path::PathBuf};

use image::{ImageFormat, RgbaImage, imageops::FilterType};

const SOURCE: &str = "assets/icon.png";

fn main() {
    println!("cargo:rerun-if-changed={SOURCE}");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let source = image::open(SOURCE)
        .unwrap_or_else(|e| panic!("failed to read {SOURCE}: {e}"))
        .into_rgba8();

    // Raw pixel buffers consumed at runtime via include_bytes!, so nothing is decoded on startup.
    fs::write(out_dir.join("tray_32.rgba"), resize(&source, 32).into_raw()).unwrap();
    let mut logo = resize(&source, 96).into_raw();
    for px in logo.chunks_exact_mut(4) {
        px.swap(0, 2); // GPUI images are BGRA
    }
    fs::write(out_dir.join("logo_96.bgra"), logo).unwrap();

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico_path = out_dir.join("app.ico");
    fs::write(&ico_path, build_ico(&source, &[16, 20, 24, 32, 40, 48, 64, 128, 256])).unwrap();

    // GPUI loads resource id 1 as the window/taskbar icon.
    let rc_path = out_dir.join("app.rc");
    let ico = ico_path.display().to_string().replace('\\', "/");
    fs::write(&rc_path, format!("1 ICON \"{ico}\"\n")).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}

fn resize(src: &RgbaImage, size: u32) -> RgbaImage {
    image::imageops::resize(src, size, size, FilterType::Lanczos3)
}

/// Multi-size .ico with PNG-compressed entries (supported since Windows Vista).
fn build_ico(src: &RgbaImage, sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let mut png = Vec::new();
            resize(src, s)
                .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
                .unwrap();
            png
        })
        .collect();

    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // color planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}
