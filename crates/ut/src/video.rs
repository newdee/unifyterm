//! `ut view VIDEO`: a video played in the terminal. ffmpeg decodes it to
//! raw RGB frames on a pipe; each frame is drawn in place, at the video's
//! own pace (30 frames a second at most), and frames are dropped when
//! drawing falls behind. No sound. `q` or Ctrl-C stops it.
//!
//! In a terminal with kitty's graphics protocol every frame replaces the
//! last under one image number; elsewhere each frame is text (half blocks,
//! braille or ASCII) drawn over the one before.

use std::fmt::Write as _;
use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use image::{DynamicImage, RgbImage};

use crate::view::{Mode, rounded};

/// The file names `ut view` plays instead of showing.
const VIDEO_EXTENSIONS: [&str; 9] = ["mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "flv", "mpg"];
/// The most frames a second it draws.
const MOST_FPS: f64 = 30.0;
/// The image number its frames have in kitty's protocol.
const IMAGE_ID: u32 = 0x00_75_74_76;
/// Pixels a column is decoded to for kitty's protocol, and the most pixels wide.
const KITTY_COLUMN: u32 = 8;
const KITTY_MOST_WIDTH: u32 = 640;
/// A terminal cell is about twice as tall as it is wide.
const CELL_ASPECT: f64 = 2.0;
/// The most of a kitty sequence's data, in bytes (the protocol's limit).
const KITTY_CHUNK: usize = 4096;
/// Frames decoded ahead of the one drawn.
const AHEAD: usize = 2;
/// How long one look for a key waits, at most.
const KEY_LOOK: Duration = Duration::from_millis(5);
const TROUBLE: i32 = 1;

pub(crate) const NO_FFMPEG: &str = "ut view: playing a video needs ffmpeg and ffprobe on PATH.
  Windows: winget install Gyan.FFmpeg
  macOS:   brew install ffmpeg
  Linux:   sudo apt install ffmpeg (or your system's package manager)";

/// Whether `file` is named as a video.
pub(crate) fn is_video(file: &str) -> bool {
    std::path::Path::new(file)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| VIDEO_EXTENSIONS.iter().any(|known| extension.eq_ignore_ascii_case(known)))
}

/// The video's first frame, as PNG bytes.
pub(crate) fn first_frame(file: &str) -> Result<Vec<u8>, String> {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i", file, "-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                NO_FFMPEG.to_owned()
            } else {
                format!("ut view: ffmpeg: {error}")
            }
        })?;
    if output.stdout.is_empty() {
        return Err(format!(
            "ut view: {file}: ffmpeg found no frame: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

/// What ffprobe says of the video's first stream.
#[derive(Debug, PartialEq)]
pub(crate) struct Probe {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: f64,
}

/// ffprobe's `width,height,avg_frame_rate` line, as `64,64,10/1`.
pub(crate) fn parse_probe(line: &str) -> Option<Probe> {
    let mut parts = line.trim().split(',');
    let width = parts.next()?.parse().ok().filter(|width| *width > 0)?;
    let height = parts.next()?.parse().ok().filter(|height| *height > 0)?;
    let rate = parts.next()?;
    let fps = match rate.split_once('/') {
        Some((top, bottom)) => top.parse::<f64>().ok()? / bottom.parse::<f64>().ok().filter(|bottom| *bottom > 0.0)?,
        None => rate.parse().ok()?,
    };
    (fps.is_finite() && fps > 0.0).then_some(Probe { width, height, fps })
}

fn probe(file: &str) -> Result<Probe, String> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,avg_frame_rate"])
        .args(["-of", "csv=p=0", file])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                NO_FFMPEG.to_owned()
            } else {
                format!("ut view: ffprobe: {error}")
            }
        })?;
    let said = String::from_utf8_lossy(&output.stdout);
    let first = said.lines().next().unwrap_or_default();
    parse_probe(first).ok_or_else(|| {
        let why = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        format!(
            "ut view: {file}: no video stream ffprobe can read{}",
            if why.is_empty() { String::new() } else { format!(": {why}") }
        )
    })
}

/// The size, in pixels, frames are decoded to for `mode` at `columns` wide,
/// and how many rows of cells they take. Even sizes: ffmpeg wants them.
pub(crate) fn frame_size(probe: &Probe, mode: Mode, columns: u32) -> ((u32, u32), u32) {
    let even = |value: f64| rounded(value).max(2) & !1;
    let ratio = f64::from(probe.height) / f64::from(probe.width);
    match mode {
        // kitty's protocol scales the picture to the cells it is given.
        Mode::Kitty => {
            let width = probe.width.min(columns * KITTY_COLUMN).min(KITTY_MOST_WIDTH);
            let rows = rounded(f64::from(columns) * ratio / CELL_ASPECT).max(1);
            ((even(f64::from(width)), even(f64::from(width) * ratio)), rows)
        }
        // Text: the rows are the ones the frame's text takes, so that room for exactly them is
        // kept. Braille shows 2 × 4 pixels a cell (its last row may be part filled).
        Mode::Braille => {
            let (width, height) = (even(f64::from(columns * 2)), even(f64::from(columns * 2) * ratio));
            ((width, height), height.div_ceil(4))
        }
        // Half blocks and ASCII: one pixel wide, two tall, a cell.
        _ => {
            let (width, height) = (even(f64::from(columns)), even(f64::from(columns) * ratio));
            ((width, height), height / 2)
        }
    }
}

/// What to do with a frame due at `due` when the next is due at `next`, at `now`.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Step {
    /// Too early: wait this long, then draw it.
    Wait(Duration),
    Draw,
    /// The next frame is due already: this one is skipped.
    Drop,
}

pub(crate) fn step(due: Duration, next: Duration, now: Duration) -> Step {
    if now >= next {
        Step::Drop
    } else if now < due {
        Step::Wait(due.saturating_sub(now))
    } else {
        Step::Draw
    }
}

/// One frame as kitty's protocol: raw RGB under [`IMAGE_ID`], placed on
/// `columns` × `rows` cells at the cursor, which stays where it is.
fn kitty_frame(frame: &RgbImage, columns: u32, rows: u32) -> String {
    let data = base64::engine::general_purpose::STANDARD.encode(frame.as_raw());
    let (width, height) = frame.dimensions();
    let mut out = String::new();
    let pieces: Vec<&[u8]> = data.as_bytes().chunks(KITTY_CHUNK).collect();
    for (index, piece) in pieces.iter().enumerate() {
        let more = u8::from(index + 1 < pieces.len());
        let piece = std::str::from_utf8(piece).unwrap_or_default();
        if index == 0 {
            let _ = write!(
                out,
                "\x1b_Ga=T,f=24,s={width},v={height},i={IMAGE_ID},c={columns},r={rows},C=1,q=2,m={more};{piece}\x1b\\"
            );
        } else {
            let _ = write!(out, "\x1b_Gm={more};{piece}\x1b\\");
        }
    }
    out
}

/// One frame as text, with the cursor back where the frame began.
fn text_frame(frame: RgbImage, mode: Mode, columns: u32) -> String {
    let image = DynamicImage::ImageRgb8(frame);
    let text = match mode {
        Mode::Braille => crate::view::braille(&image, columns),
        Mode::Ascii => crate::view::ascii(&image, columns),
        _ => crate::view::half(&image, columns),
    };
    let lines = text.lines().count();
    let mut out = text.replace('\n', "\r\n");
    if lines > 0 {
        let _ = write!(out, "\x1b[{lines}A");
    }
    out
}

/// The terminal as `ut view` leaves it: keys back to the line editor,
/// the cursor shown, below the video. Done when it goes out of scope.
struct Playing {
    raw: bool,
    rows: u32,
    ffmpeg: Child,
}

impl Drop for Playing {
    fn drop(&mut self) {
        let _ = self.ffmpeg.kill();
        let _ = self.ffmpeg.wait();
        if self.raw {
            let _ = crossterm::terminal::disable_raw_mode();
        }
        let mut out = io::stdout().lock();
        let _ = write!(out, "\x1b[{}B\r\n\x1b[?25h", self.rows);
        let _ = out.flush();
    }
}

/// Whether `q` or Ctrl-C was pressed (raw mode: Ctrl-C is a key, not a signal).
fn stop_asked() -> bool {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    while crossterm::event::poll(Duration::ZERO).unwrap_or(false) {
        if let Ok(Event::Key(key)) = crossterm::event::read()
            && key.kind == KeyEventKind::Press
            && (key.code == KeyCode::Char('q')
                || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)))
        {
            return true;
        }
    }
    false
}

/// What `ut view` says on standard error while it plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Say {
    Nothing,
    /// The video's size and rate, and how it is played (`--verbose`).
    Choices,
}

/// Plays `file` in `mode` (kitty, or a text mode), `columns` wide.
pub(crate) fn play(file: &str, mode: Mode, columns: u32, say: Say) -> i32 {
    let probe = match probe(file) {
        Ok(probe) => probe,
        Err(message) => {
            let _ = writeln!(io::stderr(), "{message}");
            return TROUBLE;
        }
    };
    let mode = match mode {
        Mode::Kitty | Mode::Braille | Mode::Ascii => mode,
        _ => Mode::Half,
    };
    let fps = probe.fps.min(MOST_FPS);
    let ((width, height), rows) = frame_size(&probe, mode, columns);
    if say == Say::Choices {
        let _ = writeln!(
            io::stderr(),
            "ut view: video {}x{} at {:.2} fps, played as {} {width}x{height} at {fps:.2} fps, {columns}x{rows} cells",
            probe.width,
            probe.height,
            probe.fps,
            mode.name()
        );
    }
    let filter = format!("scale={width}:{height},fps={fps}");
    let started = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i", file, "-vf", &filter, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut ffmpeg = match started {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let _ = writeln!(io::stderr(), "{NO_FFMPEG}");
            return TROUBLE;
        }
        Err(error) => {
            let _ = writeln!(io::stderr(), "ut view: ffmpeg: {error}");
            return TROUBLE;
        }
    };
    let Some(mut pipe) = ffmpeg.stdout.take() else { return TROUBLE };
    let frame_bytes = usize::try_from(u64::from(width) * u64::from(height) * 3).unwrap_or(usize::MAX);
    let (send, frames) = mpsc::sync_channel::<Vec<u8>>(AHEAD);
    std::thread::spawn(move || {
        loop {
            let mut frame = vec![0; frame_bytes];
            if pipe.read_exact(&mut frame).is_err() || send.send(frame).is_err() {
                break;
            }
        }
    });
    let raw = io::stdin().is_terminal() && crossterm::terminal::enable_raw_mode().is_ok();
    let playing = Playing { raw, rows, ffmpeg };
    let mut out = io::stdout().lock();
    // Room below for the whole video, so that the screen does not scroll while it plays.
    let _ = write!(out, "\x1b[?25l{}\x1b[{rows}A", "\r\n".repeat(usize::try_from(rows).unwrap_or(0)));
    let start = Instant::now();
    let period = Duration::from_secs_f64(1.0 / fps);
    let mut stopped = false;
    for (number, frame) in frames.iter().enumerate() {
        let number = u32::try_from(number).unwrap_or(u32::MAX);
        let (due, next) = (period * number, period * number.saturating_add(1));
        let mut action = step(due, next, start.elapsed());
        while let Step::Wait(left) = action {
            if playing.raw && stop_asked() {
                stopped = true;
                break;
            }
            std::thread::sleep(left.min(KEY_LOOK));
            action = step(due, next, start.elapsed());
        }
        if stopped || (playing.raw && stop_asked()) {
            break;
        }
        if action == Step::Drop {
            continue;
        }
        let Some(frame) = RgbImage::from_raw(width, height, frame) else { break };
        let text =
            if mode == Mode::Kitty { kitty_frame(&frame, columns, rows) } else { text_frame(frame, mode, columns) };
        if out.write_all(text.as_bytes()).and_then(|()| out.flush()).is_err() {
            break;
        }
    }
    drop(out);
    drop(playing);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn videos_are_told_by_their_names() {
        assert!(is_video("clip.MP4"));
        assert!(is_video("C:/a/b.webm"));
        assert!(!is_video("photo.png"));
        assert!(!is_video("mp4"));
    }

    #[test]
    fn ffprobe_lines_are_read() {
        assert_eq!(parse_probe("64,64,10/1\n"), Some(Probe { width: 64, height: 64, fps: 10.0 }));
        assert_eq!(parse_probe("1920,1080,30000/1001").map(|probe| (probe.fps * 1000.0).round()), Some(29970.0));
        assert_eq!(parse_probe("64,64,0/0"), None, "no rate");
        assert_eq!(parse_probe("0,64,25/1"), None);
        assert_eq!(parse_probe(""), None);
    }

    #[test]
    fn frames_are_sized_for_the_mode() {
        let wide = Probe { width: 1920, height: 1080, fps: 30.0 };
        assert_eq!(frame_size(&wide, Mode::Kitty, 40), ((320, 180), 11));
        assert_eq!(frame_size(&wide, Mode::Kitty, 200), ((640, 360), 56), "640 pixels wide at most");
        assert_eq!(frame_size(&wide, Mode::Half, 40), ((40, 22), 11));
        assert_eq!(frame_size(&wide, Mode::Braille, 40), ((80, 44), 11), "45 rounded down to even");
        // 29 columns of a 10:1 tall picture... and 10 columns of 0.29: braille's rows are its text's.
        for (columns, height) in [(10, 29), (10, 37), (7, 13), (40, 9)] {
            let probe = Probe { width: 100, height, fps: 25.0 };
            for mode in [Mode::Half, Mode::Braille, Mode::Ascii] {
                let ((width, frame_height), rows) = frame_size(&probe, mode, columns);
                let frame = RgbImage::from_pixel(width, frame_height, image::Rgb([90, 90, 90]));
                let lines = text_frame(frame, mode, columns).matches("\r\n").count();
                assert_eq!(u32::try_from(lines).unwrap(), rows, "{mode:?}, {columns} columns, 100x{height}");
            }
        }
        let small = Probe { width: 64, height: 64, fps: 10.0 };
        assert_eq!(frame_size(&small, Mode::Kitty, 20), ((64, 64), 10), "never larger than the video");
    }

    /// Frames due every 100 ms; drawing frame `n` takes `costs[n]` ms. Which are drawn.
    fn played(costs: &[u64]) -> Vec<bool> {
        let period = Duration::from_millis(100);
        let mut now = Duration::ZERO;
        let mut drawn = Vec::new();
        for (number, cost) in costs.iter().enumerate() {
            let number = u32::try_from(number).unwrap();
            let (due, next) = (period * number, period * (number + 1));
            match step(due, next, now) {
                Step::Wait(left) => {
                    now += left + Duration::from_millis(*cost);
                    drawn.push(true);
                }
                Step::Draw => {
                    now += Duration::from_millis(*cost);
                    drawn.push(true);
                }
                Step::Drop => drawn.push(false),
            }
        }
        drawn
    }

    #[test]
    fn frames_keep_the_pace_and_late_ones_are_dropped() {
        assert_eq!(played(&[10, 10, 10, 10]), [true, true, true, true], "fast enough: every frame");
        // Frame 1 takes 250 ms: it ends at 350 ms, when frames 2 (200) and 3 (300) are past due.
        assert_eq!(played(&[10, 250, 10, 10, 10]), [true, true, false, true, true]);
        assert_eq!(played(&[10, 450, 10, 10, 10, 10]), [true, true, false, false, false, true]);
        assert_eq!(
            step(Duration::from_millis(100), Duration::from_millis(200), Duration::from_millis(40)),
            Step::Wait(Duration::from_millis(60))
        );
    }

    #[test]
    fn a_kitty_frame_replaces_the_last_in_place() {
        let frame = RgbImage::from_pixel(4, 2, image::Rgb([1, 2, 3]));
        let text = kitty_frame(&frame, 2, 1);
        assert!(text.starts_with(&format!("\x1b_Ga=T,f=24,s=4,v=2,i={IMAGE_ID},c=2,r=1,C=1,q=2,m=0;")));
        let big = kitty_frame(&RgbImage::new(64, 64), 8, 4);
        assert!(big.split("\x1b\\").filter(|piece| !piece.is_empty()).all(|piece| piece.len() < KITTY_CHUNK + 80));
        assert!(big.ends_with("m=0;") || big.contains("\x1b_Gm=0;"), "the last piece says it is the last");
    }

    #[test]
    fn a_text_frame_goes_back_to_where_it_began() {
        let text = text_frame(RgbImage::from_pixel(4, 4, image::Rgb([200, 0, 0])), Mode::Half, 4);
        assert!(text.ends_with("\x1b[2A"), "{text:?}");
        assert!(!text.contains("\n\x1b") || text.contains("\r\n"));
    }
}
