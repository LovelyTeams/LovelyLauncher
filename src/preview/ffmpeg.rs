//! Locating, downloading and probing FFmpeg (used for video and less common audio).

use std::{
    fs,
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

use gpui::{Global, SharedString};

use crate::download::{self, Progress};

pub const DOWNLOAD_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
pub const DOWNLOAD_SIZE_HINT: &str = "约 110 MB";
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// App-wide FFmpeg status, shared by the settings page and preview windows.
#[derive(Default)]
pub struct FfmpegState {
    pub path: Option<PathBuf>,
    pub download: Option<Arc<Progress>>,
    pub error: Option<SharedString>,
}

impl Global for FfmpegState {}

pub fn tools_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("LovelyLauncher")
        .join("tools")
}

/// The configured ffmpeg.exe, else our downloaded copy, else one on PATH.
pub fn find(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() && Path::new(configured).is_file() {
        return Some(PathBuf::from(configured));
    }
    let ours = tools_dir().join("ffmpeg.exe");
    if ours.is_file() {
        return Some(ours);
    }
    crate::win::search_path("ffmpeg.exe")
}

/// Download the essentials build and extract `ffmpeg.exe`. Blocking.
pub fn download_and_install(progress: &Progress) -> Result<PathBuf, String> {
    let dir = tools_dir();
    let zip_path = dir.join("ffmpeg-download.zip");
    download::download(DOWNLOAD_URL, &zip_path, progress)?;
    let result = extract_ffmpeg(&zip_path, &dir.join("ffmpeg.exe"));
    let _ = fs::remove_file(&zip_path);
    result
}

fn extract_ffmpeg(zip_path: &Path, dest: &Path) -> Result<PathBuf, String> {
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| format!("压缩包损坏：{e}"))?;
    let name = (0..archive.len())
        .filter_map(|i| archive.name_for_index(i).and_then(|r| r.ok()).map(|n| n.to_string()))
        .find(|n| n.replace('\\', "/").ends_with("bin/ffmpeg.exe"))
        .ok_or("压缩包里没有找到 ffmpeg.exe")?;
    let mut entry = archive.by_name(&name).map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("part");
    let mut out = fs::File::create(&tmp).map_err(|e| format!("无法写入：{e}"))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = entry.read(&mut buf).map_err(|e| format!("解压失败：{e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| format!("写入失败：{e}"))?;
    }
    drop(out);
    fs::rename(&tmp, dest).map_err(|e| format!("保存失败：{e}"))?;
    Ok(dest.to_path_buf())
}

#[derive(Clone, Debug, Default)]
pub struct Probe {
    pub duration: Option<f64>,
    pub video: Option<(u32, u32)>,
    pub audio: bool,
}

/// Read duration / streams from `ffmpeg -i` output.
pub fn probe(ffmpeg: &Path, media: &Path) -> Result<Probe, String> {
    let output = Command::new(ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(media)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("无法运行 FFmpeg：{e}"))?;
    let text = String::from_utf8_lossy(&output.stderr);
    let probe = parse_probe(&text);
    if probe.video.is_none() && !probe.audio {
        return Err("FFmpeg 无法识别这个文件".into());
    }
    Ok(probe)
}

fn parse_probe(text: &str) -> Probe {
    let mut probe = Probe::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Duration:") {
            let stamp = rest.split(',').next().unwrap_or("").trim();
            let parts: Vec<f64> = stamp.split(':').filter_map(|p| p.parse().ok()).collect();
            if parts.len() == 3 {
                probe.duration = Some(parts[0] * 3600. + parts[1] * 60. + parts[2]);
            }
        } else if line.starts_with("Stream #") {
            if line.contains("Audio:") {
                probe.audio = true;
            } else if line.contains("Video:") && !line.contains("attached pic") && probe.video.is_none() {
                probe.video = line
                    .split([',', ' '])
                    .filter_map(|tok| {
                        let (w, h) = tok.split_once('x')?;
                        let (w, h) = (w.parse::<u32>().ok()?, h.parse::<u32>().ok()?);
                        (w >= 16 && h >= 16).then_some((w, h))
                    })
                    .next();
            }
        }
    }
    probe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_output() {
        let text = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':\n  Duration: 00:01:02.50, start: 0.000000, bitrate: 1000 kb/s\n  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 1920x1080 [SAR 1:1 DAR 16:9], 30 fps\n  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo\n";
        let p = parse_probe(text);
        assert_eq!(p.duration, Some(62.5));
        assert_eq!(p.video, Some((1920, 1080)));
        assert!(p.audio);

        let cover = "  Duration: 00:03:00.00, start: 0\n  Stream #0:0: Audio: mp3, 44100 Hz\n  Stream #0:1: Video: mjpeg, yuvj420p, 600x600, 90k tbr (attached pic)\n";
        let p = parse_probe(cover);
        assert_eq!(p.video, None);
        assert!(p.audio);
    }
}
