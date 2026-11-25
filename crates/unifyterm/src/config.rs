//! The settings, and the theme they give. The settings themselves (what
//! each may be, how the file is read) are `unifyterm-config`, which
//! `ut setting` uses too: this adds what only the window needs.

use std::path::Path;

use unifyterm_config::ThemeConfig;
pub(crate) use unifyterm_config::themes::Picture;
pub(crate) use unifyterm_config::{
    AiConfig, AskEnter, CommandsConfig, Config, KeysConfig, Provider, TabBar, WindowConfig,
};

use crate::theme::{Rgb, Theme};

/// The theme the settings ask for: the one they name (a file in `themes`,
/// the user's folder, or one UnifyTerm brings), with the colors the settings
/// give themselves over it. An error when the named theme is not there or
/// is wrong: the caller keeps the colors it has.
///
/// With it, where the look's pictures are, when it has them.
pub(crate) fn theme(config: &Config, themes: Option<&Path>) -> anyhow::Result<(Theme, Pictures)> {
    let look = config.look(themes)?;
    Ok((laid(&look.colors), Pictures { picture: look.picture, wallpaper: look.wallpaper }))
}

/// Where a look's two pictures are: the one at the foot of the sidebar, and
/// the one behind the text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Pictures {
    pub(crate) picture: Option<Picture>,
    pub(crate) wallpaper: Option<Picture>,
}

/// The settings' own colors, without the theme they name: for when that
/// theme could not be read as the window opens.
pub(crate) fn own_colors(config: &Config) -> Theme {
    laid(&config.theme)
}

/// The built-in theme with `colors` over it. They were checked when they
/// were read: a color that is not one cannot be here, and would leave the
/// built-in one in its place.
fn laid(colors: &ThemeConfig) -> Theme {
    let mut theme = Theme::default();
    let named = [
        (&colors.foreground, &mut theme.foreground),
        (&colors.background, &mut theme.background),
        (&colors.cursor, &mut theme.cursor),
        (&colors.selection, &mut theme.selection),
    ];
    for (text, slot) in named {
        if let Some(color) = text.as_deref().and_then(Rgb::parse) {
            *slot = color;
        }
    }
    let around = [
        (&colors.bar, &mut theme.around.bar),
        (&colors.panel, &mut theme.around.panel),
        (&colors.well, &mut theme.around.well),
        (&colors.rule, &mut theme.around.rule),
        (&colors.edge, &mut theme.around.edge),
        (&colors.muted, &mut theme.around.muted),
        (&colors.symbol, &mut theme.around.symbol),
    ];
    for (text, slot) in around {
        *slot = text.as_deref().and_then(Rgb::parse);
    }
    if let Some(ansi) = &colors.ansi {
        for (slot, text) in theme.ansi.iter_mut().zip(ansi) {
            if let Some(color) = Rgb::parse(text) {
                *slot = color;
            }
        }
    }
    if let Some(opacity) = colors.opacity {
        theme.opacity = opacity;
    }
    if let Some(blur) = colors.blur {
        theme.blur = blur;
    }
    if let Some(rounding) = colors.rounding {
        theme.rounding = rounding;
    }
    if let Some(shows) = colors.wallpaper_opacity {
        theme.wallpaper_opacity = shows;
    }
    theme.corners = colors.corners;
    theme
}

#[cfg(test)]
mod tests {
    use super::*;

    fn made(settings: &str) -> Theme {
        theme(&Config::parse(settings).expect("settings"), None).expect("a theme").0
    }

    #[test]
    fn the_settings_colors_go_over_the_built_in_theme() {
        assert_eq!(made(""), Theme::default(), "nothing set: the built-in theme");
        let made_of = made("[theme]\nbackground = \"#000000\"\ncursor = \"#ff8800\"");
        assert_eq!((made_of.background, made_of.cursor), (Rgb(0, 0, 0), Rgb(0xff, 0x88, 0)));
        assert_eq!(made_of.foreground, Theme::default().foreground, "not set: the built-in one");
        let colors: Vec<String> = (0..16).map(|index| format!("\"#0000{index:02x}\"")).collect();
        let all = made(&format!("[theme]\nansi = [{}]", colors.join(", ")));
        assert_eq!(all.ansi[15], Rgb(0, 0, 15), "the 16 colors, in order");
        assert_eq!(all.ansi.len(), unifyterm_config::ANSI_COLORS, "as many as the settings ask for");
    }

    #[test]
    fn the_named_theme_is_under_the_settings_colors() {
        assert_eq!(made("[theme]\nname = \"tokyo-night\""), Theme::default(), "the built-in theme by its name");
        let day = made("[theme]\nname = \"day\"");
        assert_eq!((day.background, day.foreground), (Rgb(0xf4, 0xf9, 0xfe), Rgb(0x14, 0x25, 0x3a)));
        assert_eq!((day.bar(), day.muted()), (Rgb(0xe8, 0xef, 0xf6), Rgb(0x41, 0x54, 0x6b)), "the theme's own");
        assert_eq!(day.symbol(), day.ansi[6], "not given: the theme's cyan");
        assert_eq!(made("[theme]\nname = \"eclipse\"").symbol(), Rgb(0xff, 0xa6, 0xa0));
        let changed = made("[theme]\nname = \"day\"\nbackground = \"#ffffff\"\nbar = \"#eeeeee\"");
        assert_eq!((changed.background, changed.bar()), (Rgb(255, 255, 255), Rgb(0xee, 0xee, 0xee)));
        assert_eq!(changed.foreground, day.foreground, "the rest is the theme's");

        let lost = Config::parse("[theme]\nname = \"nosuch\"\ncursor = \"#ff0000\"").expect("settings");
        assert!(theme(&lost, None).is_err(), "a theme that is not there is said, not passed over");
        assert_eq!(own_colors(&lost).cursor, Rgb(255, 0, 0), "the settings' own colors can still be had");
    }

    #[test]
    fn the_users_theme_file_is_read_from_its_folder() {
        let folder = tempfile::tempdir().expect("a folder");
        let text = unifyterm_config::themes::text("day", None).expect("built in").0.replace("#f4f9fe", "#fffff0");
        std::fs::write(folder.path().join("mine.toml"), text).expect("write");
        let settings = Config::parse("[theme]\nname = \"mine\"").expect("settings");
        assert_eq!(theme(&settings, Some(folder.path())).expect("the file").0.background, Rgb(0xff, 0xff, 0xf0));
        assert!(theme(&settings, None).is_err(), "no folder, no such theme");
    }

    #[test]
    fn what_is_not_a_color_comes_from_the_theme_and_the_settings_too() {
        let plain = made("");
        assert_eq!((plain.opacity, plain.blur), (1.0, false), "nothing shows through until it is asked for");
        let glass = made("[theme]\nname = \"night\"\nopacity = 0.7\nblur = true");
        assert_eq!((glass.opacity, glass.blur), (0.7, true));
        let settings = Config::parse("[theme]\nname = \"aurora\"").expect("settings");
        assert_eq!(theme(&settings, None).expect("a theme").1.picture, Some(Picture::BuiltIn("aurora.png")));
        assert_eq!(
            theme(&Config::default(), None).expect("a theme").1,
            Pictures::default(),
            "the default theme has no picture"
        );
        let walled = Config::parse("[theme]\nwallpaper = \"night.png\"\nwallpaper_opacity = 0.6").expect("settings");
        let (theme_of, pictures) = theme(&walled, None).expect("a theme");
        assert_eq!((theme_of.wallpaper_opacity, pictures.wallpaper), (0.6, Some(Picture::BuiltIn("night.png"))));
        assert_eq!(
            (plain.wallpaper_opacity, plain.blur),
            (0.25, false),
            "a wallpaper is faint until the look says otherwise"
        );
        assert_eq!((plain.rounding, plain.panel_rounding(), plain.button_rounding()), (7.0, 8.0, 5.0));
        assert_eq!(plain.corners, None, "the system's own");
        let square = made("[theme]\nrounding = 0\ncorners = \"square\"");
        assert_eq!((square.rounding, square.panel_rounding(), square.button_rounding()), (0.0, 0.0, 0.0));
        assert_eq!(square.corners, Some(unifyterm_config::Corners::Square));
        let phosphor = made("[theme]\nname = \"phosphor\"");
        assert_eq!((phosphor.rounding, phosphor.corners), (2.0, Some(unifyterm_config::Corners::Small)));
    }
}
