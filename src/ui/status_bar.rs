// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

use chrono::Local;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::theme::Theme;

/// Bottom chrome keeps focus and global keys visible at narrow widths.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    focused_widget: Option<&str>,
    mouse_capture: bool,
    theme: &Theme,
) {
    let now = Local::now();
    let clock = now.format("%H:%M:%S").to_string();
    let version = env!("CARGO_PKG_VERSION");
    let focus = focused_widget.unwrap_or("—");

    let dim = Style::default().add_modifier(Modifier::DIM);
    let sep = Span::styled("│", dim);
    let key = theme.widget_title_focused;
    let mut spans: Vec<Span> = vec![Span::styled(" DOCKET ", key)];
    if area.width >= 104 {
        spans.extend([
            Span::styled(format!(" v{version} "), dim),
            sep.clone(),
            Span::styled(format!(" {clock} "), dim),
        ]);
    } else if area.width >= 72 {
        spans.push(Span::styled(format!(" {clock} "), dim));
    }
    spans.extend([
        sep.clone(),
        Span::styled(" focus ", dim),
        Span::styled(focus.to_string(), theme.text_focused),
        sep.clone(),
    ]);
    if area.width >= 96 {
        spans.extend([
            Span::styled(" Tab ", key),
            Span::styled(" panes  ", dim),
            Span::styled(" z ", key),
            Span::styled(" zoom  ", dim),
            Span::styled(" s ", key),
            Span::styled(" settings  ", dim),
            Span::styled(" ? ", key),
            Span::styled(" help  ", dim),
            Span::styled(" q ", key),
            Span::styled(" quit", dim),
        ]);
    } else if area.width >= 82 {
        spans.extend([
            Span::styled("Tab", theme.text_shortcut),
            Span::styled(" · z zoom · s settings · ? help · q", theme.text_selected),
        ]);
    } else {
        spans.extend([
            Span::styled("z", theme.text_shortcut),
            Span::styled(" · ", dim),
            Span::styled("s", theme.text_shortcut),
            Span::styled(" settings · ", theme.text_selected),
            Span::styled("?", theme.text_shortcut),
            Span::styled(" · ", dim),
            Span::styled("q", theme.text_shortcut),
        ]);
    }
    if !mouse_capture && area.width >= 120 {
        spans.extend([sep, Span::styled(" Ctrl+T mouse ", theme.text_shortcut)]);
    }

    let line = Line::from(spans).alignment(Alignment::Left);
    frame.render_widget(Paragraph::new(line), area);
}
