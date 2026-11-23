//! The settings, and the theme they give. The settings themselves (what
//! each may be, how the file is read) are `unifyterm-config`, which
//! `ut setting` uses too: this adds what only the window needs.

pub(crate) use unifyterm_config::{AiConfig, AskEnter, CommandsConfig, Config, Provider, WindowConfig};

use crate::theme::{Rgb, Theme};

/// The built-in theme with the settings' colors over it. The settings were
/// checked when they were read: a color that is not one cannot be here, and
/// would leave the built-in one in its place.
pub(crate) fn theme(config: &Config) -> Theme {
    let mut theme = Theme::default();
    let named = [
        (&config.theme.foreground, &mut theme.foreground),
        (&config.theme.background, &mut theme.background),
        (&config.theme.cursor, &mut theme.cursor),
        (&config.theme.selection, &mut theme.selection),
    ];
    for (text, slot) in named {
        if let Some(color) = text.as_deref().and_then(Rgb::parse) {
            *slot = color;
        }
    }
    if let Some(ansi) = &config.theme.ansi {
        for (slot, text) in theme.ansi.iter_mut().zip(ansi) {
            if let Some(color) = Rgb::parse(text) {
                *slot = color;
            }
        }
    }
    theme
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_colors_go_over_the_built_in_theme() {
        assert_eq!(theme(&Config::default()), Theme::default(), "nothing set: the built-in theme");
        let config = Config::parse("[theme]\nbackground = \"#000000\"\ncursor = \"#ff8800\"").unwrap();
        let made = theme(&config);
        assert_eq!((made.background, made.cursor), (Rgb(0, 0, 0), Rgb(0xff, 0x88, 0)));
        assert_eq!(made.foreground, Theme::default().foreground, "not set: the built-in one");
        let colors: Vec<String> = (0..16).map(|index| format!("\"#0000{index:02x}\"")).collect();
        let all = Config::parse(&format!("[theme]\nansi = [{}]", colors.join(", "))).unwrap();
        assert_eq!(theme(&all).ansi[15], Rgb(0, 0, 15), "the 16 colors, in order");
        assert_eq!(theme(&all).ansi.len(), unifyterm_config::ANSI_COLORS, "as many as the settings ask for");
    }
}
