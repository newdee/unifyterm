//! Pictures in the terminal: the part of kitty's graphics protocol that
//! sends an image and places it (`ESC _ G <keys> ; <base64> ESC \`).
//!
//! An image is placed by writing Unicode placeholders into the screen: the
//! character U+10EEEE, its foreground color the image's number, a combining
//! mark for the row and one for the column of the image the cell shows.
//! The screen keeps them like any text, so a picture scrolls with the text,
//! goes into the history and is gone when the screen is cleared. A program
//! that writes placeholders itself (a "virtual" placement, `U=1`) is drawn
//! the same way.
//!
//! What is understood:
//!
//! | key | values |
//! |-----|--------|
//! | `a` | `t` send, `T` send and place, `p` place, `d` delete, `q` ask |
//! | `f` | `100` PNG (or any picture format the image library reads), `32` RGBA, `24` RGB |
//! | `t` | `d`: the data is in the sequence |
//! | `m` | `1`: more pieces follow; `0` the last |
//! | `i` | the image's number |
//! | `s`, `v` | width and height of raw RGBA or RGB data |
//! | `c`, `r` | how many columns and rows the image takes |
//! | `C` | `1`: the cursor stays where it was |
//! | `U` | `1`: a virtual placement (the program writes the placeholders) |
//! | `d` | with `a=d`: `a`/`A` every image, `i`/`I` the one numbered `i` |
//! | `q` | `1`: no answer when all went well; `2`: no answer at all |

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};

use base64::Engine as _;

/// The character that stands for a cell of an image.
pub(crate) const PLACEHOLDER: char = '\u{10EEEE}';

/// The marks for row and column numbers, in order: the Nth is N.
static DIACRITICS: LazyLock<Vec<char>> = LazyLock::new(|| {
    include_str!("rowcolumn-diacritics.txt")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .filter_map(|hex| u32::from_str_radix(hex.trim(), 16).ok().and_then(char::from_u32))
        .collect()
});
static NUMBERS: LazyLock<HashMap<char, usize>> =
    LazyLock::new(|| DIACRITICS.iter().enumerate().map(|(number, mark)| (*mark, number)).collect());

/// The mark for `number`, if there is one (there are 297).
pub(crate) fn diacritic(number: usize) -> Option<char> {
    DIACRITICS.get(number).copied()
}

/// The number a mark stands for.
pub(crate) fn number_of(mark: char) -> Option<usize> {
    NUMBERS.get(&mark).copied()
}

/// The most rows or columns an image is placed on: one mark a number.
pub(crate) fn most_cells() -> usize {
    DIACRITICS.len()
}

/// The largest an image may be once decoded, in bytes (RGBA).
const MOST_PER_IMAGE: usize = 64 * 1024 * 1024;
/// The most all images together may take; the ones used least lately go first.
const MOST_IN_ALL: usize = 256 * 1024 * 1024;
/// The most bytes of one image's data received before it is complete.
const MOST_RECEIVED: usize = 96 * 1024 * 1024;

/// A decoded image.
#[derive(Debug, Clone)]
pub(crate) struct Image {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Four bytes a pixel, rows from the top.
    pub(crate) rgba: Arc<[u8]>,
    /// Changes when the image is sent again under the same number.
    pub(crate) generation: u64,
    /// The columns and rows its last placement took.
    pub(crate) cells: Option<(usize, usize)>,
    /// When it was last sent or placed: the least lately used goes first.
    used: u64,
}

/// What the screen is to do after a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Write this answer back to the program.
    Reply(String),
    /// Write placeholders for image `id`, `cells` = (columns, rows), at the cursor.
    Place { id: u32, cells: (usize, usize), cursor: Cursor },
}

/// Where the cursor is after an image is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cursor {
    /// After the image's last cell, as after text.
    Moves,
    /// Where it was (`C=1`).
    Stays,
}

/// One command's keys and its data.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Command {
    keys: HashMap<char, String>,
    data: String,
}

impl Command {
    fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix('G')?;
        let (keys, data) = text.split_once(';').unwrap_or((text, ""));
        let keys = keys
            .split(',')
            .filter_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                let mut chars = key.chars();
                let key = chars.next().filter(|_| chars.next().is_none())?;
                Some((key, value.to_owned()))
            })
            .collect();
        Some(Self { keys, data: data.to_owned() })
    }

    fn get(&self, key: char) -> Option<&str> {
        self.keys.get(&key).map(String::as_str)
    }

    fn number(&self, key: char) -> Option<u32> {
        self.get(key).and_then(|value| value.parse().ok())
    }
}

/// An image whose pieces are still arriving.
#[derive(Debug)]
struct Arriving {
    first: Command,
    data: Vec<u8>,
}

/// The images a screen has, and the protocol's state.
#[derive(Debug, Default)]
pub(crate) struct Graphics {
    images: HashMap<u32, Image>,
    arriving: Option<Arriving>,
    clock: u64,
    generations: u64,
    /// Numbers given to images sent without one.
    next_own: u32,
}

/// What went wrong with a command, as the protocol says it.
fn refusal(code: &str, why: &str) -> String {
    format!("{code}:{why}")
}

impl Graphics {
    /// The image numbered `id`.
    pub(crate) fn image(&self, id: u32) -> Option<&Image> {
        self.images.get(&id)
    }

    /// How many bytes the decoded images take.
    pub(crate) fn bytes(&self) -> usize {
        self.images.values().map(|image| image.rgba.len()).sum()
    }

    /// The numbers of the images there are.
    #[cfg(test)]
    pub(crate) fn numbers(&self) -> Vec<u32> {
        self.images.keys().copied().collect()
    }

    /// Takes one command (the text of the sequence after `ESC _`). `cell` is a cell's size in
    /// pixels, to work out how many cells an image takes when the command does not say.
    pub(crate) fn command(&mut self, text: &str, cell: (u32, u32)) -> Vec<Effect> {
        let Some(command) = Command::parse(text) else { return Vec::new() };
        // A piece of an image that is arriving: only `m` and the data count.
        if let Some(arriving) = self.arriving.as_mut() {
            let more = command.get('m') == Some("1");
            match base64::engine::general_purpose::STANDARD.decode(command.data.trim()) {
                Ok(bytes) if arriving.data.len() + bytes.len() <= MOST_RECEIVED => arriving.data.extend(bytes),
                _ => {
                    let first = self.arriving.take().map(|arriving| arriving.first).unwrap_or_default();
                    return Self::answer(&first, Err(refusal("EINVAL", "bad or too much data")));
                }
            }
            if more {
                return Vec::new();
            }
            let Arriving { first, data } = self.arriving.take().expect("checked above");
            return self.received(&first, &data, cell);
        }
        match command.get('a').unwrap_or("t") {
            "t" | "T" | "q" => {
                if command.get('t').is_some_and(|medium| medium != "d") {
                    return Self::answer(&command, Err(refusal("ENOTSUP", "only data in the sequence (t=d)")));
                }
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(command.data.trim()) else {
                    return Self::answer(&command, Err(refusal("EINVAL", "the data is not base64")));
                };
                if command.get('m') == Some("1") {
                    self.arriving = Some(Arriving { first: command, data: bytes });
                    return Vec::new();
                }
                self.received(&command, &bytes, cell)
            }
            "p" => {
                let Some(id) = command.number('i') else {
                    return Self::answer(&command, Err(refusal("EINVAL", "which image (i)")));
                };
                if !self.images.contains_key(&id) {
                    return Self::answer(&command, Err(refusal("ENOENT", "no such image")));
                }
                self.place(&command, id, cell)
            }
            "d" => {
                match command.get('d').unwrap_or("a") {
                    "a" | "A" => self.images.clear(),
                    "i" | "I" => {
                        if let Some(id) = command.number('i') {
                            self.images.remove(&id);
                        }
                    }
                    _ => {}
                }
                Vec::new()
            }
            _ => Self::answer(&command, Err(refusal("ENOTSUP", "this action is not supported"))),
        }
    }

    /// All of an image's data is here: decode it, keep it (unless only asked), place it.
    fn received(&mut self, command: &Command, data: &[u8], cell: (u32, u32)) -> Vec<Effect> {
        let decoded = decode(command, data);
        if command.get('a') == Some("q") {
            return Self::answer(command, decoded.map(|_| ()));
        }
        let (width, height, rgba) = match decoded {
            Ok(image) => image,
            Err(why) => return Self::answer(command, Err(why)),
        };
        let id = command.number('i').unwrap_or_else(|| {
            // Numbers from the top of the range, where a program's own are unlikely.
            self.next_own += 1;
            0x00FF_0000 + self.next_own
        });
        self.clock += 1;
        self.generations += 1;
        let image =
            Image { width, height, rgba: Arc::from(rgba), generation: self.generations, cells: None, used: self.clock };
        self.images.insert(id, image);
        self.make_room(id);
        let mut effects = Self::answer(command, Ok(()));
        if command.get('a') == Some("T") {
            effects.extend(self.place(command, id, cell));
        }
        effects
    }

    /// Places image `id` as the command says: how many cells, and where the cursor ends up.
    fn place(&mut self, command: &Command, id: u32, cell: (u32, u32)) -> Vec<Effect> {
        self.clock += 1;
        let clock = self.clock;
        let Some(image) = self.images.get_mut(&id) else { return Vec::new() };
        image.used = clock;
        let (cell_width, cell_height) = (f64::from(cell.0.max(1)), f64::from(cell.1.max(1)));
        let (width, height) = (f64::from(image.width), f64::from(image.height));
        let given = |key| command.number(key).and_then(|n| usize::try_from(n).ok()).filter(|n| *n > 0);
        // Whatever is not given keeps the image's shape.
        let (columns, rows) = match (given('c'), given('r')) {
            (Some(columns), Some(rows)) => (columns, rows),
            (Some(columns), None) => (columns, cells_for(to_f64(columns) * cell_width * height / width / cell_height)),
            (None, Some(rows)) => (cells_for(to_f64(rows) * cell_height * width / height / cell_width), rows),
            (None, None) => (cells_for(width / cell_width), cells_for(height / cell_height)),
        };
        let cells = (columns.clamp(1, most_cells()), rows.clamp(1, most_cells()));
        image.cells = Some(cells);
        if command.get('U') == Some("1") {
            return Vec::new(); // The program writes the placeholders.
        }
        let cursor = if command.get('C') == Some("1") { Cursor::Stays } else { Cursor::Moves };
        vec![Effect::Place { id, cells, cursor }]
    }

    /// The answer to a command, as `q` allows; none when the command gave no number.
    fn answer(command: &Command, outcome: Result<(), String>) -> Vec<Effect> {
        let Some(id) = command.get('i') else { return Vec::new() };
        let quiet = command.number('q').unwrap_or(0);
        let text = match outcome {
            Ok(()) if quiet >= 1 => return Vec::new(),
            Ok(()) => "OK".to_owned(),
            Err(_) if quiet >= 2 => return Vec::new(),
            Err(why) => why,
        };
        vec![Effect::Reply(format!("\x1b_Gi={id};{text}\x1b\\"))]
    }

    /// Removes the least lately used images until all fit; never `keep`.
    fn make_room(&mut self, keep: u32) {
        while self.bytes() > MOST_IN_ALL {
            let oldest = self.images.iter().filter(|(id, _)| **id != keep).min_by_key(|(_, image)| image.used);
            let Some((&id, _)) = oldest else { break };
            self.images.remove(&id);
        }
    }
}

/// A count of cells for a size in cells that may have a fraction: rounded up, at least 1.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a float-to-int `as` saturates and maps NaN to 0, and the result is clamped"
)]
fn cells_for(size: f64) -> usize {
    (size.ceil() as usize).max(1)
}

/// A count as a float: cell counts are tiny.
fn to_f64(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

/// An image's pixels from its data: (width, height, RGBA bytes).
fn decode(command: &Command, data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let format = command.number('f').unwrap_or(32);
    let raw = |channels: u32| -> Result<(u32, u32, Vec<u8>), String> {
        let (Some(width), Some(height)) = (command.number('s'), command.number('v')) else {
            return Err(refusal("EINVAL", "raw pixels need their width (s) and height (v)"));
        };
        let expected =
            usize::try_from(u64::from(width) * u64::from(height) * u64::from(channels)).unwrap_or(usize::MAX);
        if expected > MOST_PER_IMAGE || data.len() != expected {
            return Err(refusal("EINVAL", "the data is not width × height pixels, or too large"));
        }
        let rgba = if channels == 4 {
            data.to_vec()
        } else {
            data.as_chunks::<3>().0.iter().flat_map(|[red, green, blue]| [*red, *green, *blue, 255]).collect()
        };
        Ok((width, height, rgba))
    };
    match format {
        32 => raw(4),
        24 => raw(3),
        100 => {
            let image = image::load_from_memory(data).map_err(|error| refusal("EBADPNG", &error.to_string()))?;
            let (width, height) = (image.width(), image.height());
            let size = usize::try_from(u64::from(width) * u64::from(height) * 4).unwrap_or(usize::MAX);
            if size > MOST_PER_IMAGE {
                return Err(refusal("EFBIG", "the image is too large"));
            }
            Ok((width, height, image.into_rgba8().into_raw()))
        }
        _ => Err(refusal("ENOTSUP", "this format is not supported")),
    }
}

/// The placeholder text that shows image `id` on `cells` = (columns, rows)
/// from column `start` (counted from 0): each row in the image's number as
/// its color, one line down and back to that column between rows; the
/// cursor as `cursor` says.
pub(crate) fn placeholders(id: u32, cells: (usize, usize), start: usize, cursor: Cursor) -> String {
    let [_, red, green, blue] = id.to_be_bytes();
    let (columns, rows) = cells;
    let mut text = String::new();
    if cursor == Cursor::Stays {
        text.push_str("\x1b7");
    }
    for row in 0..rows {
        let _ = write!(text, "\x1b[38;2;{red};{green};{blue}m");
        for column in 0..columns {
            text.push(PLACEHOLDER);
            text.extend(diacritic(row));
            text.extend(diacritic(column));
        }
        text.push_str("\x1b[39m");
        if row + 1 < rows {
            // One line down (scrolling at the bottom), then to the column itself: a move back by
            // the columns written would miss by one when the row ends at the screen's last column,
            // where the cursor stays in that column waiting to wrap.
            let _ = write!(text, "\n\x1b[{}G", start + 1);
        }
    }
    if cursor == Cursor::Stays {
        text.push_str("\x1b8");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: (u32, u32) = (10, 20);

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([u8::try_from(x % 256).unwrap(), u8::try_from(y % 256).unwrap(), 7, 255])
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn the_marks_are_kitty_s_297() {
        assert_eq!(most_cells(), 297);
        assert_eq!(diacritic(0), Some('\u{0305}'));
        assert_eq!(diacritic(1), Some('\u{030D}'));
        assert_eq!(diacritic(296), Some('\u{1D244}'));
        assert_eq!(number_of('\u{030E}'), Some(2));
        assert_eq!(number_of('a'), None);
    }

    #[test]
    fn a_png_is_sent_kept_and_placed_keeping_its_shape() {
        let mut graphics = Graphics::default();
        let effects = graphics.command(&format!("Ga=T,f=100,i=7;{}", b64(&png(40, 30))), CELL);
        assert_eq!(
            effects,
            [Effect::Reply("\x1b_Gi=7;OK\x1b\\".into()), Effect::Place { id: 7, cells: (4, 2), cursor: Cursor::Moves }],
            "40×30 pixels on 10×20 cells: 4 columns, 2 rows (1.5 rounded up)"
        );
        let image = graphics.image(7).unwrap();
        assert_eq!((image.width, image.height, image.rgba.len()), (40, 30, 40 * 30 * 4));
        assert_eq!(&image.rgba[..4], &[0, 0, 7, 255]);
        let wider = graphics.command("Ga=p,i=7,c=8,C=1", CELL);
        assert_eq!(
            wider,
            [Effect::Place { id: 7, cells: (8, 3), cursor: Cursor::Stays }],
            "columns given, rows to the shape"
        );
    }

    #[test]
    fn pieces_make_one_image_and_answers_follow_q() {
        let mut graphics = Graphics::default();
        let data = b64(&png(16, 16));
        let (one, two) = data.split_at(data.len() / 2 / 4 * 4);
        assert_eq!(graphics.command(&format!("Ga=T,f=100,i=3,q=1,m=1;{one}"), CELL), []);
        assert_eq!(
            graphics.command(&format!("Gm=0;{two}"), CELL),
            [Effect::Place { id: 3, cells: (2, 1), cursor: Cursor::Moves }]
        );
        assert!(graphics.image(3).is_some(), "q=1: no OK, the image is there");
        let refused = graphics.command("Ga=T,f=100,i=4;bm90IGFuIGltYWdl", CELL);
        assert!(
            matches!(refused.as_slice(), [Effect::Reply(text)] if text.starts_with("\x1b_Gi=4;EBADPNG")),
            "{refused:?}"
        );
        assert_eq!(graphics.command("Ga=T,f=100,i=5,q=2;bm90IGFuIGltYWdl", CELL), [], "q=2: not even errors");
        assert_eq!(graphics.command("Ga=T,f=100;bm90", CELL), [], "no number: no answer");
    }

    #[test]
    fn a_question_is_answered_and_nothing_kept() {
        let mut graphics = Graphics::default();
        let asked = graphics.command(&format!("Ga=q,f=100,i=31;{}", b64(&png(2, 2))), CELL);
        assert_eq!(asked, [Effect::Reply("\x1b_Gi=31;OK\x1b\\".into())]);
        assert!(graphics.numbers().is_empty());
    }

    #[test]
    fn raw_pixels_need_their_size() {
        let mut graphics = Graphics::default();
        let rgb = b64(&[255, 0, 0, 0, 255, 0]);
        assert_eq!(
            graphics.command(&format!("Ga=t,f=24,s=2,v=1,i=9;{rgb}"), CELL),
            [Effect::Reply("\x1b_Gi=9;OK\x1b\\".into())]
        );
        assert_eq!(&*graphics.image(9).unwrap().rgba, &[255, 0, 0, 255, 0, 255, 0, 255]);
        let short = graphics.command(&format!("Ga=t,f=32,s=2,v=2,i=10;{rgb}"), CELL);
        assert!(matches!(short.as_slice(), [Effect::Reply(text)] if text.contains("EINVAL")));
    }

    #[test]
    fn deleting_and_the_memory_limit() {
        let mut graphics = Graphics::default();
        graphics.command(&format!("Ga=t,f=100,i=1;{}", b64(&png(4, 4))), CELL);
        graphics.command(&format!("Ga=t,f=100,i=2;{}", b64(&png(4, 4))), CELL);
        graphics.command("Ga=d,d=i,i=1", CELL);
        assert_eq!(graphics.numbers(), [2]);
        graphics.command("Ga=d", CELL);
        assert!(graphics.numbers().is_empty());
        // Raw images of 48 MiB each: the fifth pushes the total past 256 MiB, the oldest goes.
        let side = 3584_u32; // 3584 × 3584 × 4 = 49 MiB
        let pixels = b64(&vec![0; usize::try_from(side * side * 4).unwrap()]);
        for id in 1..=6 {
            graphics.command(&format!("Ga=t,f=32,s={side},v={side},i={id};{pixels}"), CELL);
            assert!(graphics.bytes() <= MOST_IN_ALL, "after {id}: {} bytes", graphics.bytes());
        }
        let mut kept = graphics.numbers();
        kept.sort_unstable();
        assert_eq!(kept, [2, 3, 4, 5, 6], "the least lately used went");
        let too_big = graphics.command("Ga=t,f=32,s=5000,v=5000,i=99;AAAA", CELL);
        assert!(matches!(too_big.as_slice(), [Effect::Reply(text)] if text.contains("EINVAL")));
    }

    #[test]
    fn placeholders_carry_the_number_row_and_column() {
        let text = placeholders(0x01_0203, (2, 2), 4, Cursor::Moves);
        let rows: Vec<&str> = text.split("\n\x1b[5G").collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].starts_with("\x1b[38;2;1;2;3m"));
        let marks: Vec<char> = rows[1].chars().filter(|c| number_of(*c).is_some()).collect();
        assert_eq!(marks, ['\u{030D}', '\u{0305}', '\u{030D}', '\u{030D}'], "row 1: (1,0) then (1,1)");
        assert!(
            placeholders(1, (1, 1), 0, Cursor::Stays).starts_with("\x1b7")
                && placeholders(1, (1, 1), 0, Cursor::Stays).ends_with("\x1b8")
        );
    }
}
