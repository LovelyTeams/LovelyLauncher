//! Download a file with progress and cancellation. Blocking; run on a background thread.

use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use crate::http;

#[derive(Default)]
pub struct Progress {
    pub downloaded: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    /// Fraction in 0..=1, or `None` while the size is unknown.
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| self.downloaded.load(Ordering::Relaxed) as f32 / total as f32)
    }
}

/// Download `url` to `dest`, writing to `dest.part` first so a partial file is never run.
pub fn download(url: &str, dest: &Path, progress: &Progress) -> Result<(), String> {
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    }
    let part = dest.with_extension("part");
    match fetch_to(url, &part, progress) {
        Ok(()) => fs::rename(&part, dest).map_err(|e| format!("保存文件失败：{e}")),
        Err(err) => {
            let _ = fs::remove_file(&part);
            Err(err)
        }
    }
}

fn fetch_to(url: &str, part: &Path, progress: &Progress) -> Result<(), String> {
    let mut file = File::create(part).map_err(|e| format!("无法写入文件：{e}"))?;
    let mut write_error = None;
    let status = http::send(
        &http::Request {
            method: "GET",
            url,
            headers: &[],
            body: None,
            receive_timeout_ms: 60_000,
        },
        |head| {
            if let Some(len) = head.content_length {
                progress.total.store(len, Ordering::Relaxed);
            }
        },
        |chunk| {
            if progress.cancel.load(Ordering::Relaxed) {
                return false;
            }
            if let Err(e) = file.write_all(chunk) {
                write_error = Some(format!("写入文件失败：{e}"));
                return false;
            }
            progress.downloaded.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            true
        },
    )?;
    if let Some(err) = write_error {
        return Err(err);
    }
    if progress.cancel.load(Ordering::Relaxed) {
        return Err("已取消下载".into());
    }
    if status != 200 {
        return Err(format!("下载失败，服务器返回 HTTP {status}"));
    }
    file.flush().map_err(|e| format!("写入文件失败：{e}"))?;
    let total = progress.total.load(Ordering::Relaxed);
    if total > 0 && progress.downloaded.load(Ordering::Relaxed) != total {
        return Err("下载不完整，请重试".into());
    }
    Ok(())
}
