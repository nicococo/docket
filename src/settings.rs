// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 nicococo

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub struct Settings {
    path: PathBuf,
    original: String,
    lines: Vec<String>,
    row: usize,
    col: usize,
    error: Option<String>,
}

impl Settings {
    pub fn open(path: PathBuf) -> Result<Self> {
        let original = std::fs::read_to_string(&path)
            .with_context(|| format!("Could not read {}", path.display()))?;
        let lines = original.split('\n').map(str::to_owned).collect();
        Ok(Self {
            path,
            original,
            lines,
            row: 0,
            col: 0,
            error: None,
        })
    }

    fn byte_index(&self) -> usize {
        self.lines[self.row]
            .char_indices()
            .nth(self.col)
            .map(|(i, _)| i)
            .unwrap_or(self.lines[self.row].len())
    }

    fn clamp_col(&mut self) {
        self.col = self.col.min(self.lines[self.row].chars().count());
    }

    pub fn insert(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control() || *c == '\n') {
            if c == '\n' {
                let at = self.byte_index();
                let tail = self.lines[self.row].split_off(at);
                self.row += 1;
                self.lines.insert(self.row, tail);
                self.col = 0;
            } else {
                let at = self.byte_index();
                self.lines[self.row].insert(at, c);
                self.col += 1;
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        self.error = None;
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                if let Err(err) = self.save() {
                    self.error = Some(format!("{err:#}"));
                } else {
                    return true;
                }
            }
            (_, KeyCode::Esc) => return true,
            (_, KeyCode::Up) => {
                self.row = self.row.saturating_sub(1);
                self.clamp_col();
            }
            (_, KeyCode::Down) => {
                self.row = (self.row + 1).min(self.lines.len() - 1);
                self.clamp_col();
            }
            (_, KeyCode::Left) => self.col = self.col.saturating_sub(1),
            (_, KeyCode::Right) => {
                self.col += 1;
                self.clamp_col();
            }
            (_, KeyCode::Home) => self.col = 0,
            (_, KeyCode::End) => self.col = self.lines[self.row].chars().count(),
            (_, KeyCode::Enter) => self.insert("\n"),
            (_, KeyCode::Backspace) if self.col > 0 => {
                let end = self.byte_index();
                self.col -= 1;
                let start = self.byte_index();
                self.lines[self.row].replace_range(start..end, "");
            }
            (_, KeyCode::Backspace) if self.row > 0 => {
                let line = self.lines.remove(self.row);
                self.row -= 1;
                self.col = self.lines[self.row].chars().count();
                self.lines[self.row].push_str(&line);
            }
            (m, KeyCode::Char(c)) if m.is_empty() || m == KeyModifiers::SHIFT => {
                self.insert(&c.to_string())
            }
            _ => {}
        }
        false
    }

    fn save(&mut self) -> Result<()> {
        let text = self.lines.join("\n");
        let _: crate::config::Config = toml::from_str(&text).context("Invalid settings")?;
        if std::fs::read_to_string(&self.path)? != self.original {
            bail!("Settings changed on disk. Reopen settings before saving.");
        }
        let tmp = self
            .path
            .with_extension(format!("toml.{}.tmp", std::process::id()));
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        let result = (|| -> Result<()> {
            if let Ok(metadata) = std::fs::metadata(&self.path) {
                file.set_permissions(metadata.permissions())?;
            }
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&tmp, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result?;
        self.original = text;
        Ok(())
    }

    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        if area.width < 10 || area.height < 6 {
            return;
        }
        let rect = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::LightCyan))
            .title(Span::styled(
                " Settings · Ctrl+S save · Esc cancel ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let height = inner.height.saturating_sub(1) as usize;
        let top = self.row.saturating_sub(height.saturating_sub(1));
        let horizontal = self
            .col
            .saturating_sub(inner.width.saturating_sub(1) as usize);
        let lines: Vec<Line> = self
            .lines
            .iter()
            .skip(top)
            .take(height)
            .map(|line| Line::raw(line.chars().skip(horizontal).collect::<String>()))
            .collect();
        frame.render_widget(
            Paragraph::new(lines),
            Rect::new(inner.x, inner.y, inner.width, height as u16),
        );
        let status = self
            .error
            .as_deref()
            .unwrap_or("Edit configuration · credentials remain in credentials/");
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(if self.error.is_some() {
                Color::LightRed
            } else {
                Color::LightYellow
            })),
            Rect::new(
                inner.x,
                inner.y + inner.height.saturating_sub(1),
                inner.width,
                1,
            ),
        );
        if height > 0 {
            frame.set_cursor_position((
                inner.x + (self.col - horizontal) as u16,
                inner.y + (self.row - top) as u16,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fixture() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "docket-settings-{}-{}.toml",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, "[global]\nmouse_capture = true\n").unwrap();
        path
    }

    #[test]
    fn saves_valid_settings_and_rejects_invalid_toml() {
        let path = fixture();
        let mut settings = Settings::open(path.clone()).unwrap();
        settings.lines[1] = "mouse_capture = false".into();
        settings.save().unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("mouse_capture = false"));
        settings.lines[1] = "mouse_capture = [".into();
        assert!(settings.save().is_err());
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("mouse_capture = false"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn does_not_overwrite_external_changes() {
        let path = fixture();
        let mut settings = Settings::open(path.clone()).unwrap();
        std::fs::write(&path, "[global]\nmouse_capture = false\n").unwrap();
        assert!(settings.save().is_err());
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("mouse_capture = false"));
        std::fs::remove_file(path).unwrap();
    }
}
