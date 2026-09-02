// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 nicococo

//! Add/remove extracted-todo and extracted-date items into the Notes
//! and Calendar widgets, from the Email popup's `ExtractTodo` action
//! (see `mod.rs`'s `AiPopup`/`render_ai_popup`/`handle_popup_key`).
//!
//! This is the first place in docket where one widget writes another
//! widget's on-disk data. Rather than inventing a cross-widget
//! command bus, it calls straight into `notes::store`/`notes::board`
//! — both are already pure, `Arc<Mutex<NotesState>>`-independent
//! library functions — and a small new calendar helper module
//! (`calendar::local`) that does the equivalent for `calendar.toml`.
//! Both integrations are gated behind the same Cargo feature that
//! gates the target widget itself, so a slim `--features widget-email`
//! build compiles fine; the functions below just become no-ops and
//! `*_available()` reports `false` so the popup can hide the affected
//! keys/rows rather than offering something that can't work.
//!
//! **Add/remove state has no separate tracking file** — each item's
//! "added" state is derived by searching the target file for a
//! `docket:extract:<id>` marker (see `item_id`). Removing the marker
//! by hand (or through docket) always means "not added" again, and
//! nothing can drift out of sync with what's actually on disk.

/// Stable id for one extracted item — same message + same item content
/// always hashes to the same id, so "already added" detection survives
/// a cache clear / re-extraction. `kind` namespaces todos from dates so
/// a todo and a date that happen to share text don't collide.
fn item_id(kind: &str, message_id: &str, content: &str) -> String {
    crate::cache::short_hash_key("", &format!("{kind}\u{0}{message_id}\u{0}{content}"))
}

pub fn todo_item_id(message_id: &str, item_text: &str) -> String {
    item_id("todo", message_id, item_text)
}

pub fn date_item_id(message_id: &str, title: &str, date: &str) -> String {
    item_id("date", message_id, &format!("{title}\u{0}{date}"))
}

fn marker_line(id: &str) -> String {
    format!("<!-- docket:extract:{id} -->")
}

// ── Notes (todos) ───────────────────────────────────────────────────

pub const TODO_NOTE_TITLE: &str = "Email Todos";
const TODO_COLUMN: &str = "Todo";

pub fn notes_integration_available() -> bool {
    cfg!(feature = "widget-notes")
}

#[cfg(feature = "widget-notes")]
mod notes_impl {
    use super::{marker_line, TODO_COLUMN, TODO_NOTE_TITLE};
    use crate::widgets::notes::{board, store};
    use anyhow::Result;

    /// Every source line as `split('\n')` would — mirrors
    /// `board.rs`'s private `split_lines` convention (empty body is
    /// one blank line) since that's what `column_insert_line`'s
    /// `total_lines` expects and we can't reach the private helper
    /// from outside the notes module.
    fn line_count(body: &str) -> usize {
        if body.is_empty() {
            1
        } else {
            body.split('\n').count()
        }
    }

    fn insert_lines_at(body: &str, at: usize, new_lines: &[&str]) -> String {
        let mut lines: Vec<String> = if body.is_empty() {
            vec![String::new()]
        } else {
            body.split('\n').map(str::to_string).collect()
        };
        let at = at.min(lines.len());
        for (i, l) in new_lines.iter().enumerate() {
            lines.insert(at + i, l.to_string());
        }
        lines.join("\n")
    }

    fn load_todo_note() -> Result<(std::path::PathBuf, String, Option<store::Note>)> {
        // Read the user's actual configured `notes_dir` rather than
        // assuming docket's built-in default — Notes is very often
        // pointed at something like an Obsidian vault, and landing
        // "Email Todos" somewhere the user never opens defeats the
        // point. Re-reads config.toml directly (cheap, infrequent —
        // only on extract-popup add/remove) rather than threading the
        // already-loaded app Config through email's key-handling path.
        let notes_dir = crate::config::load(None)
            .ok()
            .and_then(|cfg| cfg.notes.notes_dir);
        let (root, _) = store::resolve_root(notes_dir.as_deref())?;
        let instance = "main".to_string();
        let note = store::load_all(&root, &instance)
            .into_iter()
            .find(|n| n.display_name() == TODO_NOTE_TITLE);
        Ok((root, instance, note))
    }

    pub fn marker_present(id: &str) -> bool {
        let marker = marker_line(id);
        matches!(load_todo_note(), Ok((_, _, Some(note))) if note.body.contains(&marker))
    }

    pub fn add(item_text: &str, id: &str) -> Result<()> {
        let marker = marker_line(id);
        let (root, instance, existing) = load_todo_note()?;
        let mut note = match existing {
            Some(n) => n,
            None => {
                let mut n = store::Note {
                    id: store::new_id(),
                    body: format!(
                        "{TODO_NOTE_TITLE}\n{}\n\n## {TODO_COLUMN}\n",
                        board::MARKER
                    ),
                    modified: std::time::SystemTime::now(),
                };
                store::save(&root, &instance, &mut n)?;
                n
            }
        };
        if note.body.contains(&marker) {
            return Ok(()); // already added — idempotent
        }
        let mut model = board::parse(&note.body);
        if !model.columns.iter().any(|c| c.title == TODO_COLUMN) {
            // User hand-edited the note and removed the Todo heading —
            // put it back rather than failing to add the card.
            let mut body = note.body.clone();
            if !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str(&format!("\n## {TODO_COLUMN}\n"));
            note.body = body;
            model = board::parse(&note.body);
        }
        let col = model
            .columns
            .iter()
            .position(|c| c.title == TODO_COLUMN)
            .expect("just ensured the Todo column exists");
        let insert_at =
            board::column_insert_line(&model, col, line_count(&note.body)).unwrap_or(0);
        note.body = insert_lines_at(&note.body, insert_at, &[&marker, &format!("- [ ] {item_text}")]);
        store::save(&root, &instance, &mut note)
    }

    pub fn remove(id: &str) -> Result<()> {
        let marker = marker_line(id);
        let (root, instance, existing) = load_todo_note()?;
        let Some(mut note) = existing else {
            return Ok(());
        };
        let lines: Vec<&str> = note.body.split('\n').collect();
        let Some(idx) = lines.iter().position(|l| l.trim() == marker) else {
            return Ok(()); // already gone — idempotent
        };
        let remove_count = if lines
            .get(idx + 1)
            .is_some_and(|l| l.trim_start().starts_with("- ["))
        {
            2
        } else {
            1
        };
        let mut owned: Vec<String> = lines.into_iter().map(str::to_string).collect();
        owned.drain(idx..(idx + remove_count).min(owned.len()));
        note.body = owned.join("\n");
        store::save(&root, &instance, &mut note)
    }
}

#[cfg(feature = "widget-notes")]
pub fn todo_marker_present(id: &str) -> bool {
    notes_impl::marker_present(id)
}
#[cfg(not(feature = "widget-notes"))]
pub fn todo_marker_present(_id: &str) -> bool {
    false
}

#[cfg(feature = "widget-notes")]
pub fn add_todo(item_text: &str, id: &str) -> anyhow::Result<()> {
    notes_impl::add(item_text, id)
}
#[cfg(not(feature = "widget-notes"))]
pub fn add_todo(_item_text: &str, _id: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(feature = "widget-notes")]
pub fn remove_todo(id: &str) -> anyhow::Result<()> {
    notes_impl::remove(id)
}
#[cfg(not(feature = "widget-notes"))]
pub fn remove_todo(_id: &str) -> anyhow::Result<()> {
    Ok(())
}

// ── Calendar (dates) ────────────────────────────────────────────────

pub fn calendar_integration_available() -> bool {
    cfg!(feature = "widget-calendar")
}

#[cfg(feature = "widget-calendar")]
pub fn event_marker_present(id: &str) -> bool {
    crate::widgets::calendar::local::event_marker_present(id).unwrap_or(false)
}
#[cfg(not(feature = "widget-calendar"))]
pub fn event_marker_present(_id: &str) -> bool {
    false
}

#[cfg(feature = "widget-calendar")]
pub fn add_event(title: &str, date: &str, id: &str) -> anyhow::Result<()> {
    crate::widgets::calendar::local::add_event(title, date, id)
}
#[cfg(not(feature = "widget-calendar"))]
pub fn add_event(_title: &str, _date: &str, _id: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(feature = "widget-calendar")]
pub fn remove_event(id: &str) -> anyhow::Result<()> {
    crate::widgets::calendar::local::remove_event(id)
}
#[cfg(not(feature = "widget-calendar"))]
pub fn remove_event(_id: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(all(test, feature = "widget-notes"))]
mod tests {
    use super::*;
    use crate::widgets::test_support::IsolatedConfigHome;

    #[test]
    fn add_then_marker_present_then_remove_round_trips() {
        let _cfg = IsolatedConfigHome::new();
        let id = todo_item_id("msg-1", "Reply with numbers");
        assert!(!todo_marker_present(&id));

        add_todo("Reply with numbers", &id).unwrap();
        assert!(todo_marker_present(&id));

        remove_todo(&id).unwrap();
        assert!(!todo_marker_present(&id));
    }

    #[test]
    fn add_is_idempotent_and_creates_the_note_once() {
        let _cfg = IsolatedConfigHome::new();
        let id = todo_item_id("msg-2", "Send the deck");
        add_todo("Send the deck", &id).unwrap();
        add_todo("Send the deck", &id).unwrap();

        let (root, _) = crate::widgets::notes::store::resolve_root(None).unwrap();
        let notes = crate::widgets::notes::store::load_all(&root, "main");
        assert_eq!(notes.len(), 1, "add_todo must not create duplicate notes");
        let card_count = notes[0].body.matches("- [ ] Send the deck").count();
        assert_eq!(card_count, 1, "second add_todo call must be a no-op");
    }

    #[test]
    fn remove_of_never_added_item_is_a_noop() {
        let _cfg = IsolatedConfigHome::new();
        let id = todo_item_id("msg-3", "never added");
        remove_todo(&id).unwrap(); // must not error
        assert!(!todo_marker_present(&id));
    }

    #[test]
    fn different_messages_with_same_text_get_different_ids() {
        let a = todo_item_id("msg-a", "Follow up");
        let b = todo_item_id("msg-b", "Follow up");
        assert_ne!(a, b);
    }

    #[test]
    fn todo_and_date_ids_never_collide_on_shared_text() {
        let t = todo_item_id("msg-1", "Budget review");
        let d = date_item_id("msg-1", "Budget review", "2026-09-03");
        assert_ne!(t, d);
    }
}
