//! The prompt UnifyTerm writes for a shell (`[prompt]`): the line left of
//! the cursor, put together from segments (the directory, git, how the last
//! command ended, ...) in one of four styles, in the theme's colors.
//!
//! The settings file and a theme file have the same table. Every key but
//! `enabled` may be left out, and three layers are put together: the
//! settings' own over the theme's over UnifyTerm's defaults ([`PromptConfig::over`],
//! then [`PromptConfig::resolved`]). `enabled` is the settings file's alone:
//! a theme does not replace the prompt a user has without being asked.
//!
//! A color is one of the theme's 16 by its name, or `#rrggbb`: a prompt
//! that names the theme's colors changes with the theme.

use std::time::Duration;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// How the segments are drawn. The names are Powerlevel10k's.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Style {
    /// Colored words, nothing behind them.
    #[default]
    Lean,
    /// Every segment on one band of the theme's bar color, thin lines between them.
    Classic,
    /// Every segment on a color of its own, arrows between them.
    Rainbow,
    /// ASCII only: the same in any terminal and any font.
    Plain,
}

/// The shape between two segments of `classic` and `rainbow`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Separator {
    #[default]
    Sharp,
    Round,
    Slant,
}

/// How the colors change along the prompt.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Gradient {
    /// Each segment in its own color.
    #[default]
    Off,
    /// Each segment from its own color to the next segment's.
    Segments,
    /// One set of colors (`colors`) over the whole side.
    Whole,
}

/// The light that crosses the newest prompt. The terminal draws it, over the
/// cells that have something in them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Shine {
    #[default]
    Off,
    /// Once, when a new prompt is there.
    Once,
    /// Every `shine_every` seconds, while the prompt waits.
    Loop,
}

/// What a segment shows.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Segment {
    /// The directory: the home as `~`, `/` between the parts.
    Dir,
    /// The git branch, and the changes when they are counted in time.
    Git,
    /// The last command's exit code, when it is not 0.
    Status,
    /// How long the last command took, when it was 2 seconds or more.
    Duration,
    /// The Python virtual environment (or conda's) in use.
    Venv,
    /// The time the prompt was written.
    Time,
}

impl Segment {
    /// Every segment, in the order they are listed.
    pub const ALL: [Self; 6] = [Self::Dir, Self::Git, Self::Status, Self::Duration, Self::Venv, Self::Time];

    /// Its name in the file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Dir => "dir",
            Self::Git => "git",
            Self::Status => "status",
            Self::Duration => "duration",
            Self::Venv => "venv",
            Self::Time => "time",
        }
    }

    /// The segment called `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|segment| segment.name() == name)
    }
}

/// One of the theme's 16 colors, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ansi(u8);

impl Ansi {
    /// The names, in the order of `theme.ansi`.
    pub const NAMES: [&'static str; 16] = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "bright-black",
        "bright-red",
        "bright-green",
        "bright-yellow",
        "bright-blue",
        "bright-magenta",
        "bright-cyan",
        "bright-white",
    ];
    pub const RED: Self = Self(1);
    pub const GREEN: Self = Self(2);
    pub const YELLOW: Self = Self(3);
    pub const BLUE: Self = Self(4);
    pub const MAGENTA: Self = Self(5);
    pub const CYAN: Self = Self(6);
    pub const BRIGHT_BLACK: Self = Self(8);

    /// The color called `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::NAMES.iter().position(|known| *known == name).and_then(|at| u8::try_from(at).ok()).map(Self)
    }

    /// Where it is in `theme.ansi`: 0 to 15.
    #[must_use]
    pub fn index(self) -> usize {
        usize::from(self.0)
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        Self::NAMES[self.index()]
    }
}

/// A color the prompt is written in: one of the theme's, or a color of its own.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(try_from = "String", into = "String")]
pub enum Paint {
    Ansi(Ansi),
    Rgb([u8; 3]),
}

impl Paint {
    /// What it is with these 16 colors (`theme.ansi`).
    #[must_use]
    pub fn rgb(self, ansi: &[[u8; 3]; 16]) -> [u8; 3] {
        match self {
            Self::Ansi(color) => ansi[color.index()],
            Self::Rgb(rgb) => rgb,
        }
    }
}

impl TryFrom<String> for Paint {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if let Some(color) = Ansi::named(&text) {
            return Ok(Self::Ansi(color));
        }
        if text.starts_with('#')
            && let Some(rgb) = crate::parse_color(&text)
        {
            return Ok(Self::Rgb(rgb));
        }
        Err(format!("\"{text}\" is not a color: one of the theme's (black, red, ..., bright-white) or #rrggbb"))
    }
}

impl From<Paint> for String {
    fn from(paint: Paint) -> Self {
        match paint {
            Paint::Ansi(color) => color.name().to_owned(),
            Paint::Rgb([red, green, blue]) => format!("#{red:02x}{green:02x}{blue:02x}"),
        }
    }
}

/// `prompt.lines`: the prompt on one line, or on two (the segments on the
/// first, the input on the second).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lines {
    One,
    Two,
}

/// How many colors a segment's color is: one, or two for a gradient of its own.
pub const SEGMENT_COLORS: std::ops::RangeInclusive<usize> = 1..=2;
/// How many colors `colors` (the whole side's gradient) is.
pub const WHOLE_COLORS: std::ops::RangeInclusive<usize> = 2..=8;
/// `shine_every`, in seconds.
pub const SHINE_EVERY: std::ops::RangeInclusive<f32> = 1.0..=600.0;
/// `lines`.
pub const LINES: std::ops::RangeInclusive<u8> = 1..=2;

/// `[prompt]` as one layer gives it: the settings file, or a theme file.
/// Left out: the layer under it says.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PromptConfig {
    /// UnifyTerm writes the prompt in new tabs. The settings file's alone.
    pub enabled: bool,
    pub style: Option<Style>,
    pub separator: Option<Separator>,
    pub lines: Option<u8>,
    pub left: Option<Vec<Segment>>,
    /// At the end of the first line when the prompt has two; at the line's
    /// end in zsh and fish when it has one. PowerShell and bash have no
    /// place for them on one line.
    pub right: Option<Vec<Segment>>,
    pub gradient: Option<Gradient>,
    /// The whole side's colors, left to right, for `gradient = "whole"`.
    pub colors: Option<Vec<Paint>>,
    pub dir_color: Option<Vec<Paint>>,
    pub git_color: Option<Vec<Paint>>,
    /// Git's segment when there are changes.
    pub git_dirty_color: Option<Vec<Paint>>,
    pub status_color: Option<Vec<Paint>>,
    pub duration_color: Option<Vec<Paint>>,
    pub venv_color: Option<Vec<Paint>>,
    pub time_color: Option<Vec<Paint>>,
    pub shine: Option<Shine>,
    /// Seconds between two lights, for `shine = "loop"`.
    pub shine_every: Option<f32>,
}

/// Which file a `[prompt]` is in: what it may say depends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Settings,
    Theme,
}

/// Each segment's colors, put together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentPaints {
    pub dir: Vec<Paint>,
    pub git: Vec<Paint>,
    pub git_dirty: Vec<Paint>,
    pub status: Vec<Paint>,
    pub duration: Vec<Paint>,
    pub venv: Vec<Paint>,
    pub time: Vec<Paint>,
}

/// Whether the git working tree has changes: git's segment has a color for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Worktree {
    Clean,
    Changed,
}

impl SegmentPaints {
    /// `segment`'s colors; git's as `worktree` is (the others are the same either way).
    #[must_use]
    pub fn of(&self, segment: Segment, worktree: Worktree) -> &[Paint] {
        match segment {
            Segment::Dir => &self.dir,
            Segment::Git if worktree == Worktree::Changed => &self.git_dirty,
            Segment::Git => &self.git,
            Segment::Status => &self.status,
            Segment::Duration => &self.duration,
            Segment::Venv => &self.venv,
            Segment::Time => &self.time,
        }
    }
}

/// The prompt in use: the three layers put together, nothing left to choose.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub enabled: bool,
    pub style: Style,
    pub separator: Separator,
    pub lines: Lines,
    pub left: Vec<Segment>,
    pub right: Vec<Segment>,
    pub gradient: Gradient,
    pub colors: Vec<Paint>,
    pub paints: SegmentPaints,
    pub shine: Shine,
    pub shine_every: Duration,
}

/// UnifyTerm's own, under every theme.
const DEFAULT_LEFT: [Segment; 2] = [Segment::Dir, Segment::Git];
const DEFAULT_RIGHT: [Segment; 3] = [Segment::Status, Segment::Duration, Segment::Venv];
const DEFAULT_COLORS: [Paint; 3] = [Paint::Ansi(Ansi::CYAN), Paint::Ansi(Ansi::BLUE), Paint::Ansi(Ansi::MAGENTA)];
const DEFAULT_SHINE_EVERY: f32 = 4.0;

impl PromptConfig {
    /// Every key the table has, as the file writes it. A theme file's key
    /// that is not one of these is passed over (see [`crate::themes::prompt`]).
    pub const KEYS: [&'static str; 17] = [
        "enabled",
        "style",
        "separator",
        "lines",
        "left",
        "right",
        "gradient",
        "colors",
        "dir_color",
        "git_color",
        "git_dirty_color",
        "status_color",
        "duration_color",
        "venv_color",
        "time_color",
        "shine",
        "shine_every",
    ];

    /// The colors of each segment, by their keys' names, in the file's order.
    fn paints(&self) -> [(&'static str, &Option<Vec<Paint>>); 7] {
        [
            ("dir_color", &self.dir_color),
            ("git_color", &self.git_color),
            ("git_dirty_color", &self.git_dirty_color),
            ("status_color", &self.status_color),
            ("duration_color", &self.duration_color),
            ("venv_color", &self.venv_color),
            ("time_color", &self.time_color),
        ]
    }

    /// What the table says may be said, in the file `layer` it is in.
    pub fn check(&self, layer: Layer) -> Result<()> {
        if layer == Layer::Theme && self.enabled {
            bail!("prompt.enabled is for the settings file: a theme does not turn the prompt on");
        }
        if let Some(lines) = self.lines
            && !LINES.contains(&lines)
        {
            bail!("prompt.lines is {lines}: it must be 1 or 2");
        }
        if let Some(every) = self.shine_every
            && !SHINE_EVERY.contains(&every)
        {
            bail!(
                "prompt.shine_every is {every}: it must be from {} to {} seconds",
                SHINE_EVERY.start(),
                SHINE_EVERY.end()
            );
        }
        if let Some(colors) = &self.colors
            && !WHOLE_COLORS.contains(&colors.len())
        {
            let (least, most) = (WHOLE_COLORS.start(), WHOLE_COLORS.end());
            bail!("prompt.colors has {} colors: it must have from {least} to {most}", colors.len());
        }
        for (key, paints) in self.paints() {
            if let Some(paints) = paints
                && !SEGMENT_COLORS.contains(&paints.len())
            {
                bail!("prompt.{key} has {} colors: it has 1, or 2 for a gradient of its own", paints.len());
            }
        }
        let mut seen: Vec<Segment> = Vec::new();
        for (side, segments) in [("left", &self.left), ("right", &self.right)] {
            for segment in segments.iter().flatten() {
                if seen.contains(segment) {
                    bail!("prompt.{side} has {}, which is there already: a segment is shown once", segment.name());
                }
                seen.push(*segment);
            }
        }
        Ok(())
    }

    /// This layer's values, with `under`'s where this one leaves one out.
    /// `enabled` is this layer's: only the settings file has it.
    #[must_use]
    pub fn over(self, under: Self) -> Self {
        Self {
            enabled: self.enabled,
            style: self.style.or(under.style),
            separator: self.separator.or(under.separator),
            lines: self.lines.or(under.lines),
            left: self.left.or(under.left),
            right: self.right.or(under.right),
            gradient: self.gradient.or(under.gradient),
            colors: self.colors.or(under.colors),
            dir_color: self.dir_color.or(under.dir_color),
            git_color: self.git_color.or(under.git_color),
            git_dirty_color: self.git_dirty_color.or(under.git_dirty_color),
            status_color: self.status_color.or(under.status_color),
            duration_color: self.duration_color.or(under.duration_color),
            venv_color: self.venv_color.or(under.venv_color),
            time_color: self.time_color.or(under.time_color),
            shine: self.shine.or(under.shine),
            shine_every: self.shine_every.or(under.shine_every),
        }
    }

    /// The prompt in use, with UnifyTerm's defaults for what no layer gave.
    /// A segment the right side names is not on the left as well, whichever
    /// layer each side came from: the left keeps it.
    #[must_use]
    pub fn resolved(&self) -> Prompt {
        let one = |paint: Ansi| vec![Paint::Ansi(paint)];
        let left = self.left.clone().unwrap_or_else(|| DEFAULT_LEFT.to_vec());
        let right: Vec<Segment> = self
            .right
            .clone()
            .unwrap_or_else(|| DEFAULT_RIGHT.to_vec())
            .into_iter()
            .filter(|segment| !left.contains(segment))
            .collect();
        Prompt {
            enabled: self.enabled,
            style: self.style.unwrap_or_default(),
            separator: self.separator.unwrap_or_default(),
            lines: if self.lines == Some(1) { Lines::One } else { Lines::Two },
            left,
            right,
            gradient: self.gradient.unwrap_or_default(),
            colors: self.colors.clone().unwrap_or_else(|| DEFAULT_COLORS.to_vec()),
            paints: SegmentPaints {
                dir: self.dir_color.clone().unwrap_or_else(|| one(Ansi::BLUE)),
                git: self.git_color.clone().unwrap_or_else(|| one(Ansi::GREEN)),
                git_dirty: self.git_dirty_color.clone().unwrap_or_else(|| one(Ansi::YELLOW)),
                status: self.status_color.clone().unwrap_or_else(|| one(Ansi::RED)),
                duration: self.duration_color.clone().unwrap_or_else(|| one(Ansi::YELLOW)),
                venv: self.venv_color.clone().unwrap_or_else(|| one(Ansi::CYAN)),
                time: self.time_color.clone().unwrap_or_else(|| one(Ansi::BRIGHT_BLACK)),
            },
            shine: self.shine.unwrap_or_default(),
            shine_every: Duration::from_secs_f32(self.shine_every.unwrap_or(DEFAULT_SHINE_EVERY)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<PromptConfig, toml::de::Error> {
        toml::from_str(text)
    }

    #[test]
    fn nothing_given_is_unifyterms_own_prompt_and_it_is_off() {
        let prompt = PromptConfig::default().resolved();
        assert!(!prompt.enabled, "the prompt a user has stays until asked");
        assert_eq!((prompt.style, prompt.separator, prompt.lines), (Style::Lean, Separator::Sharp, Lines::Two));
        assert_eq!((prompt.left.as_slice(), prompt.right.as_slice()), (&DEFAULT_LEFT[..], &DEFAULT_RIGHT[..]));
        assert_eq!((prompt.gradient, prompt.shine), (Gradient::Off, Shine::Off));
        assert_eq!(prompt.shine_every, Duration::from_secs(4));
        assert_eq!(prompt.paints.of(Segment::Git, Worktree::Clean), [Paint::Ansi(Ansi::GREEN)]);
        assert_eq!(prompt.paints.of(Segment::Git, Worktree::Changed), [Paint::Ansi(Ansi::YELLOW)], "with changes");
        assert_eq!(prompt.paints.of(Segment::Time, Worktree::Clean), [Paint::Ansi(Ansi::BRIGHT_BLACK)]);
    }

    #[test]
    fn the_file_is_read_as_a_person_writes_it() {
        let given = read(
            r##"
            enabled = true
            style = "rainbow"
            separator = "round"
            lines = 1
            left = ["dir", "git"]
            right = ["status", "time"]
            gradient = "whole"
            colors = ["cyan", "#7aa2f7", "bright-magenta"]
            dir_color = ["blue", "magenta"]
            shine = "loop"
            shine_every = 2.5
            "##,
        )
        .expect("a [prompt]");
        given.check(Layer::Settings).expect("fine");
        let prompt = given.resolved();
        assert!(prompt.enabled);
        assert_eq!((prompt.style, prompt.separator, prompt.lines), (Style::Rainbow, Separator::Round, Lines::One));
        assert_eq!(prompt.right, [Segment::Status, Segment::Time]);
        assert_eq!(
            prompt.colors,
            [
                Paint::Ansi(Ansi::CYAN),
                Paint::Rgb([0x7a, 0xa2, 0xf7]),
                Paint::Ansi(Ansi::named("bright-magenta").expect("a name"))
            ]
        );
        assert_eq!(prompt.paints.dir, [Paint::Ansi(Ansi::BLUE), Paint::Ansi(Ansi::MAGENTA)]);
        assert_eq!((prompt.shine, prompt.shine_every), (Shine::Loop, Duration::from_millis(2500)));
    }

    #[test]
    fn a_color_is_a_name_of_the_themes_sixteen_or_rrggbb_and_is_written_back_the_same() {
        for name in Ansi::NAMES {
            let paint = Paint::try_from(name.to_owned()).expect(name);
            assert_eq!(String::from(paint), name);
        }
        assert_eq!(String::from(Paint::try_from("#7AA2F7".to_owned()).expect("a color")), "#7aa2f7");
        for wrong in ["", "Red", "purple", "7aa2f7", "#7aa2f", "bright-", "ansi4"] {
            assert!(Paint::try_from(wrong.to_owned()).is_err(), "{wrong}");
        }
        let ansi: [[u8; 3]; 16] = std::array::from_fn(|at| [u8::try_from(at).expect("small"); 3]);
        assert_eq!(Paint::Ansi(Ansi::CYAN).rgb(&ansi), [6, 6, 6], "the theme's sixth");
        assert_eq!(Paint::Rgb([1, 2, 3]).rgb(&ansi), [1, 2, 3]);
        assert!(read("colors = [\"cyan\", \"purple\"]").is_err(), "the file names the wrong one");
    }

    #[test]
    fn what_cannot_be_is_refused_with_the_key_named() {
        let said = |text: &str, layer: Layer| {
            let given = read(text).expect("TOML of the right shape");
            format!("{:#}", given.check(layer).expect_err("refused"))
        };
        assert!(said("enabled = true", Layer::Theme).contains("prompt.enabled is for the settings file"));
        assert!(said("lines = 3", Layer::Settings).contains("prompt.lines is 3"));
        assert!(said("lines = 0", Layer::Settings).contains("prompt.lines is 0"));
        assert!(said("shine_every = 0.5", Layer::Settings).contains("prompt.shine_every is 0.5"));
        assert!(said("shine_every = 601", Layer::Settings).contains("from 1 to 600"));
        assert!(said("colors = [\"red\"]", Layer::Settings).contains("prompt.colors has 1 colors"));
        assert!(said("dir_color = [\"red\", \"green\", \"blue\"]", Layer::Settings).contains("prompt.dir_color has 3"));
        assert!(said("git_dirty_color = []", Layer::Settings).contains("prompt.git_dirty_color has 0"));
        assert!(said("left = [\"dir\", \"dir\"]", Layer::Settings).contains("prompt.left has dir, which is there"));
        assert!(said("left = [\"git\"]\nright = [\"git\"]", Layer::Settings).contains("prompt.right has git"));
        // The shape of the file is serde's to refuse, with the key named.
        for wrong in ["style = \"fancy\"", "left = [\"weather\"]", "shine = \"always\"", "lines = \"two\"", "sise = 1"]
        {
            assert!(read(wrong).is_err(), "{wrong}");
        }
        for fine in ["left = []", "right = []", "lines = 1", "shine_every = 1", "shine_every = 600"] {
            read(fine).expect(fine).check(Layer::Settings).expect(fine);
        }
    }

    #[test]
    fn the_settings_go_over_the_theme_and_the_theme_over_the_defaults() {
        let theme = read("style = \"rainbow\"\ngradient = \"segments\"\nright = [\"time\"]").expect("a theme's");
        theme.check(Layer::Theme).expect("fine");
        let settings = read("enabled = true\nstyle = \"plain\"").expect("the settings'");
        let prompt = settings.over(theme.clone()).resolved();
        assert!(prompt.enabled, "the settings' own");
        assert_eq!(prompt.style, Style::Plain, "the settings' own over the theme's");
        assert_eq!(
            (prompt.gradient, prompt.right.as_slice()),
            (Gradient::Segments, &[Segment::Time][..]),
            "the theme's"
        );
        assert_eq!(prompt.separator, Separator::Sharp, "neither: the default");
        assert!(!PromptConfig::default().over(theme).resolved().enabled, "a theme never turns it on");
    }

    #[test]
    fn a_segment_on_both_sides_from_two_layers_stays_on_the_left() {
        // The settings move git to the right; the theme's left still has it. Each layer alone is fine.
        let theme = read("left = [\"dir\", \"git\"]").expect("a theme's");
        let settings = read("right = [\"git\", \"time\"]").expect("the settings'");
        let prompt = settings.over(theme).resolved();
        assert_eq!((prompt.left.as_slice(), prompt.right.as_slice()), (&DEFAULT_LEFT[..], &[Segment::Time][..]));
    }

    #[test]
    fn the_tables_keys_are_the_ones_listed() {
        let paint = Some(vec![Paint::Ansi(Ansi::RED)]);
        let all = PromptConfig {
            enabled: true,
            style: Some(Style::Lean),
            separator: Some(Separator::Round),
            lines: Some(1),
            left: Some(vec![Segment::Dir]),
            right: Some(vec![Segment::Time]),
            gradient: Some(Gradient::Whole),
            colors: Some(DEFAULT_COLORS.to_vec()),
            dir_color: paint.clone(),
            git_color: paint.clone(),
            git_dirty_color: paint.clone(),
            status_color: paint.clone(),
            duration_color: paint.clone(),
            venv_color: paint.clone(),
            time_color: paint,
            shine: Some(Shine::Once),
            shine_every: Some(4.0),
        };
        let written = toml::Value::try_from(&all).expect("a table");
        let mut keys: Vec<&str> = written.as_table().expect("a table").keys().map(String::as_str).collect();
        let mut listed = PromptConfig::KEYS.to_vec();
        keys.sort_unstable();
        listed.sort_unstable();
        assert_eq!(keys, listed);
        let back: PromptConfig = written.try_into().expect("read back");
        assert_eq!(back, all, "written and read back, the same");
        let names: Vec<&str> = Segment::ALL.iter().map(|segment| segment.name()).collect();
        assert_eq!(names, ["dir", "git", "status", "duration", "venv", "time"]);
        assert!(Segment::ALL.iter().all(|segment| Segment::named(segment.name()) == Some(*segment)));
    }
}
