//! Colors: the 16 named ones a theme gives, and the 240 computed from them.

/// One color, 8 bits a channel.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// `#rrggbb` (the `#` may be left out).
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        Some(Self(channel(0)?, channel(2)?, channel(4)?))
    }

    /// This color moved `amount` (0 to 1) of the way to `other`.
    pub fn toward(self, other: Self, amount: f32) -> Self {
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
        Self(mix(self.0, other.0), mix(self.1, other.1), mix(self.2, other.2))
    }

    pub fn hex(self) -> u32 {
        u32::from(self.0) << 16 | u32::from(self.1) << 8 | u32::from(self.2)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    /// Black, red, green, yellow, blue, magenta, cyan, white, then the bright eight.
    pub ansi: [Rgb; 16],
}

impl Default for Theme {
    fn default() -> Self {
        let c = |hex: u32| Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
        Self {
            foreground: c(0xc0caf5),
            background: c(0x16161e),
            cursor: c(0xc0caf5),
            selection: c(0x33467c),
            ansi: [
                c(0x15161e), c(0xf7768e), c(0x9ece6a), c(0xe0af68), c(0x7aa2f7), c(0xbb9af7), c(0x7dcfff), c(0xa9b1d6),
                c(0x414868), c(0xf7768e), c(0x9ece6a), c(0xe0af68), c(0x7aa2f7), c(0xbb9af7), c(0x7dcfff), c(0xc0caf5),
            ],
        }
    }
}

/// How many steps each channel of the 6x6x6 color cube has.
const CUBE: u8 = 6;
/// The first color of the cube, and of the 24 greys after it.
const CUBE_START: u8 = 16;
const GREY_START: u8 = 232;

impl Theme {
    /// Color `index` of xterm's 256: the theme's 16, a 6x6x6 cube, 24 greys.
    pub fn indexed(&self, index: u8) -> Rgb {
        if index < CUBE_START {
            return self.ansi[usize::from(index)];
        }
        if index >= GREY_START {
            let level = 8 + 10 * (index - GREY_START);
            return Rgb(level, level, level);
        }
        let cube = index - CUBE_START;
        let level = |step: u8| if step == 0 { 0 } else { 55 + 40 * step };
        Rgb(level(cube / (CUBE * CUBE)), level(cube / CUBE % CUBE), level(cube % CUBE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_parse() {
        assert_eq!(Rgb::parse("#ff8000"), Some(Rgb(255, 128, 0)));
        assert_eq!(Rgb::parse("0a0B0c"), Some(Rgb(10, 11, 12)));
        assert_eq!(Rgb::parse("#fff"), None);
        assert_eq!(Rgb::parse("#gggggg"), None);
        assert_eq!(Rgb::parse("#ffé00"), None, "not ASCII: no slicing inside a character");
    }

    #[test]
    fn the_256_colors_are_xterms() {
        let theme = Theme::default();
        assert_eq!(theme.indexed(1), theme.ansi[1]);
        assert_eq!(theme.indexed(16), Rgb(0, 0, 0));
        assert_eq!(theme.indexed(196), Rgb(255, 0, 0));
        assert_eq!(theme.indexed(21), Rgb(0, 0, 255));
        assert_eq!(theme.indexed(231), Rgb(255, 255, 255));
        assert_eq!(theme.indexed(232), Rgb(8, 8, 8));
        assert_eq!(theme.indexed(255), Rgb(238, 238, 238));
    }

    #[test]
    fn a_color_moves_toward_another() {
        assert_eq!(Rgb(0, 100, 200).toward(Rgb(100, 100, 0), 0.5), Rgb(50, 100, 100));
        assert_eq!(Rgb(10, 20, 30).toward(Rgb(200, 200, 200), 0.0), Rgb(10, 20, 30));
    }
}
