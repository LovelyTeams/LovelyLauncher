//! Custom window background: a picture or a looping video, with blur, brightness,
//! saturation and a veil in the theme's colour drawn over it.
//!
//! Pictures are processed once on a background thread. Videos are decoded by FFmpeg
//! with the same effects as filters, at a fixed rate, and paced here so that the
//! playback speed setting simply changes how fast the source is consumed.

use std::{
    io::Read,
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crate::config::Wallpaper;
use crate::preview::ffmpeg::CREATE_NO_WINDOW;

pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "jfif", "bmp", "webp"];
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "mov", "webm", "avi", "wmv", "m4v", "flv", "gif"];

/// Largest side a wallpaper is processed at.
pub const MAX_SIZE: u32 = 2560;
/// Frames per second a video wallpaper is shown at.
const VIDEO_FPS: f64 = 30.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Image,
    Video,
}

pub fn kind(path: &str) -> Option<Kind> {
    let ext = Path::new(path).extension()?.to_string_lossy().to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        Some(Kind::Image)
    } else if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(Kind::Video)
    } else {
        None
    }
}

/// Blur radius in pixels of an image `width` wide (the setting is relative to ~1280 px).
fn sigma(settings: &Wallpaper, width: u32) -> f32 {
    settings.blur as f32 * width as f32 / 1280.
}

/// Load and process a picture: fit within [`MAX_SIZE`], blur, brightness and
/// saturation. Returns BGRA pixels, as GPUI wants them.
pub fn render_image(path: &Path, settings: &Wallpaper) -> Result<image::RgbaImage, String> {
    let picture = image::open(path).map_err(|e| format!("无法打开图片：{e}"))?;
    let picture = if picture.width() > MAX_SIZE || picture.height() > MAX_SIZE {
        picture.resize(MAX_SIZE, MAX_SIZE, image::imageops::FilterType::Triangle)
    } else {
        picture
    };
    let mut rgba = picture.to_rgba8();
    let blur = sigma(settings, rgba.width());
    if blur >= 0.5 {
        rgba = image::imageops::fast_blur(&rgba, blur);
    }
    adjust(&mut rgba, settings);
    for px in rgba.pixels_mut() {
        px.0.swap(0, 2);
    }
    Ok(rgba)
}

/// Brightness (added, ±50% at the ends) and saturation (scaled around grey).
fn adjust(rgba: &mut image::RgbaImage, settings: &Wallpaper) {
    let lift = settings.brightness as f32 / 100. * 127.;
    let saturation = settings.saturation as f32 / 100.;
    if lift == 0. && saturation == 1. {
        return;
    }
    for px in rgba.pixels_mut() {
        let [r, g, b, _] = px.0;
        let grey = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
        for (i, c) in [r, g, b].into_iter().enumerate() {
            let v = grey + (c as f32 - grey) * saturation + lift;
            px.0[i] = v.round().clamp(0., 255.) as u8;
        }
    }
}

/// A looping, silent video decoded by FFmpeg into BGRA frames of `size`.
pub struct VideoPlayer {
    child: Child,
    pub size: (u32, u32),
    latest: Arc<Mutex<Option<Vec<u8>>>>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl VideoPlayer {
    /// `max_width`: the window's width in device pixels (frames are scaled down to it).
    pub fn start(ffmpeg: &Path, path: &Path, settings: &Wallpaper, max_width: u32) -> Result<Self, String> {
        let probe = crate::preview::ffmpeg::probe(ffmpeg, path)?;
        let (w, h) = probe.video.ok_or("这个文件里没有视频画面")?;
        let out_w = (w.min(max_width.clamp(320, MAX_SIZE)) / 2 * 2).max(2);
        let out_h = ((h as f64 * out_w as f64 / w as f64).round() as u32 / 2 * 2).max(2);
        let speed = settings.speed.clamp(0.25, 4.);
        let mut filters = vec![
            format!("setpts=PTS/{speed}"),
            format!("fps={VIDEO_FPS}"),
            format!("scale={out_w}:{out_h}"),
        ];
        let blur = sigma(settings, out_w);
        if blur >= 0.5 {
            filters.push(format!("gblur=sigma={blur:.2}"));
        }
        if settings.brightness != 0 || settings.saturation != 100 {
            filters.push(format!(
                "eq=brightness={:.3}:saturation={:.3}",
                settings.brightness as f32 / 200.,
                settings.saturation as f32 / 100.
            ));
        }
        let mut child = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-stream_loop", "-1", "-i"])
            .arg(path)
            .args(["-an", "-vf", &filters.join(","), "-pix_fmt", "bgra", "-f", "rawvideo", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("无法运行 FFmpeg：{e}"))?;
        let mut stdout = child.stdout.take().ok_or("无法读取 FFmpeg 输出")?;
        let latest = Arc::new(Mutex::new(None));
        let paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let frame_len = (out_w * out_h * 4) as usize;
        {
            let (latest, paused, stop) = (latest.clone(), paused.clone(), stop.clone());
            std::thread::Builder::new()
                .name("wallpaper-video".into())
                .spawn(move || {
                    let step = Duration::from_secs_f64(1. / VIDEO_FPS);
                    let mut next = Instant::now();
                    loop {
                        // While paused nothing is read, so FFmpeg blocks and uses no CPU.
                        while paused.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(100));
                            next = Instant::now();
                        }
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        let mut frame = vec![0u8; frame_len];
                        if stdout.read_exact(&mut frame).is_err() {
                            return;
                        }
                        *latest.lock().unwrap() = Some(frame);
                        next += step;
                        let now = Instant::now();
                        if next > now {
                            std::thread::sleep(next - now);
                        } else {
                            next = now;
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
        }
        Ok(Self { child, size: (out_w, out_h), latest, paused, stop })
    }

    /// The newest frame, if there is one that hasn't been taken yet.
    pub fn take_frame(&self) -> Option<Vec<u8>> {
        self.latest.lock().unwrap().take()
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(kind("C:/a/b.JPG"), Some(Kind::Image));
        assert_eq!(kind("x.webm"), Some(Kind::Video));
        assert_eq!(kind("x.txt"), None);
    }

    #[test]
    fn brightness_and_saturation() {
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([200, 100, 50, 255]));
        adjust(&mut img, &Wallpaper { saturation: 0, ..Default::default() });
        let [r, g, b, a] = img.get_pixel(0, 0).0;
        assert!(r == g && g == b && a == 255, "fully desaturated is grey");
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([100, 100, 100, 255]));
        adjust(&mut img, &Wallpaper { brightness: 50, ..Default::default() });
        assert!(img.get_pixel(0, 0).0[0] > 150);
    }
}
