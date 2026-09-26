//! Colour themes: every colour the UI uses, by role. No IO.
//!
//! Themes keep the terminal's own background and foreground (never painted)
//! and use its 16-colour palette where that reads well, so they follow the
//! user's terminal scheme. `Dark` suits dark terminal backgrounds (Leeroy's
//! original look), `Light` light ones. The `auto` setting picks one from the
//! background colour the terminal reports at startup.

use ratatui::style::{Color, Style};

/// Background brightness of the terminal, as detected at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

/// The `ui.theme` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Follow the terminal background (dark when it can't be detected).
    Auto,
    Dark,
    Light,
}

impl ThemeChoice {
    /// Setting values, in the order Enter cycles through them.
    pub const VALUES: [&'static str; 3] = ["auto", "dark", "light"];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(ThemeChoice::Auto),
            "dark" => Some(ThemeChoice::Dark),
            "light" => Some(ThemeChoice::Light),
            _ => None,
        }
    }

    /// The theme to use, given what the terminal reported (`None`: nothing).
    pub fn resolve(self, detected: Option<Appearance>) -> &'static Theme {
        match (self, detected) {
            (ThemeChoice::Light, _) | (ThemeChoice::Auto, Some(Appearance::Light)) => &LIGHT,
            _ => &DARK,
        }
    }
}

/// Colours by role. Styles that set a background also set a foreground, so
/// their text stays readable whatever the terminal's default colours are.
#[derive(Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    /// Header bar across the full width.
    pub bar: Style,
    /// Secondary text on the header bar (Jenkins version, user).
    pub bar_dim: Color,
    /// Key hint on the header bar (`h/?`).
    pub bar_key: Style,
    /// Active tab, frame lines, section titles.
    pub accent: Color,
    /// Active tab label and context-bar hotkeys: text on `accent`.
    pub on_accent: Style,
    /// Inactive tab labels.
    pub tab_inactive: Color,
    /// Context bar (last line).
    pub context_bar: Style,
    /// Background of the selected list row.
    pub selected_bg: Color,
    /// Replaces text colours equal to `selected_bg` on the selected row.
    pub selected_fg_on_bg: Color,
    /// Secondary text: hints, labels, not-built jobs.
    pub dim: Color,
    pub success: Color,
    /// Unstable builds, running builds, progress.
    pub warning: Color,
    pub error: Color,
    /// Aborted builds.
    pub neutral: Color,
    /// Text being typed, applied filters, commit ids, the selected setting.
    pub highlight: Color,
    /// The text cursor in input fields.
    pub cursor: Style,
    /// Loud warnings (`⚠ TLS NOT VERIFIED`).
    pub danger: Style,
    /// Refresh status block while auto-refresh is on / off.
    pub refresh_on: Style,
    pub refresh_off: Style,
}

/// For dark terminal backgrounds: Leeroy's original colours.
pub const DARK: Theme = Theme {
    name: "dark",
    bar: Style::new().fg(Color::Black).bg(Color::White),
    bar_dim: Color::DarkGray,
    bar_key: Style::new().fg(Color::White).bg(Color::Black),
    accent: Color::Blue,
    on_accent: Style::new().fg(Color::Black).bg(Color::Blue),
    tab_inactive: Color::Gray,
    context_bar: Style::new().fg(Color::White).bg(Color::DarkGray),
    selected_bg: Color::DarkGray,
    selected_fg_on_bg: Color::Gray,
    dim: Color::DarkGray,
    success: Color::Green,
    warning: Color::Yellow,
    error: Color::Red,
    neutral: Color::Gray,
    highlight: Color::Yellow,
    cursor: Style::new().fg(Color::Black).bg(Color::Yellow),
    danger: Style::new().fg(Color::White).bg(Color::Red),
    refresh_on: Style::new().fg(Color::Black).bg(Color::Green),
    refresh_off: Style::new().fg(Color::White).bg(Color::DarkGray),
};

/// Dark goldenrod (256-colour palette): ANSI yellow is unreadable on white in
/// many schemes.
const DARK_YELLOW: Color = Color::Indexed(136);
/// Darker than ANSI green, which is faint on white in many schemes.
const DARK_GREEN: Color = Color::Indexed(28);

/// For light terminal backgrounds: the same layout with the bars inverted, a
/// light selection and darker yellow/green.
pub const LIGHT: Theme = Theme {
    name: "light",
    bar: Style::new().fg(Color::White).bg(Color::Black),
    bar_dim: Color::Gray,
    bar_key: Style::new().fg(Color::Black).bg(Color::White),
    accent: Color::Blue,
    on_accent: Style::new().fg(Color::White).bg(Color::Blue),
    tab_inactive: Color::DarkGray,
    context_bar: Style::new().fg(Color::Black).bg(Color::Gray),
    selected_bg: Color::Gray,
    selected_fg_on_bg: Color::DarkGray,
    dim: Color::DarkGray,
    success: DARK_GREEN,
    warning: DARK_YELLOW,
    error: Color::Red,
    neutral: Color::DarkGray,
    highlight: DARK_YELLOW,
    cursor: Style::new().fg(Color::White).bg(DARK_YELLOW),
    danger: Style::new().fg(Color::White).bg(Color::Red),
    refresh_on: Style::new().fg(Color::White).bg(DARK_GREEN),
    refresh_off: Style::new().fg(Color::Black).bg(Color::Gray),
};

impl Theme {
    pub fn dim(&self) -> Style {
        Style::new().fg(self.dim)
    }

    /// A text colour for the selected row: `color` unless it would vanish
    /// into the selection background.
    pub fn on_selected(&self, color: Color) -> Color {
        if color == self.selected_bg {
            self.selected_fg_on_bg
        } else {
            color
        }
    }

    /// Every style that sets both colours, for contrast checks.
    pub fn filled_styles(&self) -> [(&'static str, Style); 8] {
        [
            ("bar", self.bar),
            ("bar_key", self.bar_key),
            ("on_accent", self.on_accent),
            ("context_bar", self.context_bar),
            ("cursor", self.cursor),
            ("danger", self.danger),
            ("refresh_on", self.refresh_on),
            ("refresh_off", self.refresh_off),
        ]
    }

    /// Text colours that may appear on the selected row.
    pub fn row_colors(&self) -> [Color; 6] {
        [
            self.success,
            self.warning,
            self.error,
            self.neutral,
            self.dim,
            self.highlight,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEMES: [&Theme; 2] = [&DARK, &LIGHT];

    #[test]
    fn resolution() {
        use Appearance as A;
        use ThemeChoice as C;
        assert_eq!(C::Auto.resolve(Some(A::Light)).name, "light");
        assert_eq!(C::Auto.resolve(Some(A::Dark)).name, "dark");
        assert_eq!(C::Auto.resolve(None).name, "dark", "unknown: dark");
        assert_eq!(C::Dark.resolve(Some(A::Light)).name, "dark");
        assert_eq!(C::Light.resolve(Some(A::Dark)).name, "light");
    }

    #[test]
    fn parsing() {
        for value in ThemeChoice::VALUES {
            assert!(ThemeChoice::parse(value).is_some(), "{value}");
        }
        assert_eq!(ThemeChoice::parse(" Light "), Some(ThemeChoice::Light));
        assert_eq!(ThemeChoice::parse("solarized"), None);
    }

    #[test]
    fn filled_styles_are_readable() {
        for theme in THEMES {
            for (role, style) in theme.filled_styles() {
                let (fg, bg) = (style.fg.unwrap(), style.bg.unwrap());
                assert_ne!(fg, bg, "{}: {role}", theme.name);
            }
        }
    }

    #[test]
    fn bar_text_is_readable() {
        for theme in THEMES {
            let bar = theme.bar.bg.unwrap();
            for (role, color) in [("bar_dim", theme.bar_dim), ("success", theme.success)] {
                assert_ne!(color, bar, "{}: {role} on the bar", theme.name);
            }
        }
    }

    #[test]
    fn selected_row_text_stays_readable() {
        for theme in THEMES {
            for color in theme.row_colors() {
                assert_ne!(
                    theme.on_selected(color),
                    theme.selected_bg,
                    "{}: {color:?}",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn dark_is_the_original_look() {
        // The colours Leeroy used before themes existed.
        assert_eq!(DARK.bar, Style::new().fg(Color::Black).bg(Color::White));
        assert_eq!(DARK.selected_bg, Color::DarkGray);
        assert_eq!(DARK.accent, Color::Blue);
    }
}
