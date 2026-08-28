// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 nicococo

//! Pure parsing/mutation helpers for "board" notes. No I/O, no widget
//! state — everything here operates on a note body as a plain
//! `&str`/`&mut String`, mirroring how `store.rs` keeps persistence
//! logic separate and independently testable.
//!
//! A note opts into board rendering with a marker line right after
//! the title (line 0): [`MARKER`] on line 1. `## Heading` lines from
//! there on become columns, in the order they appear; GFM task-list
//! items (`- [ ] ...` / `- [x] ...`) directly under a heading become
//! that column's cards. Everything else about the file — plain
//! Markdown, one `.md` per note — is unchanged; board is a way of
//! *viewing and navigating* the body, not a new storage format.

/// The line that flags a note as board-rendered. Lives at line index
/// 1 (right after the title on line 0) so it doesn't collide with the
/// title-is-first-line convention `store.rs` relies on for filenames.
pub const MARKER: &str = "<!-- board -->";

const UNCHECKED_PREFIX: &str = "- [ ]";
const CHECKED_PREFIX: &str = "- [x]";

/// One card: a single task-list line under some column's heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// Source line index (0-based) this card came from.
    pub line: usize,
    pub checked: bool,
    pub text: String,
}

/// One column: a `## Heading` line plus the cards found under it,
/// before the next heading (or end of file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub heading_line: usize,
    pub title: String,
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoardModel {
    pub columns: Vec<Column>,
}

/// Split a note body into lines the same way the rest of the notes
/// widget counts them: an empty body is one blank line, and a
/// trailing `\n` contributes one extra trailing blank line (so the
/// cursor has somewhere to sit after pressing Enter at end-of-file).
fn split_lines(body: &str) -> Vec<String> {
    if body.is_empty() {
        return vec![String::new()];
    }
    body.split('\n').map(str::to_string).collect()
}

fn join_lines(lines: &[String]) -> String {
    lines.join("\n")
}

/// True when `body`'s second line (index 1) is exactly [`MARKER`]
/// (surrounding whitespace ignored).
pub fn is_board(body: &str) -> bool {
    split_lines(body).get(1).map(|l| l.trim()) == Some(MARKER)
}

/// Add or remove the board marker on `body`. Idempotent toggle: if
/// the note is already a board, this turns it back into a plain note
/// and vice versa. Safe on a title-only (single-line) body.
pub fn toggle_marker(body: &mut String) {
    let mut lines = split_lines(body);
    if lines.get(1).map(|l| l.trim()) == Some(MARKER) {
        lines.remove(1);
    } else {
        let at = 1.min(lines.len());
        lines.insert(at, MARKER.to_string());
    }
    *body = join_lines(&lines);
}

fn parse_card(trimmed: &str) -> Option<(bool, String)> {
    if let Some(rest) = trimmed.strip_prefix(UNCHECKED_PREFIX) {
        return Some((false, rest.trim().to_string()));
    }
    if let Some(rest) = trimmed
        .strip_prefix(CHECKED_PREFIX)
        .or_else(|| trimmed.strip_prefix("- [X]"))
    {
        return Some((true, rest.trim().to_string()));
    }
    None
}

/// Parse `body` into columns. Lines before the first `## Heading`
/// (other than the title and marker) don't belong to any column and
/// are simply not represented in the model — they're still there in
/// the raw text, just not part of the board.
pub fn parse(body: &str) -> BoardModel {
    let lines = split_lines(body);
    let mut columns: Vec<Column> = Vec::new();
    let mut current: Option<Column> = None;

    for (i, raw) in lines.iter().enumerate() {
        // Title (line 0) and marker (line 1, if present) are never
        // headings or cards.
        if i == 0 {
            continue;
        }
        if i == 1 && raw.trim() == MARKER {
            continue;
        }
        let trimmed = raw.trim_start();
        if let Some(title) = trimmed.strip_prefix("## ") {
            if let Some(col) = current.take() {
                columns.push(col);
            }
            current = Some(Column {
                heading_line: i,
                title: title.trim().to_string(),
                cards: Vec::new(),
            });
            continue;
        }
        if let Some(col) = current.as_mut() {
            if let Some((checked, text)) = parse_card(trimmed) {
                col.cards.push(Card {
                    line: i,
                    checked,
                    text,
                });
            }
        }
    }
    if let Some(col) = current.take() {
        columns.push(col);
    }
    BoardModel { columns }
}

/// Flip `- [ ]` <-> `- [x]` on `line`, preserving leading indent and
/// trailing text. No-op if that line isn't a task-list line (a
/// defensive guard — callers should only pass card lines from a
/// freshly-parsed [`BoardModel`]).
pub fn toggle_checkbox(body: &mut String, line: usize) {
    let mut lines = split_lines(body);
    let Some(l) = lines.get_mut(line) else { return };
    let trimmed = l.trim_start();
    let indent = &l[..l.len() - trimmed.len()];
    let new_line = if let Some(rest) = trimmed.strip_prefix(UNCHECKED_PREFIX) {
        format!("{indent}{CHECKED_PREFIX}{rest}")
    } else if let Some(rest) = trimmed
        .strip_prefix(CHECKED_PREFIX)
        .or_else(|| trimmed.strip_prefix("- [X]"))
    {
        format!("{indent}{UNCHECKED_PREFIX}{rest}")
    } else {
        return;
    };
    *l = new_line;
    *body = join_lines(&lines);
}

/// The line index at which a new card should be appended to `col` —
/// either the next column's heading line, or `total_lines` (end of
/// file) if `col` is the last column. `total_lines` is the caller's
/// current line count (i.e. `split_lines(body).len()`).
pub fn column_insert_line(model: &BoardModel, col: usize, total_lines: usize) -> Option<usize> {
    model.columns.get(col)?;
    Some(match model.columns.get(col + 1) {
        Some(next) => next.heading_line,
        None => total_lines,
    })
}

/// Move the card at `model.columns[col].cards[row]` into the
/// adjacent column `col + dir` (`dir` is `1` or `-1`), appending it
/// as that column's last card. Returns the new column index on
/// success; `None` (body untouched) if there's no such source card or
/// no column in that direction.
pub fn move_card(body: &mut String, model: &BoardModel, col: usize, row: usize, dir: i32) -> Option<usize> {
    let target_col = col as i32 + dir;
    if target_col < 0 {
        return None;
    }
    let target_col = target_col as usize;
    if target_col >= model.columns.len() {
        return None;
    }
    let card_line = model.columns.get(col)?.cards.get(row)?.line;

    let mut lines = split_lines(body);
    let total_before = lines.len();
    let text = lines.remove(card_line);

    let raw_insert_at = column_insert_line(model, target_col, total_before)?;
    // Removing `card_line` shifts every later index down by one.
    let insert_at = if raw_insert_at > card_line {
        raw_insert_at - 1
    } else {
        raw_insert_at
    }
    .min(lines.len());
    lines.insert(insert_at, text);
    *body = join_lines(&lines);
    Some(target_col)
}

/// Extract `[[Note Name]]`-style wikilinks from `text`, trimmed, in
/// order of appearance. Duplicates are kept (rare in a single card,
/// and de-duplicating buys little). An unterminated `[[` at the end
/// of `text` is ignored rather than treated as a link.
pub fn extract_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            break;
        };
        let name = after[..end].trim();
        if !name.is_empty() {
            out.push(name.to_string());
        }
        rest = &after[end + 2..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "My Board\n<!-- board -->\n\n## Todo\n- [ ] write plan\n- [x] pick columns\n\n## Doing\n- [ ] implement\n\n## Done\n";

    #[test]
    fn is_board_detects_marker_line() {
        assert!(is_board(SAMPLE));
        assert!(!is_board("Plain note\nsome text"));
        assert!(!is_board(""));
        assert!(!is_board("Title only"));
    }

    #[test]
    fn toggle_marker_adds_and_removes() {
        let mut body = "Title only".to_string();
        toggle_marker(&mut body);
        assert_eq!(body, "Title only\n<!-- board -->");
        assert!(is_board(&body));
        toggle_marker(&mut body);
        assert_eq!(body, "Title only");
        assert!(!is_board(&body));
    }

    #[test]
    fn toggle_marker_on_empty_body() {
        let mut body = String::new();
        toggle_marker(&mut body);
        assert!(is_board(&body));
        toggle_marker(&mut body);
        assert!(!is_board(&body));
    }

    #[test]
    fn parse_extracts_columns_and_cards_in_order() {
        let model = parse(SAMPLE);
        assert_eq!(model.columns.len(), 3);
        assert_eq!(model.columns[0].title, "Todo");
        assert_eq!(model.columns[0].cards.len(), 2);
        assert_eq!(model.columns[0].cards[0].text, "write plan");
        assert!(!model.columns[0].cards[0].checked);
        assert_eq!(model.columns[0].cards[1].text, "pick columns");
        assert!(model.columns[0].cards[1].checked);
        assert_eq!(model.columns[1].title, "Doing");
        assert_eq!(model.columns[1].cards.len(), 1);
        assert_eq!(model.columns[2].title, "Done");
        assert!(model.columns[2].cards.is_empty());
    }

    #[test]
    fn parse_handles_marker_with_no_headings_yet() {
        let model = parse("Title\n<!-- board -->\n");
        assert!(model.columns.is_empty());
    }

    #[test]
    fn parse_ignores_non_card_lines_under_a_heading() {
        let model = parse("T\n<!-- board -->\n## Todo\nsome plain note\n- [ ] a card\n");
        assert_eq!(model.columns[0].cards.len(), 1);
        assert_eq!(model.columns[0].cards[0].text, "a card");
    }

    #[test]
    fn toggle_checkbox_flips_state_and_is_noop_on_non_card_lines() {
        let mut body = SAMPLE.to_string();
        toggle_checkbox(&mut body, 4); // "- [ ] write plan"
        assert!(body.lines().nth(4).unwrap().starts_with("- [x]"));
        toggle_checkbox(&mut body, 4);
        assert!(body.lines().nth(4).unwrap().starts_with("- [ ]"));

        let before = body.clone();
        toggle_checkbox(&mut body, 3); // "## Todo" heading line
        assert_eq!(body, before, "toggling a non-card line must be a no-op");
    }

    #[test]
    fn move_card_moves_to_adjacent_column_and_reports_new_index() {
        let mut body = SAMPLE.to_string();
        let model = parse(&body);
        // "write plan" is Todo.cards[0] -> move to Doing (col 0 -> 1).
        let new_col = move_card(&mut body, &model, 0, 0, 1);
        assert_eq!(new_col, Some(1));

        let model2 = parse(&body);
        assert_eq!(model2.columns[0].cards.len(), 1);
        assert_eq!(model2.columns[0].cards[0].text, "pick columns");
        assert_eq!(model2.columns[1].cards.len(), 2);
        assert_eq!(model2.columns[1].cards[1].text, "write plan");
    }

    #[test]
    fn move_card_into_last_column_appends_at_end_of_file() {
        let mut body = SAMPLE.to_string();
        let model = parse(&body);
        // "implement" is Doing.cards[0] -> move to Done (col 1 -> 2), the last column.
        let new_col = move_card(&mut body, &model, 1, 0, 1);
        assert_eq!(new_col, Some(2));
        let model2 = parse(&body);
        assert!(model2.columns[1].cards.is_empty());
        assert_eq!(model2.columns[2].cards.len(), 1);
        assert_eq!(model2.columns[2].cards[0].text, "implement");
    }

    #[test]
    fn move_card_at_edges_is_a_noop() {
        let mut body = SAMPLE.to_string();
        let model = parse(&body);
        assert_eq!(move_card(&mut body, &model, 0, 0, -1), None);
        assert_eq!(body, SAMPLE, "no columns to the left of the first: body unchanged");

        let mut body2 = SAMPLE.to_string();
        let model2 = parse(&body2);
        assert_eq!(move_card(&mut body2, &model2, 2, 0, 1), None); // Done has no cards anyway
        assert_eq!(body2, SAMPLE);
    }

    #[test]
    fn column_insert_line_uses_next_heading_or_end_of_file() {
        let model = parse(SAMPLE);
        let total = split_lines(SAMPLE).len();
        assert_eq!(
            column_insert_line(&model, 0, total),
            Some(model.columns[1].heading_line)
        );
        assert_eq!(column_insert_line(&model, 2, total), Some(total));
        assert_eq!(column_insert_line(&model, 99, total), None);
    }

    #[test]
    fn extract_links_finds_all_wikilinks_in_order() {
        assert_eq!(
            extract_links("- [ ] follow up on [[Project Plan]] and [[Budget]]"),
            vec!["Project Plan".to_string(), "Budget".to_string()]
        );
    }

    #[test]
    fn extract_links_trims_whitespace_inside_brackets() {
        assert_eq!(
            extract_links("[[  Spaced Note  ]]"),
            vec!["Spaced Note".to_string()]
        );
    }

    #[test]
    fn extract_links_returns_empty_for_no_links() {
        assert!(extract_links("just a plain task").is_empty());
    }

    #[test]
    fn extract_links_ignores_empty_and_unterminated_brackets() {
        assert!(extract_links("[[]]").is_empty());
        assert!(extract_links("dangling [[unterminated").is_empty());
        assert_eq!(
            extract_links("ok [[First]] then dangling [[").len(),
            1
        );
    }
}
