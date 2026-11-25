// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone)]
pub struct Theme {
    pub border_focused: Style,
    pub border_unfocused: Style,
    pub widget_title_focused: Style,
    pub widget_title_unfocused: Style,
    pub metadata_focused: Style,
    pub metadata_unfocused: Style,
    pub text_plain: Style,
    pub text_brilliant: Style,
    pub text_dim: Style,
    pub text_selected: Style,
    pub text_focused: Style,
    pub text_shortcut: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self::builtin_defaults()
    }
}

impl Theme {
    /// Hardcoded fallback palette — matches the colors docket shipped with
    /// before the theme system existed. Used to fill in any roles a scheme
    /// omits.
    pub fn builtin_defaults() -> Self {
        Self {
            border_focused: Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            border_unfocused: Style::default().fg(Color::Magenta),
            // Focus on the title is conveyed by the `┤ ├` bracket pad
            // (painted in border_focused) — the title text itself stays
            // the same bold across states. Schemes can still override
            // either side to add a fg shift if they want.
            widget_title_focused: Style::default()
                .fg(Color::Black)
                .bg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            widget_title_unfocused: Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
            metadata_focused: Style::default().fg(Color::LightGreen),
            metadata_unfocused: Style::default().add_modifier(Modifier::DIM),
            text_plain: Style::default(),
            text_brilliant: Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
            text_dim: Style::default().add_modifier(Modifier::DIM),
            text_selected: Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
            text_focused: Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            text_shortcut: Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        }
    }

    /// Helper used by widget borders: picks `border_focused` when the cell
    /// is currently focused, `border_unfocused` otherwise.
    pub fn border_style(&self, focused: bool) -> Style {
        if focused {
            self.border_focused
        } else {
            self.border_unfocused
        }
    }

    /// Focused/unfocused pair for the title text on the border.
    pub fn widget_title_style(&self, focused: bool) -> Style {
        if focused {
            self.widget_title_focused
        } else {
            self.widget_title_unfocused
        }
    }

    /// Focused/unfocused pair for the right-aligned metadata suffix.
    pub fn metadata_style(&self, focused: bool) -> Style {
        if focused {
            self.metadata_focused
        } else {
            self.metadata_unfocused
        }
    }
}

pub(crate) fn parse_color(s: &str) -> Option<Color> {
    let norm = s.trim().to_ascii_lowercase().replace('-', "_");
    if norm.is_empty() || norm == "default" || norm == "reset" || norm == "none" {
        return None;
    }
    if let Some(c) = parse_hex_color(&norm) {
        return Some(c);
    }
    Some(match norm.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "dark_gray" | "dark_grey" => Color::DarkGray,
        "light_red" | "bright_red" => Color::LightRed,
        "light_green" | "bright_green" => Color::LightGreen,
        "light_yellow" | "bright_yellow" => Color::LightYellow,
        "light_blue" | "bright_blue" => Color::LightBlue,
        "light_magenta" | "bright_magenta" | "light_purple" => Color::LightMagenta,
        "light_cyan" | "bright_cyan" => Color::LightCyan,
        _ => {
            tracing::warn!(color = %s, "unknown color name, treating as inherit");
            return None;
        }
    })
}

fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}
