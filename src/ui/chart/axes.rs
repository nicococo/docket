// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Chart-axis primitives shared by widgets that render a braille
//! time-series. Today: the right-anchored x-axis label layout and the
//! horizontal reference-line overlay. All routines are
//! presentation-only — they don't know what units the series carries.

use ratatui::{style::Style, Frame};

/// Place `labels` evenly across a line of `width` cells, right-anchored:
/// the last label's right edge sits at exactly column `width`, the first
/// label sits flush-left at column 0, intermediate labels are spaced
/// linearly. A trailing char that would overflow `width` is clipped.
pub fn lay_out_x_axis_labels(labels: &[&str], width: usize) -> String {
    if labels.is_empty() || width == 0 {
        return String::new();
    }
    let n = labels.len();
    if n == 1 {
        return labels[0].chars().take(width).collect();
    }
    let last_w = labels.last().map(|s| s.chars().count()).unwrap_or(0);
    let usable = width.saturating_sub(last_w);
    let mut line = String::with_capacity(width);
    for (i, lbl) in labels.iter().enumerate() {
        let target = (i * usable) / (n - 1);
        while line.chars().count() < target {
            line.push(' ');
        }
        for c in lbl.chars() {
            if line.chars().count() >= width {
                break;
            }
            line.push(c);
        }
    }
    line
}

/// Overlay a horizontal reference line at the row corresponding to
/// `value`, painting `ch` only at columns the trace left blank. Writing
/// directly into the frame buffer keeps the trace's braille glyphs
/// intact where they sit on the same row.
///
/// `trace_rows` is the rendered braille trace (`plot_h` rows of `plot_w`
/// chars). The overlay walks the full `plot_w` even when the trace is
/// narrower (e.g. 1D trading-day-progress mode) so the line extends
/// across the empty "future trading time" portion too.
#[allow(clippy::too_many_arguments)]
pub fn draw_reference_line(
    frame: &mut Frame,
    plot_x: u16,
    plot_top: u16,
    plot_h: u16,
    plot_w: u16,
    plot_min: f64,
    plot_max: f64,
    trace_rows: &[String],
    value: f64,
    ch: char,
    style: Style,
) {
    if plot_h == 0 || !value.is_finite() || plot_max <= plot_min {
        return;
    }
    if value < plot_min || value > plot_max {
        return;
    }
    let frac = (plot_max - value) / (plot_max - plot_min);
    let ref_row = (frac * (plot_h as f64 - 1.0)).round() as usize;
    if ref_row >= trace_rows.len() {
        return;
    }
    let trace = &trace_rows[ref_row];
    let trace_chars: Vec<char> = trace.chars().collect();
    let y = plot_top + ref_row as u16;
    let buf = frame.buffer_mut();
    for i in 0..plot_w as usize {
        let trace_owns_cell = match trace_chars.get(i) {
            Some(&c) => c != ' ',
            None => false,
        };
        if trace_owns_cell {
            continue;
        }
        let x = plot_x + i as u16;
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(ch);
            cell.set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lay_out_x_axis_labels_right_anchors_last_label() {
        let out = lay_out_x_axis_labels(&["A", "B", "C"], 10);
        assert_eq!(out.chars().count(), 10);
        // Last label's right edge sits at column 10 (i.e. char 9 is 'C').
        assert!(out.ends_with('C'));
        assert!(out.starts_with('A'));
    }

    #[test]
    fn lay_out_x_axis_labels_handles_empty_and_single() {
        assert_eq!(lay_out_x_axis_labels(&[], 10), "");
        assert_eq!(lay_out_x_axis_labels(&["ABC"], 10), "ABC");
        assert_eq!(lay_out_x_axis_labels(&["ABC"], 2), "AB");
    }
}
