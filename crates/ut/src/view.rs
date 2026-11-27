//! `ut view FILE`: a picture in the terminal, in the best way the terminal
//! can show one.
//!
//! | mode      | how                                   | chosen when                                   |
//! |-----------|---------------------------------------|-----------------------------------------------|
//! | `kitty`   | kitty's graphics protocol             | UnifyTerm, kitty, `WezTerm`, ghostty, a keepane pane with them on (`KEEPANE_GRAPHICS=kitty`) |
//! | `iterm`   | iTerm2's inline images                | iTerm2                                        |
//! | `sixel`   | sixel graphics                        | Windows Terminal                              |
//! | `half`    | `▀` with two true colors a cell       | the terminal says it has true color           |
//! | `braille` | braille dots, 2 × 4 a cell            | a UTF-8 terminal                              |
//! | `ascii`   | ` .:-=+*#%@`                          | anything else, and output that is no terminal |
//!
//! Which terminal it is comes from the environment it sets (`TERM_PROGRAM`
//! and the like): nothing is asked of the terminal and nothing read back.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, IsTerminal as _, Write as _};

use base64::Engine as _;
use image::{DynamicImage, imageops::FilterType};

const USAGE: &str = "usage: ut view [--mode MODE] [--width COLUMNS] [--verbose] FILE

  Shows a picture (PNG, JPEG, GIF, WebP, BMP) in the terminal, or plays a
  video (MP4, MKV, WebM, MOV, AVI, ...) with ffmpeg: no sound; q or Ctrl-C stops it.
  Output that is no terminal gets a video's first frame as a picture.
  --mode     kitty, iterm, sixel, half, braille or ascii (default: the best this terminal has)
  --width    how many columns wide (default: the terminal's width, at most 80)
  --verbose  say on standard error which mode was chosen and why";

const USAGE_ERROR: i32 = 2;
const TROUBLE: i32 = 1;
/// Columns when the output is no terminal, or the terminal will not say.
const DEFAULT_WIDTH: u16 = 80;
/// The most of a kitty sequence's data, in bytes (the protocol's limit).
const KITTY_CHUNK: usize = 4096;
/// Pixels a column is taken to be in sixel mode, whose size is in pixels.
const SIXEL_COLUMN: u32 = 10;
/// From dark to light, for the ASCII mode.
const RAMP: &[u8] = b" .:-=+*#%@";

/// How a picture is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Kitty,
    Iterm,
    Sixel,
    Half,
    Braille,
    Ascii,
}

impl Mode {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "kitty" => Self::Kitty,
            "iterm" => Self::Iterm,
            "sixel" => Self::Sixel,
            "half" => Self::Half,
            "braille" => Self::Braille,
            "ascii" => Self::Ascii,
            _ => return None,
        })
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Kitty => "kitty",
            Self::Iterm => "iterm",
            Self::Sixel => "sixel",
            Self::Half => "half",
            Self::Braille => "braille",
            Self::Ascii => "ascii",
        }
    }
}

/// Where the output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Output {
    Terminal,
    Other,
}

/// The best mode for a terminal, from its environment (`get` reads a variable), and why.
pub(crate) fn choose(get: impl Fn(&str) -> Option<String>, output: Output) -> (Mode, String) {
    if output == Output::Other {
        return (Mode::Ascii, "the output is no terminal".into());
    }
    let program = get("TERM_PROGRAM").unwrap_or_default();
    let term = get("TERM").unwrap_or_default();
    if matches!(program.as_str(), "unifyterm" | "WezTerm" | "ghostty")
        || get("KITTY_WINDOW_ID").is_some()
        || term == "xterm-kitty"
    {
        return (
            Mode::Kitty,
            format!("the terminal ({}) has kitty's graphics", if program.is_empty() { "kitty" } else { &program }),
        );
    }
    // A keepane pane (0.99 on) keeps kitty's pictures while its server has
    // them on, and says so; it shows them in a terminal that can, a label in
    // one that cannot. With them off it says nothing: half blocks, below.
    if program == "keepane" && get("KEEPANE_GRAPHICS").as_deref() == Some("kitty") {
        return (Mode::Kitty, "a keepane pane, which keeps kitty's pictures: KEEPANE_GRAPHICS=kitty".into());
    }
    if program == "iTerm.app" {
        return (Mode::Iterm, "the terminal is iTerm2".into());
    }
    if get("WT_SESSION").is_some() {
        return (Mode::Sixel, "the terminal is Windows Terminal, which has sixel".into());
    }
    if term == "dumb" || get("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        return (Mode::Ascii, "the terminal wants no color".into());
    }
    if get("COLORTERM").is_some_and(|value| value == "truecolor" || value == "24bit") {
        return (Mode::Half, "the terminal has true color".into());
    }
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"].iter().find_map(|name| get(name).filter(|value| !value.is_empty()));
    if cfg!(windows) || locale.is_some_and(|value| value.to_ascii_uppercase().contains("UTF")) {
        return (Mode::Braille, "the terminal takes UTF-8".into());
    }
    (Mode::Ascii, "nothing better is known of the terminal".into())
}

/// A picture made smaller (never larger) to `width` pixels across, keeping its shape;
/// `height_scale`: how much taller a pixel of the result is than wide.
fn fit(image: &DynamicImage, width: u32, height_scale: f64) -> image::RgbaImage {
    let width = width.min(image.width()).max(1);
    let height = f64::from(image.height()) * f64::from(width) / f64::from(image.width()) / height_scale;
    let height = rounded(height).max(1);
    image.resize_exact(width, height, FilterType::Triangle).to_rgba8()
}

#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a pixel count: small and positive"
)]
pub(crate) fn rounded(value: f64) -> u32 {
    value.round().max(0.0) as u32
}

/// A pixel's brightness, 0 to 255 (Rec. 601 weights, in whole numbers: the same on every machine).
fn brightness(pixel: image::Rgba<u8>) -> u32 {
    let [red, green, blue, alpha] = pixel.0;
    (299 * u32::from(red) + 587 * u32::from(green) + 114 * u32::from(blue)) / 1000 * u32::from(alpha) / 255
}

/// The ASCII picture: a character a cell, a cell twice as tall as wide.
pub(crate) fn ascii(image: &DynamicImage, columns: u32) -> String {
    let small = fit(image, columns, 2.0);
    let mut out = String::new();
    for y in 0..small.height() {
        for x in 0..small.width() {
            let level = brightness(*small.get_pixel(x, y)) * u32::try_from(RAMP.len() - 1).unwrap_or(9) / 255;
            out.push(char::from(RAMP[usize::try_from(level).unwrap_or(0)]));
        }
        out.push('\n');
    }
    out
}

/// The half-block picture: in each cell the upper pixel is the text color, the lower the background.
pub(crate) fn half(image: &DynamicImage, columns: u32) -> String {
    let small = fit(image, columns, 1.0);
    let mut out = String::new();
    for y in (0..small.height()).step_by(2) {
        for x in 0..small.width() {
            let [r, g, b, _] = small.get_pixel(x, y).0;
            if y + 1 < small.height() {
                let [r2, g2, b2, _] = small.get_pixel(x, y + 1).0;
                let _ = write!(out, "\x1b[38;2;{r};{g};{b};48;2;{r2};{g2};{b2}m▀");
            } else {
                let _ = write!(out, "\x1b[49;38;2;{r};{g};{b}m▀");
            }
        }
        out.push_str("\x1b[0m\n");
    }
    out
}

/// The braille picture: 2 × 4 dots a cell, a dot lit where the picture is
/// brighter than its average.
pub(crate) fn braille(image: &DynamicImage, columns: u32) -> String {
    // The bit of each dot, by (column, row) in the cell: Unicode's order.
    const BITS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    let small = fit(image, columns * 2, 1.0);
    let (width, height) = small.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    let total: u64 = small.pixels().map(|pixel| u64::from(brightness(*pixel))).sum();
    let average = u32::try_from(total / pixels.max(1)).unwrap_or(128);
    let mut out = String::new();
    for top in (0..height).step_by(4) {
        for left in (0..width).step_by(2) {
            let mut dots = 0;
            for (dx, column) in BITS.iter().enumerate() {
                for (dy, bit) in column.iter().enumerate() {
                    let (x, y) = (left + u32::try_from(dx).unwrap_or(0), top + u32::try_from(dy).unwrap_or(0));
                    if x < width && y < height && brightness(*small.get_pixel(x, y)) > average {
                        dots |= bit;
                    }
                }
            }
            out.push(char::from_u32(0x2800 + dots).unwrap_or(' '));
        }
        out.push('\n');
    }
    out
}

/// The picture as PNG bytes: the file itself when it is one.
fn png(image: &DynamicImage, file: &[u8]) -> Vec<u8> {
    if file.starts_with(b"\x89PNG") {
        return file.to_vec();
    }
    let mut bytes = Vec::new();
    // Writing to memory does not fail; an empty picture would show nothing, which is what there is.
    let _ = image.write_to(&mut io::Cursor::new(&mut bytes), image::ImageFormat::Png);
    bytes
}

/// kitty's graphics protocol: the PNG in pieces of at most 4096 bytes, placed `columns` wide.
pub(crate) fn kitty(image: &DynamicImage, file: &[u8], columns: u32) -> String {
    let data = base64::engine::general_purpose::STANDARD.encode(png(image, file));
    let pieces: Vec<&str> =
        data.as_bytes().chunks(KITTY_CHUNK).filter_map(|piece| std::str::from_utf8(piece).ok()).collect();
    let mut out = String::new();
    for (index, piece) in pieces.iter().enumerate() {
        let more = u8::from(index + 1 < pieces.len());
        if index == 0 {
            let _ = write!(out, "\x1b_Ga=T,f=100,q=2,c={columns},m={more};{piece}\x1b\\");
        } else {
            let _ = write!(out, "\x1b_Gm={more};{piece}\x1b\\");
        }
    }
    out.push('\n');
    out
}

/// iTerm2's inline image, `columns` wide.
pub(crate) fn iterm(image: &DynamicImage, file: &[u8], columns: u32) -> String {
    let data = base64::engine::general_purpose::STANDARD.encode(png(image, file));
    format!("\x1b]1337;File=inline=1;width={columns};preserveAspectRatio=1:{data}\x07\n")
}

/// Sixel graphics: 216 colors (6 levels of red, green and blue), six rows of pixels a band.
pub(crate) fn sixel(image: &DynamicImage, columns: u32) -> String {
    let small = fit(image, columns * SIXEL_COLUMN, 1.0);
    let (width, height) = small.dimensions();
    let level = |value: u8| u32::from(value) * 5 / 255;
    let index = |pixel: image::Rgba<u8>| level(pixel.0[0]) * 36 + level(pixel.0[1]) * 6 + level(pixel.0[2]);
    let mut out = String::from("\x1bPq");
    let _ = write!(out, "\"1;1;{width};{height}");
    for color in 0..216u32 {
        let percent = |part: u32| part * 100 / 5;
        let _ = write!(out, "#{color};2;{};{};{}", percent(color / 36), percent(color / 6 % 6), percent(color % 6));
    }
    for band in (0..height).step_by(6) {
        let mut used: Vec<u32> = Vec::new();
        for y in band..(band + 6).min(height) {
            for x in 0..width {
                let color = index(*small.get_pixel(x, y));
                if !used.contains(&color) {
                    used.push(color);
                }
            }
        }
        used.sort_unstable();
        for (n, color) in used.iter().enumerate() {
            let _ = write!(out, "#{color}");
            for x in 0..width {
                let mut bits = 0u8;
                for dy in 0..6 {
                    let y = band + dy;
                    if y < height && index(*small.get_pixel(x, y)) == *color {
                        bits |= 1 << dy;
                    }
                }
                out.push(char::from(63 + bits));
            }
            // `$` goes back to the band's start for the next color; `-` to the next band.
            out.push(if n + 1 < used.len() { '$' } else { '-' });
        }
    }
    out.push_str("\x1b\\\n");
    out
}

/// The terminal's width in columns, if the output is one.
fn terminal_width() -> Option<u16> {
    terminal_size::terminal_size().map(|(terminal_size::Width(width), _)| width)
}

pub(crate) fn main(args: &[OsString]) -> i32 {
    let mut words = args.iter().map(|arg| arg.to_string_lossy().into_owned());
    let (mut mode, mut width, mut verbose, mut file) = (None, None, false, None);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--help" | "-h" => {
                let _ = writeln!(io::stdout(), "{USAGE}");
                return 0;
            }
            "--mode" => match words.next().as_deref().and_then(Mode::named) {
                Some(named) => mode = Some(named),
                None => return usage("--mode is one of kitty, iterm, sixel, half, braille, ascii"),
            },
            "--width" => match words.next().and_then(|value| value.parse::<u32>().ok()).filter(|value| *value > 0) {
                Some(columns) => width = Some(columns),
                None => return usage("--width is a number of columns"),
            },
            "--verbose" => verbose = true,
            other if other.starts_with("--") => return usage(&format!("unknown option '{other}'")),
            _ if file.is_none() => file = Some(word),
            _ => return usage(&format!("unexpected argument '{word}'")),
        }
    }
    let Some(file) = file else { return usage("which file?") };
    let output = if io::stdout().is_terminal() { Output::Terminal } else { Output::Other };
    if crate::video::is_video(&file) && output == Output::Terminal {
        let (chosen, why) = choose(|name| std::env::var(name).ok(), output);
        let mode = mode.unwrap_or(chosen);
        if verbose && mode == chosen {
            let _ = writeln!(io::stderr(), "ut view: {} ({why})", mode.name());
        }
        let columns = width.unwrap_or_else(|| u32::from(terminal_width().unwrap_or(DEFAULT_WIDTH).min(DEFAULT_WIDTH)));
        let say = if verbose { crate::video::Say::Choices } else { crate::video::Say::Nothing };
        return crate::video::play(&file, mode, columns, say);
    }
    let read = if crate::video::is_video(&file) {
        crate::video::first_frame(&file)
    } else {
        std::fs::read(&file).map_err(|error| format!("ut view: {file}: {error}"))
    };
    let bytes = match read {
        Ok(bytes) => bytes,
        Err(message) => {
            let _ = writeln!(io::stderr(), "{message}");
            return TROUBLE;
        }
    };
    let image = match image::load_from_memory(&bytes) {
        Ok(image) => image,
        Err(error) => {
            let _ = writeln!(io::stderr(), "ut view: {file}: not a picture this can read: {error}");
            return TROUBLE;
        }
    };
    let (chosen, why) = choose(|name| std::env::var(name).ok(), output);
    let (mode, why) = match mode {
        Some(asked) => (asked, "asked for with --mode".to_owned()),
        None => (chosen, why),
    };
    let width = width.unwrap_or_else(|| {
        let columns =
            if output == Output::Terminal { terminal_width().unwrap_or(DEFAULT_WIDTH) } else { DEFAULT_WIDTH };
        u32::from(columns.min(DEFAULT_WIDTH))
    });
    if verbose {
        let _ = writeln!(io::stderr(), "ut view: {} ({why}), {width} columns", mode.name());
    }
    let text = match mode {
        Mode::Kitty => kitty(&image, &bytes, width),
        Mode::Iterm => iterm(&image, &bytes, width),
        Mode::Sixel => sixel(&image, width),
        Mode::Half => half(&image, width),
        Mode::Braille => braille(&image, width),
        Mode::Ascii => ascii(&image, width),
    };
    match io::stdout().lock().write_all(text.as_bytes()) {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            let _ = writeln!(io::stderr(), "ut view: {error}");
            TROUBLE
        }
        _ => 0,
    }
}

fn usage(problem: &str) -> i32 {
    let _ = writeln!(io::stderr(), "ut view: {problem}\n{USAGE}");
    USAGE_ERROR
}

/// The test picture: 64 × 64, four colored quarters and a white diagonal.
#[cfg(test)]
pub(crate) fn fixture() -> DynamicImage {
    DynamicImage::ImageRgba8(image::RgbaImage::from_fn(64, 64, |x, y| {
        if x == y || x + 1 == y {
            return image::Rgba([255, 255, 255, 255]);
        }
        match (x < 32, y < 32) {
            (true, true) => image::Rgba([220, 40, 40, 255]),
            (false, true) => image::Rgba([40, 200, 60, 255]),
            (true, false) => image::Rgba([40, 80, 220, 255]),
            (false, false) => image::Rgba([230, 210, 40, 255]),
        }
    }))
}

#[cfg(test)]
mod tests {
    use image::GenericImageView as _;

    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
    }

    #[test]
    fn the_mode_follows_the_terminal() {
        let mode = |pairs: &[(&str, &str)]| choose(env(pairs), Output::Terminal).0;
        assert_eq!(mode(&[("TERM_PROGRAM", "unifyterm")]), Mode::Kitty);
        assert_eq!(mode(&[("KITTY_WINDOW_ID", "1")]), Mode::Kitty);
        assert_eq!(mode(&[("TERM_PROGRAM", "iTerm.app")]), Mode::Iterm);
        assert_eq!(mode(&[("WT_SESSION", "x")]), Mode::Sixel);
        assert_eq!(mode(&[("TERM", "dumb"), ("COLORTERM", "truecolor")]), Mode::Ascii);
        assert_eq!(mode(&[("COLORTERM", "truecolor")]), Mode::Half);
        // A keepane pane: kitty's pictures while its server keeps them; half
        // blocks (its true color) with them off, as before.
        let pane = [("TERM_PROGRAM", "keepane"), ("COLORTERM", "truecolor"), ("KEEPANE_GRAPHICS", "kitty")];
        assert_eq!(mode(&pane), Mode::Kitty);
        assert_eq!(mode(&pane[..2]), Mode::Half, "kitty-graphics off: nothing said");
        assert_ne!(
            mode(&[("TERM_PROGRAM", "other"), ("KEEPANE_GRAPHICS", "kitty")]),
            Mode::Kitty,
            "only a keepane pane is taken at its word"
        );
        if !cfg!(windows) {
            assert_eq!(mode(&[("LANG", "en_US.UTF-8")]), Mode::Braille);
            assert_eq!(mode(&[("LANG", "C")]), Mode::Ascii);
        }
        assert_eq!(choose(env(&[("TERM_PROGRAM", "unifyterm")]), Output::Other).0, Mode::Ascii, "a pipe gets text");
    }

    #[test]
    fn text_pictures_keep_the_shape() {
        let image = fixture();
        let art = ascii(&image, 40);
        assert_eq!(art.lines().count(), 20, "64 × 64 at 40 columns: 20 rows of cells twice as tall");
        assert!(art.lines().all(|line| line.len() == 40));
        let dots = braille(&image, 20);
        assert_eq!(dots.lines().count(), 10, "40 × 40 dots: 10 rows of 4");
        assert!(
            dots.lines()
                .all(|line| line.chars().count() == 20 && line.chars().all(|c| ('\u{2800}'..='\u{28FF}').contains(&c)))
        );
        let blocks = half(&image, 16);
        assert_eq!(blocks.lines().count(), 8, "16 × 16 pixels, two a cell");
        assert_eq!(blocks.matches('▀').count(), 16 * 8);
        // Away from the diagonal the quarters keep their colors (the top left cell is on the diagonal: mixed).
        for color in ["220;40;40", "40;200;60", "40;80;220", "230;210;40"] {
            assert!(blocks.contains(&format!("38;2;{color};48;2;{color}m")), "{color} is there");
        }
        assert!(ascii(&image, 400).lines().all(|line| line.len() == 64), "never wider than the picture");
    }

    #[test]
    fn kitty_sends_pieces_of_at_most_4096_bytes() {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(200, 200, |x, y| {
            image::Rgba([
                u8::try_from(x % 256).unwrap(),
                u8::try_from(y % 256).unwrap(),
                u8::try_from((x ^ y) % 256).unwrap(),
                255,
            ])
        }));
        let text = kitty(&image, b"not a png", 30);
        let pieces: Vec<&str> = text.split("\x1b\\").filter(|piece| piece.starts_with("\x1b_G")).collect();
        assert!(pieces.len() > 1, "a large picture goes in pieces");
        assert!(pieces[0].starts_with("\x1b_Ga=T,f=100,q=2,c=30,m=1;"));
        assert!(pieces.last().unwrap().starts_with("\x1b_Gm=0;"));
        for piece in &pieces {
            let data = piece.split_once(';').unwrap().1;
            assert!(data.len() <= KITTY_CHUNK, "{}", data.len());
        }
        let data: String = pieces.iter().map(|piece| piece.split_once(';').unwrap().1).collect();
        let png = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
        assert_eq!(
            image::load_from_memory(&png).unwrap().dimensions(),
            (200, 200),
            "the pieces make the picture again"
        );
    }

    #[test]
    fn sixel_and_iterm_are_well_formed() {
        let image = fixture();
        let six = sixel(&image, 4);
        assert!(six.starts_with("\x1bPq\"1;1;40;40#0;2;0;0;0") && six.ends_with("\x1b\\\n"));
        let bands = six.matches('-').count();
        assert_eq!(bands, 7, "40 rows of pixels: 7 bands of 6");
        assert!(six.chars().filter(|c| ('?'..='~').contains(c)).count() > 0);
        let inline = iterm(&image, b"\x89PNG fake", 20);
        assert!(
            inline.starts_with("\x1b]1337;File=inline=1;width=20;preserveAspectRatio=1:") && inline.ends_with("\x07\n")
        );
    }
}
