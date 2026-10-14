//! Audio / video playback for previews.
//!
//! - Common audio (mp3, wav, wma, mid) plays through Windows MCI — nothing to install.
//! - Video and other audio go through FFmpeg: one process decodes paced video frames
//!   (raw BGRA over a pipe), another decodes PCM that we play with waveOut. Pausing or
//!   seeking restarts the processes at the new position.

use std::{
    io::Read,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use windows::{
    Win32::Media::{
        Audio::{
            CALLBACK_NULL, HWAVEOUT, WAVE_FORMAT_PCM, WAVE_MAPPER, WAVEFORMATEX, WAVEHDR, WHDR_DONE,
            waveOutClose, waveOutOpen, waveOutPrepareHeader, waveOutReset, waveOutUnprepareHeader,
            waveOutWrite,
        },
        Multimedia::mciSendStringW,
    },
    core::{HSTRING, PSTR},
};

use super::ffmpeg::{CREATE_NO_WINDOW, Probe};

/// Extensions MCI ("mpegvideo" device) plays reliably on stock Windows.
pub const MCI_AUDIO: &[&str] = &["mp3", "wav", "wma", "mid", "midi"];

/// The newest decoded video frame, handed from the decoder thread to the UI.
#[derive(Default)]
pub struct FrameSlot {
    pub seq: AtomicU64,
    pub frame: Mutex<Option<Vec<u8>>>,
    pub ended: AtomicBool,
}

pub enum Backend {
    Mci { alias: String },
    Ffmpeg { exe: PathBuf, procs: Vec<Child>, stop: Arc<AtomicBool> },
}

pub struct Player {
    path: PathBuf,
    backend: Backend,
    pub duration: Option<f64>,
    /// Size of the frames we ask FFmpeg for (fits the preview).
    pub video_size: Option<(u32, u32)>,
    pub has_audio: bool,
    pub frames: Arc<FrameSlot>,
    playing: bool,
    /// Clock for the FFmpeg backend: position at `started`, and when it started.
    offset: f64,
    started: Option<Instant>,
}

fn mci(command: &str) -> Result<String, u32> {
    let mut buf = [0u16; 256];
    let err = unsafe { mciSendStringW(&HSTRING::from(command), Some(&mut buf), None) };
    if err != 0 {
        return Err(err);
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(String::from_utf16_lossy(&buf[..len]))
}

impl Player {
    pub fn open_mci(path: &Path) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let alias = format!("lovely{}", NEXT.fetch_add(1, Ordering::Relaxed));
        mci(&format!("open \"{}\" type mpegvideo alias {alias}", path.display()))
            .map_err(|code| format!("无法播放这个音频（MCI 错误 {code}）"))?;
        let _ = mci(&format!("set {alias} time format milliseconds"));
        let duration = mci(&format!("status {alias} length"))
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .map(|ms| ms / 1000.);
        Ok(Self {
            path: path.to_path_buf(),
            backend: Backend::Mci { alias },
            duration,
            video_size: None,
            has_audio: true,
            frames: Arc::default(),
            playing: false,
            offset: 0.,
            started: None,
        })
    }

    /// `max` is the largest frame size worth decoding (the preview's size).
    pub fn open_ffmpeg(exe: &Path, path: &Path, probe: &Probe, max: (u32, u32)) -> Self {
        let video_size = probe.video.map(|(w, h)| {
            let scale = (max.0 as f64 / w as f64).min(max.1 as f64 / h as f64).min(1.);
            // Even dimensions keep every pixel format happy.
            let even = |v: f64| ((v as u32).max(2)) & !1;
            (even(w as f64 * scale), even(h as f64 * scale))
        });
        Self {
            path: path.to_path_buf(),
            backend: Backend::Ffmpeg {
                exe: exe.to_path_buf(),
                procs: Vec::new(),
                stop: Arc::new(AtomicBool::new(false)),
            },
            duration: probe.duration,
            video_size,
            has_audio: probe.audio,
            frames: Arc::default(),
            playing: false,
            offset: 0.,
            started: None,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn position(&self) -> f64 {
        let pos = match &self.backend {
            Backend::Mci { alias } => mci(&format!("status {alias} position"))
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .map(|ms| ms / 1000.)
                .unwrap_or(0.),
            Backend::Ffmpeg { .. } => {
                self.offset + self.started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.)
            }
        };
        match self.duration {
            Some(d) => pos.min(d),
            None => pos,
        }
    }

    /// True once playback reached the end (the UI then resets to the start).
    pub fn finished(&self) -> bool {
        if !self.playing {
            return false;
        }
        match &self.backend {
            Backend::Mci { alias } => mci(&format!("status {alias} mode")).is_ok_and(|m| m.trim() == "stopped"),
            Backend::Ffmpeg { .. } => {
                let done_by_clock = self.duration.is_some_and(|d| self.position() >= d - 0.05);
                let video_done = self.video_size.is_none() || self.frames.ended.load(Ordering::Relaxed);
                done_by_clock && video_done
            }
        }
    }

    pub fn play(&mut self) {
        if self.playing {
            return;
        }
        if self.duration.is_some_and(|d| self.position() >= d - 0.05) {
            self.seek(0.);
        }
        match &mut self.backend {
            Backend::Mci { alias } => {
                let _ = mci(&format!("play {alias}"));
            }
            Backend::Ffmpeg { .. } => self.start_ffmpeg(),
        }
        self.playing = true;
    }

    pub fn pause(&mut self) {
        if !self.playing {
            return;
        }
        match &mut self.backend {
            Backend::Mci { alias } => {
                let _ = mci(&format!("pause {alias}"));
            }
            Backend::Ffmpeg { .. } => {
                self.offset = self.position();
                self.started = None;
                self.stop_ffmpeg();
            }
        }
        self.playing = false;
    }

    pub fn toggle(&mut self) {
        if self.playing { self.pause() } else { self.play() }
    }

    pub fn seek(&mut self, seconds: f64) {
        let seconds = seconds.max(0.).min(self.duration.unwrap_or(f64::MAX));
        match &mut self.backend {
            Backend::Mci { alias } => {
                let _ = mci(&format!("seek {alias} to {}", (seconds * 1000.) as u64));
                if self.playing {
                    let _ = mci(&format!("play {alias}"));
                }
            }
            Backend::Ffmpeg { .. } => {
                let was_playing = self.playing;
                self.stop_ffmpeg();
                self.offset = seconds;
                self.started = None;
                self.playing = false;
                if was_playing {
                    self.start_ffmpeg();
                    self.playing = true;
                }
            }
        }
    }

    fn stop_ffmpeg(&mut self) {
        if let Backend::Ffmpeg { procs, stop, .. } = &mut self.backend {
            stop.store(true, Ordering::Relaxed);
            for mut child in procs.drain(..) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    fn start_ffmpeg(&mut self) {
        let Backend::Ffmpeg { exe, procs, stop } = &mut self.backend else {
            return;
        };
        let stop_flag = Arc::new(AtomicBool::new(false));
        *stop = stop_flag.clone();
        let start = format!("{:.3}", self.offset);
        self.frames.ended.store(false, Ordering::Relaxed);

        if let Some((w, h)) = self.video_size {
            let child = Command::new(&*exe)
                .args(["-hide_banner", "-loglevel", "error", "-ss", &start, "-re", "-i"])
                .arg(&self.path)
                .args(["-an", "-sn", "-vf", &format!("scale={w}:{h}"), "-f", "rawvideo", "-pix_fmt", "bgra", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            if let Ok(mut child) = child {
                if let Some(stdout) = child.stdout.take() {
                    let frames = self.frames.clone();
                    let stop = stop_flag.clone();
                    std::thread::spawn(move || read_frames(stdout, (w * h * 4) as usize, frames, stop));
                }
                procs.push(child);
            }
        }
        if self.has_audio {
            let child = Command::new(&*exe)
                .args(["-hide_banner", "-loglevel", "error", "-ss", &start, "-i"])
                .arg(&self.path)
                .args(["-vn", "-sn", "-f", "s16le", "-ac", "2", "-ar", "48000", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            if let Ok(mut child) = child {
                if let Some(stdout) = child.stdout.take() {
                    let stop = stop_flag.clone();
                    std::thread::spawn(move || play_pcm(stdout, stop));
                }
                procs.push(child);
            }
        }
        self.started = Some(Instant::now());
    }

    pub fn close(&mut self) {
        match &self.backend {
            Backend::Mci { alias } => {
                let _ = mci(&format!("close {alias}"));
            }
            Backend::Ffmpeg { .. } => self.stop_ffmpeg(),
        }
        self.playing = false;
    }

}

impl Drop for Player {
    fn drop(&mut self) {
        self.close();
    }
}

fn read_frames(mut stdout: ChildStdout, frame_len: usize, slot: Arc<FrameSlot>, stop: Arc<AtomicBool>) {
    let mut buf = vec![0u8; frame_len];
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if stdout.read_exact(&mut buf).is_err() {
            if !stop.load(Ordering::Relaxed) {
                slot.ended.store(true, Ordering::Relaxed);
            }
            return;
        }
        *slot.frame.lock().unwrap() = Some(buf.clone());
        slot.seq.fetch_add(1, Ordering::Relaxed);
    }
}

/// Play 48 kHz stereo s16le PCM from `stdout` until it ends or `stop` is set.
/// At most a few 100 ms buffers are queued, so the pipe throttles FFmpeg.
fn play_pcm(mut stdout: ChildStdout, stop: Arc<AtomicBool>) {
    const RATE: u32 = 48_000;
    const BLOCK: usize = 4; // 2 channels × 16 bit
    const BUF_BYTES: usize = RATE as usize * BLOCK / 10;
    const BUFFERS: usize = 4;

    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 2,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * BLOCK as u32,
        nBlockAlign: BLOCK as u16,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let mut device = HWAVEOUT::default();
    let opened = unsafe { waveOutOpen(Some(&mut device), WAVE_MAPPER, &format, None, None, CALLBACK_NULL) };
    if opened != 0 {
        return;
    }
    let mut data: Vec<Vec<u8>> = (0..BUFFERS).map(|_| vec![0u8; BUF_BYTES]).collect();
    let mut headers: Box<[WAVEHDR; BUFFERS]> = Box::new(std::array::from_fn(|_| WAVEHDR::default()));
    let mut in_use = [false; BUFFERS];
    let header_size = std::mem::size_of::<WAVEHDR>() as u32;

    let mut slot = 0;
    'outer: loop {
        // Wait for this slot's previous buffer to finish playing.
        while in_use[slot] && done_flag(&headers[slot]) == 0 {
            if stop.load(Ordering::Relaxed) {
                break 'outer;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if in_use[slot] {
            unsafe { waveOutUnprepareHeader(device, &mut headers[slot], header_size) };
            in_use[slot] = false;
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }
        // Fill the buffer (the pipe may deliver it in pieces).
        let mut filled = 0;
        while filled < BUF_BYTES {
            match stdout.read(&mut data[slot][filled..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => filled += n,
            }
        }
        filled -= filled % BLOCK;
        if filled == 0 {
            break;
        }
        headers[slot] = WAVEHDR {
            lpData: PSTR(data[slot].as_mut_ptr()),
            dwBufferLength: filled as u32,
            ..Default::default()
        };
        unsafe {
            waveOutPrepareHeader(device, &mut headers[slot], header_size);
            waveOutWrite(device, &mut headers[slot], header_size);
        }
        in_use[slot] = true;
        slot = (slot + 1) % BUFFERS;
    }
    unsafe {
        if stop.load(Ordering::Relaxed) {
            waveOutReset(device);
        } else {
            // Let the tail play out.
            for i in 0..BUFFERS {
                while in_use[i] && done_flag(&headers[i]) == 0 && !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            waveOutReset(device);
        }
        for i in 0..BUFFERS {
            if in_use[i] {
                waveOutUnprepareHeader(device, &mut headers[i], header_size);
            }
        }
        waveOutClose(device);
    }
}

/// The driver sets WHDR_DONE from another thread. WAVEHDR is packed, so read
/// the field through an unaligned raw pointer (fresh on every call).
fn done_flag(header: &WAVEHDR) -> u32 {
    unsafe { std::ptr::addr_of!(header.dwFlags).read_unaligned() & WHDR_DONE }
}

pub fn format_time(seconds: f64) -> String {
    let s = seconds.max(0.) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn ffmpeg_needed(ext: &str) -> bool {
    !MCI_AUDIO.contains(&ext)
}
