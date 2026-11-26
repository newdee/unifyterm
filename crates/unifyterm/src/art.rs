//! The picture at the foot of the sidebar: the one the look in use names.
//!
//! A theme (or the settings) names a picture's file. UnifyTerm brings 13
//! pictures of its own: one for each of its themes with a preview page
//! (`demo/appearance.html`), and one for each theme with a character
//! (`whale`, `clay`, `mint`, `twin`, `void`, `lyre`, `prism`); they are in
//! the program, and a theme names one the same way it names a file of the
//! user's own.
//!
//! A character's picture is a drawing of an original character, made for
//! UnifyTerm with an image model and cut to the same 960 by 496 pixels, the
//! character in the middle and low, where the sidebar's strip shows it.
//!
//! The other six are what the preview page draws, made by that page's own
//! drawing code at twice the page's size (960 by 496 pixels) and saved as
//! PNG, which loses nothing. The sidebar shows a picture 268 pixels wide: it
//! is made smaller on every screen up to 276 % scaling. Drawn again here
//! with GPUI's own shapes they could only be like the page's: the page uses
//! a radial gradient, a glow and a gradient for every column of pixels,
//! which GPUI's two-color gradients do not give.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use gpui::{Image, ImageFormat};

use crate::config::Picture;

macro_rules! own {
    ($name:literal) => {
        ($name, include_bytes!(concat!("../../../demo/themes/", $name)).as_slice())
    };
}

/// UnifyTerm's own pictures by their files' names: the ones `unifyterm-config` lists.
const OWN: [(&str, &[u8]); 13] = [
    own!("aurora.png"),
    own!("eclipse.png"),
    own!("phosphor.png"),
    own!("riso.png"),
    own!("night.png"),
    own!("day.png"),
    own!("whale.png"),
    own!("clay.png"),
    own!("mint.png"),
    own!("twin.png"),
    own!("void.png"),
    own!("lyre.png"),
    own!("prism.png"),
];

/// A picture's file is not read past this: a picture for a strip 268 pixels wide is far below it.
const MOST_BYTES: u64 = 16 * 1024 * 1024;

/// One of UnifyTerm's own pictures, by its file's name.
fn own(name: &str) -> Option<Arc<Image>> {
    // Made once: an image is known to GPUI by a number worked out from all its bytes.
    static MADE: OnceLock<Vec<(&'static str, Arc<Image>)>> = OnceLock::new();
    MADE.get_or_init(|| {
        OWN.iter().map(|(name, bytes)| (*name, Arc::new(Image::from_bytes(ImageFormat::Png, bytes.to_vec())))).collect()
    })
    .iter()
    .find(|(known, _)| *known == name)
    .map(|(_, picture)| Arc::clone(picture))
}

/// What kind of picture a file is, by its name's ending.
fn format_of(file: &Path) -> Option<ImageFormat> {
    let ending = file.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ending.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "webp" => ImageFormat::Webp,
        "gif" => ImageFormat::Gif,
        "bmp" => ImageFormat::Bmp,
        "svg" => ImageFormat::Svg,
        "tif" | "tiff" => ImageFormat::Tiff,
        _ => return None,
    })
}

/// A file's picture. Why not, when it cannot be had.
fn read(file: &Path) -> Result<Arc<Image>, String> {
    let format = format_of(file).ok_or("its name does not end in png, jpg, webp, gif, bmp, svg or tif")?;
    let size = std::fs::metadata(file).map_err(|error| error.to_string())?.len();
    if size > MOST_BYTES {
        return Err(format!("it is {size} bytes: more than {MOST_BYTES}"));
    }
    let bytes = std::fs::read(file).map_err(|error| error.to_string())?;
    Ok(Arc::new(Image::from_bytes(format, bytes)))
}

/// The picture in use, kept from one look to the next: a file is read again
/// only when it is another file, or was written since.
#[derive(Default)]
pub(crate) struct Shown {
    last: Option<LastRead>,
}

/// The file last read.
struct LastRead {
    file: PathBuf,
    /// What told the file's state then: when it was written, and its size. `None`: it was not there.
    stamp: Option<(SystemTime, u64)>,
    /// Its picture; `None` when it could not be had.
    picture: Option<Arc<Image>>,
}

impl Shown {
    /// The picture at `at`; `None` for no picture, and for a file that
    /// cannot be had (the log says why, once for each state of the file).
    pub(crate) fn of(&mut self, at: Option<&Picture>) -> Option<Arc<Image>> {
        match at? {
            Picture::BuiltIn(name) => own(name),
            Picture::File(file) => {
                let stamp = std::fs::metadata(file).ok().and_then(|about| Some((about.modified().ok()?, about.len())));
                if let Some(last) = &self.last
                    && last.file == *file
                    && last.stamp == stamp
                {
                    return last.picture.clone();
                }
                let picture = read(file)
                    .map_err(|why| log::warn!("theme: the picture {} is not shown: {why}", file.display()))
                    .ok();
                self.last = Some(LastRead { file: file.clone(), stamp, picture: picture.clone() });
                picture
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unifyterms_own_pictures_are_the_ones_the_settings_library_lists() {
        let here: Vec<&str> = OWN.iter().map(|(name, _)| *name).collect();
        assert_eq!(here, unifyterm_config::themes::PICTURES, "each name the library knows has its picture here");
        for (name, bytes) in OWN {
            // A PNG: its eight-byte mark, then the first chunk, which gives the width and the height.
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{name}");
            let number = |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            assert_eq!((number(16), number(20)), (960, 496), "{name}: twice the page's 480 by 248");
        }
        let mut shown = Shown::default();
        let first = shown.of(Some(&Picture::BuiltIn("aurora.png"))).expect("there");
        let again = shown.of(Some(&Picture::BuiltIn("aurora.png"))).expect("there");
        assert!(Arc::ptr_eq(&first, &again), "made once, not for every look");
        assert!(shown.of(None).is_none(), "no picture named");
    }

    #[test]
    fn a_file_is_read_once_until_it_changes_and_a_wrong_one_shows_nothing() {
        let folder = tempfile::tempdir().expect("a folder");
        let file = folder.path().join("mine.PNG");
        std::fs::write(&file, OWN[0].1).expect("write");
        let at = Picture::File(file.clone());
        let mut shown = Shown::default();
        let first = shown.of(Some(&at)).expect("read");
        assert!(Arc::ptr_eq(&first, &shown.of(Some(&at)).expect("kept")), "the same file, not read again");
        // Another size: another state of the file, whatever the clock says.
        std::fs::write(&file, OWN[5].1).expect("write");
        let second = shown.of(Some(&at)).expect("read again");
        assert!(!Arc::ptr_eq(&first, &second), "a changed file is read again");

        assert!(shown.of(Some(&Picture::File(folder.path().join("not-there.png")))).is_none());
        let text = folder.path().join("notes.txt");
        std::fs::write(&text, "x").expect("write");
        assert!(shown.of(Some(&Picture::File(text))).is_none(), "not a picture's name");
        assert_eq!(format_of(Path::new("a/b.JPeG")), Some(ImageFormat::Jpeg));
        assert_eq!(format_of(Path::new("no-ending")), None);
    }
}
