use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Theme {
    pub accent: Color,
    pub panel_bg: Color,
    pub surface0: Color,
    pub surface1: Color,
    pub surface_dim: Color,
    pub overlay0: Color,
    pub overlay1: Color,
    pub text: Color,
    pub subtext0: Color,
    pub mauve: Color,
    pub green: Color,
    pub yellow: Color,
    pub red: Color,
    pub blue: Color,
    pub teal: Color,
    pub peach: Color,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedTheme {
    pub theme: Theme,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct HerdrConfig {
    theme: HerdrTheme,
    ui: HerdrUi,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct HerdrTheme {
    name: Option<String>,
    custom: Option<ThemeOverrides>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct HerdrUi {
    accent: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThemeOverrides {
    accent: Option<String>,
    panel_bg: Option<String>,
    surface0: Option<String>,
    surface1: Option<String>,
    surface_dim: Option<String>,
    overlay0: Option<String>,
    overlay1: Option<String>,
    text: Option<String>,
    subtext0: Option<String>,
    mauve: Option<String>,
    green: Option<String>,
    yellow: Option<String>,
    red: Option<String>,
    blue: Option<String>,
    teal: Option<String>,
    peach: Option<String>,
}

impl Theme {
    pub(crate) fn catppuccin() -> Self {
        Self {
            accent: Color::Rgb(137, 180, 250),
            panel_bg: Color::Rgb(24, 24, 37),
            surface0: Color::Rgb(49, 50, 68),
            surface1: Color::Rgb(69, 71, 90),
            surface_dim: Color::Rgb(30, 30, 46),
            overlay0: Color::Rgb(108, 112, 134),
            overlay1: Color::Rgb(127, 132, 156),
            text: Color::Rgb(205, 214, 244),
            subtext0: Color::Rgb(166, 173, 200),
            mauve: Color::Rgb(203, 166, 247),
            green: Color::Rgb(166, 227, 161),
            yellow: Color::Rgb(249, 226, 175),
            red: Color::Rgb(243, 139, 168),
            blue: Color::Rgb(137, 180, 250),
            teal: Color::Rgb(148, 226, 213),
            peach: Color::Rgb(250, 179, 135),
        }
    }

    fn terminal() -> Self {
        Self {
            accent: Color::Blue,
            panel_bg: Color::Reset,
            surface0: Color::Reset,
            surface1: Color::DarkGray,
            surface_dim: Color::DarkGray,
            overlay0: Color::Gray,
            overlay1: Color::White,
            text: Color::Reset,
            subtext0: Color::Gray,
            mauve: Color::Magenta,
            green: Color::Green,
            yellow: Color::Yellow,
            red: Color::LightRed,
            blue: Color::Blue,
            teal: Color::Cyan,
            peach: Color::Yellow,
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().replace([' ', '_'], "-").as_str() {
            "catppuccin" | "catppuccin-mocha" => Some(Self::catppuccin()),
            "terminal" => Some(Self::terminal()),
            _ => None,
        }
    }

    pub(crate) fn root_style(self) -> Style {
        Style::default().fg(self.text).bg(self.panel_bg)
    }

    pub(crate) fn panel_style(self) -> Style {
        self.root_style()
    }

    pub(crate) fn text_style(self) -> Style {
        self.root_style()
    }

    pub(crate) fn muted_style(self) -> Style {
        Style::default()
            .fg(self.overlay0)
            .bg(self.panel_bg)
            .add_modifier(Modifier::DIM)
    }

    pub(crate) fn secondary_style(self) -> Style {
        Style::default().fg(self.subtext0).bg(self.panel_bg)
    }

    pub(crate) fn border_style(self, focused: bool) -> Style {
        Style::default()
            .fg(if focused {
                self.accent
            } else {
                self.surface_dim
            })
            .bg(self.panel_bg)
    }

    pub(crate) fn title_style(self, focused: bool) -> Style {
        let style = Style::default()
            .fg(if focused { self.accent } else { self.overlay1 })
            .bg(self.panel_bg);
        if focused {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        }
    }

    pub(crate) fn selected_style(self) -> Style {
        Style::default()
            .fg(contrast_foreground(self.accent, self.panel_bg, self.text))
            .bg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub(crate) fn warning_style(self) -> Style {
        Style::default().fg(self.yellow).bg(self.panel_bg)
    }

    pub(crate) fn error_style(self) -> Style {
        Style::default().fg(self.red).bg(self.panel_bg)
    }

    pub(crate) fn skill_chip_style(self) -> Style {
        let background = [self.surface0, self.surface1, self.surface_dim]
            .into_iter()
            .find(|color| *color != Color::Reset);
        match background {
            Some(background) => Style::default()
                .fg(self.mauve)
                .bg(background)
                .add_modifier(Modifier::BOLD),
            None => Style::default()
                .fg(self.mauve)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        }
    }

    fn apply_overrides(&mut self, overrides: &ThemeOverrides) {
        macro_rules! apply {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(value) = overrides.$field.as_deref() {
                    self.$field = parse_color(value);
                })+
            };
        }
        apply!(
            accent,
            panel_bg,
            surface0,
            surface1,
            surface_dim,
            overlay0,
            overlay1,
            text,
            subtext0,
            mauve,
            green,
            yellow,
            red,
            blue,
            teal,
            peach,
        );
    }
}

pub(crate) fn load_active() -> LoadedTheme {
    load_from_path(&config_path_from(
        std::env::var_os("HERDR_CONFIG_PATH").as_deref(),
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    ))
}

fn config_path_from(
    explicit: Option<&OsStr>,
    xdg: Option<&OsStr>,
    home: Option<&OsStr>,
) -> PathBuf {
    if let Some(path) = explicit {
        return PathBuf::from(path);
    }
    if let Some(root) = xdg {
        return PathBuf::from(root).join("herdr/config.toml");
    }
    if let Some(home) = home {
        return PathBuf::from(home).join(".config/herdr/config.toml");
    }
    PathBuf::from("/tmp/herdr/config.toml")
}

fn load_from_path(path: &Path) -> LoadedTheme {
    let mut loaded = LoadedTheme {
        theme: Theme::catppuccin(),
        diagnostic: None,
    };
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return loaded,
        Err(error) => {
            loaded.diagnostic = Some(format!("Herdr theme: {error}"));
            return loaded;
        }
    };
    let config: HerdrConfig = match toml::from_str(&source) {
        Ok(config) => config,
        Err(error) => {
            loaded.diagnostic = Some(format!("Herdr theme config: {error}"));
            return loaded;
        }
    };
    let requested = config.theme.name.as_deref().unwrap_or("catppuccin");
    loaded.theme = Theme::from_name(requested).unwrap_or_else(|| {
        loaded.diagnostic = Some(format!(
            "Herdr theme {requested:?} is not supported yet; using catppuccin"
        ));
        Theme::catppuccin()
    });
    if let Some(overrides) = &config.theme.custom {
        loaded.theme.apply_overrides(overrides);
    }
    if config
        .theme
        .custom
        .as_ref()
        .and_then(|custom| custom.accent.as_ref())
        .is_none()
    {
        if let Some(accent) = config
            .ui
            .accent
            .as_deref()
            .filter(|accent| *accent != "cyan")
        {
            loaded.theme.accent = parse_color(accent);
        }
    }
    loaded
}

fn parse_color(value: &str) -> Color {
    let value = value.trim().to_lowercase();
    if matches!(value.as_str(), "reset" | "default" | "none" | "transparent") {
        return Color::Reset;
    }
    if let Some(hex) = value.strip_prefix('#') {
        let expanded = if hex.len() == 3 {
            hex.chars().flat_map(|ch| [ch, ch]).collect::<String>()
        } else {
            hex.to_string()
        };
        if expanded.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&expanded[0..2], 16),
                u8::from_str_radix(&expanded[2..4], 16),
                u8::from_str_radix(&expanded[4..6], 16),
            ) {
                return Color::Rgb(r, g, b);
            }
        }
    }
    if let Some(inner) = value
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let values = inner
            .split(',')
            .map(str::trim)
            .map(str::parse::<u8>)
            .collect::<Result<Vec<_>, _>>();
        if let Ok(values) = values {
            if values.len() == 3 {
                return Color::Rgb(values[0], values[1], values[2]);
            }
        }
    }
    match value.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        _ => Color::Cyan,
    }
}

fn contrast_foreground(background: Color, first: Color, second: Color) -> Color {
    let Some(background_luma) = luma(background) else {
        return first;
    };
    [first, second, Color::Black, Color::White]
        .into_iter()
        .max_by_key(|color| {
            luma(*color)
                .map(|candidate| (candidate - background_luma).abs())
                .unwrap_or_default()
        })
        .unwrap_or(first)
}

fn luma(color: Color) -> Option<i32> {
    let Color::Rgb(r, g, b) = color else {
        return None;
    };
    Some((299 * i32::from(r) + 587 * i32::from(g) + 114 * i32::from(b)) / 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_theme_uses_herdr_catppuccin_default() {
        let config: HerdrConfig = toml::from_str("").unwrap();
        assert!(config.theme.name.is_none());
        assert_eq!(Theme::catppuccin().accent, Color::Rgb(137, 180, 250));
    }

    #[test]
    fn custom_colors_parse() {
        assert_eq!(parse_color("#cba6f7"), Color::Rgb(203, 166, 247));
        assert_eq!(parse_color("rgb(24, 24, 37)"), Color::Rgb(24, 24, 37));
        assert_eq!(parse_color("transparent"), Color::Reset);
    }

    #[test]
    fn config_path_prefers_explicit_then_xdg() {
        assert_eq!(
            config_path_from(Some(OsStr::new("/a")), Some(OsStr::new("/b")), None),
            PathBuf::from("/a")
        );
        assert_eq!(
            config_path_from(None, Some(OsStr::new("/b")), None),
            PathBuf::from("/b/herdr/config.toml")
        );
    }
}
