// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! App-level zoom target — names the widget currently filling the
//! centered zoom overlay.
//!
//! [`WidgetManager`]: crate::widgets::WidgetManager

/// Names the zoom target. Carried in `App::zoom_target` while zoom is
/// active; `None` means zoom is off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoomTarget {
    /// Top-level key in `WidgetManager` (one of the four fixed panes).
    pub widget_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_target_eq() {
        let a = ZoomTarget {
            widget_id: "calendar".into(),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn zoom_target_ne_different_widget() {
        let a = ZoomTarget {
            widget_id: "calendar".into(),
        };
        let b = ZoomTarget {
            widget_id: "notes".into(),
        };
        assert_ne!(a, b);
    }
}
