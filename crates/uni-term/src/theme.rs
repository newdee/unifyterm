//! Colors: the 16 named ones a theme gives, and the 240 computed from them.

/// One color, 8 bits a channel.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Rgb(pub(crate) u8, pub(crate) u8, pub(crate) u8);

impl Rgb {
    /// `#rrggbb` (the `#` may be left out).
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        Some(Self(channel(0)?, channel(2)?, channel(4)?))
    }

    /// This color moved `amount` (0 to 1) of the way to `other`.
    pub(crate) fn toward(self, other: Self, amount: f32) -> Self {
        let mix = |a: u8, b: u8| channel(f32::from(a) + (f32::from(b) - f32::from(a)) * amount);
        Self(mix(self.0, other.0), mix(self.1, other.1), mix(self.2, other.2))
    }

    /// The color as `0xRRGGBB`.
    pub(crate) fn hex(self) -> u32 {
        u32::from(self.0) << 16 | u32::from(self.1) << 8 | u32::from(self.2)
    }
}

/// A computed channel value as a channel: rounded, and held to 0..=255.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to a u8's range first, and float-to-int `as` maps NaN to 0"
)]
fn channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Theme {
    pub(crate) foreground: Rgb,
    pub(crate) background: Rgb,
    pub(crate) cursor: Rgb,
    pub(crate) selection: Rgb,
    /// Black, red, green, yellow, blue, magenta, cyan, white, then the bright eight.
    pub(crate) ansi: [Rgb; 16],
}

impl Default for Theme {
    /// The built-in theme (Tokyo Night's colors).
    fn default() -> Self {
        let red = Rgb(0xf7, 0x76, 0x8e);
        let green = Rgb(0x9e, 0xce, 0x6a);
        let yellow = Rgb(0xe0, 0xaf, 0x68);
        let blue = Rgb(0x7a, 0xa2, 0xf7);
        let magenta = Rgb(0xbb, 0x9a, 0xf7);
        let cyan = Rgb(0x7d, 0xcf, 0xff);
        let foreground = Rgb(0xc0, 0xca, 0xf5);
        let (black, white, bright_black) = (Rgb(0x15, 0x16, 0x1e), Rgb(0xa9, 0xb1, 0xd6), Rgb(0x41, 0x48, 0x68));
        Self {
            foreground,
            background: Rgb(0x16, 0x16, 0x1e),
            cursor: foreground,
            selection: Rgb(0x33, 0x46, 0x7c),
            // The bright colors are the plain ones again, but for black and white.
            ansi: [
                black,
                red,
                green,
                yellow,
                blue,
                magenta,
                cyan,
                white,
                bright_black,
                red,
                green,
                yellow,
                blue,
                magenta,
                cyan,
                foreground,
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
    pub(crate) fn indexed(&self, index: u8) -> Rgb {
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
        assert_eq!(Rgb(0xc0, 0xca, 0xf5).hex(), 0x00c0_caf5);
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
    fn the_built_in_theme_is_the_one_it_always_was() {
        let theme = Theme::default();
        let hexes: Vec<u32> = theme.ansi.iter().map(|color| color.hex()).collect();
        let plain =
            [0x0015_161e, 0x00f7_768e, 0x009e_ce6a, 0x00e0_af68, 0x007a_a2f7, 0x00bb_9af7, 0x007d_cfff, 0x00a9_b1d6];
        let bright =
            [0x0041_4868, 0x00f7_768e, 0x009e_ce6a, 0x00e0_af68, 0x007a_a2f7, 0x00bb_9af7, 0x007d_cfff, 0x00c0_caf5];
        assert_eq!(hexes, [plain, bright].concat());
        let named = [theme.foreground, theme.background, theme.cursor, theme.selection].map(Rgb::hex);
        assert_eq!(named, [0x00c0_caf5, 0x0016_161e, 0x00c0_caf5, 0x0033_467c]);
    }

    #[test]
    fn a_color_moves_toward_another() {
        assert_eq!(Rgb(0, 100, 200).toward(Rgb(100, 100, 0), 0.5), Rgb(50, 100, 100));
        assert_eq!(Rgb(10, 20, 30).toward(Rgb(200, 200, 200), 0.0), Rgb(10, 20, 30));
        assert_eq!(Rgb(10, 20, 30).toward(Rgb(200, 100, 0), 1.0), Rgb(200, 100, 0));
        assert_eq!(Rgb(250, 5, 0).toward(Rgb(255, 0, 0), 9.0), Rgb(255, 0, 0), "past the end stays a color");
    }
}
