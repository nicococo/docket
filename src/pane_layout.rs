// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 nicococo

//! The fixed 4-pane arrangement: Calendar / Feeds (AI news) / Notes /
//! Email. docket is an opinionated dashboard, not a configurable one —
//! this replaces what used to be a user-editable `[layout]` grid with a
//! single hardcoded shape. The math (independent column/row weight
//! splits, cells as unions of slices) mirrors the old generic grid
//! resolver exactly, so the on-screen result is pixel-identical to the
//! layout every existing install already renders.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Column weights (sum doesn't need to be 100 — `Constraint::Ratio`
/// normalises), matching the arrangement docket has shipped since the
/// grid layout was configurable: a wider right column for the tall
/// feeds pane.
const COLUMNS: [u16; 2] = [56, 44];
/// Row weights: top two rows share the calendar/notes stack and the
/// feeds pane; the bottom row is the full-width email pane.
const ROWS: [u16; 3] = [34, 33, 33];

/// Stable ids for the four fixed panes. These are also the
/// `WidgetManager` keys `register_default_widgets` registers under.
pub const CALENDAR: &str = "calendar";
pub const FEEDS: &str = "feeds@ai";
pub const NOTES: &str = "notes";
pub const EMAIL: &str = "email";

/// Focus-cycling order (Tab / Shift+Tab), also the order panes render
/// and appear in the help overlay.
pub const FOCUS_ORDER: [&str; 4] = [CALENDAR, FEEDS, NOTES, EMAIL];

/// The four fixed pane rectangles for a given terminal area.
#[derive(Debug, Clone, Copy)]
pub struct PaneAreas {
    pub calendar: Rect,
    pub feeds: Rect,
    pub notes: Rect,
    pub email: Rect,
}

impl PaneAreas {
    /// Resolve the fixed grid against `area`: calendar top-left, feeds
    /// top-right (spanning both top rows), notes mid-left, email
    /// bottom (spanning both columns).
    pub fn resolve(area: Rect) -> Self {
        let col_slices = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(weights_to_constraints(&COLUMNS))
            .split(area);
        let row_slices = Layout::default()
            .direction(Direction::Vertical)
            .constraints(weights_to_constraints(&ROWS))
            .split(area);

        let union = |col_start: usize, col_end: usize, row_start: usize, row_end: usize| -> Rect {
            let x = col_slices[col_start].x;
            let y = row_slices[row_start].y;
            let width = col_slices[col_end].x + col_slices[col_end].width - x;
            let height = row_slices[row_end].y + row_slices[row_end].height - y;
            Rect { x, y, width, height }
        };

        Self {
            calendar: union(0, 0, 0, 0),
            feeds: union(1, 1, 0, 1),
            notes: union(0, 0, 1, 1),
            email: union(0, 1, 2, 2),
        }
    }

    /// `(widget id, area)` pairs in focus order — the shape every render
    /// / hit-test loop needs.
    pub fn iter(&self) -> [(&'static str, Rect); 4] {
        [
            (CALENDAR, self.calendar),
            (FEEDS, self.feeds),
            (NOTES, self.notes),
            (EMAIL, self.email),
        ]
    }

    /// Area for a specific pane id, or `None` if `id` isn't one of the
    /// four fixed panes (e.g. a stale zoom target after a hot-reload).
    pub fn get(&self, id: &str) -> Option<Rect> {
        match id {
            CALENDAR => Some(self.calendar),
            FEEDS => Some(self.feeds),
            NOTES => Some(self.notes),
            EMAIL => Some(self.email),
            _ => None,
        }
    }
}

fn weights_to_constraints(weights: &[u16]) -> Vec<Constraint> {
    let sum: u32 = weights.iter().map(|w| u32::from(*w)).sum();
    weights
        .iter()
        .map(|w| Constraint::Ratio(u32::from(*w), sum))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_fills_area_with_expected_arrangement() {
        let panes = PaneAreas::resolve(Rect::new(0, 0, 100, 40));

        // Calendar top-left.
        assert_eq!(panes.calendar.x, 0);
        assert_eq!(panes.calendar.y, 0);

        // Feeds top-right, spans both top rows (taller than calendar).
        assert!(panes.feeds.x > 0);
        assert_eq!(panes.feeds.y, 0);
        assert!(panes.feeds.height > panes.calendar.height);

        // Notes below calendar, same column.
        assert_eq!(panes.notes.x, panes.calendar.x);
        assert!(panes.notes.y > panes.calendar.y);

        // Email spans the full width at the bottom.
        assert_eq!(panes.email.x, 0);
        assert_eq!(panes.email.width, panes.calendar.width + panes.feeds.width);
        assert!(panes.email.y > panes.notes.y);
    }

    #[test]
    fn iter_and_get_agree() {
        let panes = PaneAreas::resolve(Rect::new(0, 0, 100, 40));
        for (id, area) in panes.iter() {
            assert_eq!(panes.get(id), Some(area));
        }
        assert_eq!(panes.get("unknown"), None);
    }

    #[test]
    fn focus_order_matches_pane_ids() {
        let panes = PaneAreas::resolve(Rect::new(0, 0, 100, 40));
        for id in FOCUS_ORDER {
            assert!(panes.get(id).is_some());
        }
    }
}
