// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 nicococo

//! Tree view of the notes list, derived purely from note titles: a
//! title of `Work>TODOs` nests a `TODOs` leaf under a `Work` group.
//! Nothing about the hierarchy is stored on disk — files stay flat
//! (`>` is filename-unsafe, so that note lives at `Work-TODOs.md`).
//!
//! A note whose whole title matches a group's path (`Work` alongside
//! `Work>TODOs`) becomes that group's row instead of a separate leaf.
//! Sibling order follows the input order, so with `notes` sorted
//! newest-first, a group sorts by its most recently edited descendant.

use std::collections::HashSet;

use super::store::Note;

pub const SEPARATOR: char = '>';

/// One visible row of the flattened tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// Normalized path (`Work>TODOs`) — the key for expanded state.
    pub path: String,
    pub label: String,
    pub depth: usize,
    /// Index into `notes` when this row opens a note; `None` for a
    /// pure group row.
    pub note: Option<usize>,
    pub has_children: bool,
    pub expanded: bool,
    /// Notes nested anywhere below this row — shown on collapsed groups.
    pub descendant_notes: usize,
}

/// Title split on [`SEPARATOR`], segments trimmed, empty ones dropped.
/// A title with no non-empty segment keeps the whole title as one.
pub fn segments(title: &str) -> Vec<&str> {
    let segs: Vec<&str> = title
        .split(SEPARATOR)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if segs.is_empty() {
        vec![title]
    } else {
        segs
    }
}

/// Group paths enclosing a note with this title — `A>B>C` yields
/// `["A", "A>B"]`. Empty for a top-level note.
pub fn ancestor_paths(title: &str) -> Vec<String> {
    let segs = segments(title);
    (1..segs.len())
        .map(|n| segs[..n].join(&SEPARATOR.to_string()))
        .collect()
}

#[derive(Default)]
struct Node {
    label: String,
    path: String,
    note: Option<usize>,
    children: Vec<Node>,
}

impl Node {
    fn descendant_notes(&self) -> usize {
        self.children
            .iter()
            .map(|c| usize::from(c.note.is_some()) + c.descendant_notes())
            .sum()
    }
}

/// Flatten `notes` into the rows currently visible given which group
/// paths are `expanded`.
pub fn visible_rows(notes: &[Note], expanded: &HashSet<String>) -> Vec<TreeRow> {
    let mut root = Node::default();
    for (idx, note) in notes.iter().enumerate() {
        let segs = segments(note.display_name());
        let mut node = &mut root;
        for (depth, seg) in segs.iter().enumerate() {
            let last = depth + 1 == segs.len();
            // Duplicate titles: the second note at a taken path gets
            // its own sibling row rather than being swallowed.
            let pos = node
                .children
                .iter()
                .position(|c| c.label == *seg && !(last && c.note.is_some()));
            let pos = match pos {
                Some(p) => p,
                None => {
                    let path = segs[..=depth].join(&SEPARATOR.to_string());
                    node.children.push(Node {
                        label: (*seg).to_string(),
                        path,
                        ..Node::default()
                    });
                    node.children.len() - 1
                }
            };
            node = &mut node.children[pos];
            if last {
                node.note = Some(idx);
            }
        }
    }
    let mut out = Vec::new();
    flatten(&root.children, 0, expanded, &mut out);
    out
}

fn flatten(nodes: &[Node], depth: usize, expanded: &HashSet<String>, out: &mut Vec<TreeRow>) {
    for n in nodes {
        let has_children = !n.children.is_empty();
        let is_expanded = has_children && expanded.contains(&n.path);
        out.push(TreeRow {
            path: n.path.clone(),
            label: n.label.clone(),
            depth,
            note: n.note,
            has_children,
            expanded: is_expanded,
            descendant_notes: n.descendant_notes(),
        });
        if is_expanded {
            flatten(&n.children, depth + 1, expanded, out);
        }
    }
}

/// Visible row index of the parent of `rows[i]` (nearest preceding
/// row one level shallower). `None` for top-level rows.
pub fn parent_row(rows: &[TreeRow], i: usize) -> Option<usize> {
    let depth = rows.get(i)?.depth;
    if depth == 0 {
        return None;
    }
    rows[..i].iter().rposition(|r| r.depth + 1 == depth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn note(title: &str) -> Note {
        Note {
            id: title.to_string(),
            body: format!("{title}\nbody"),
            modified: SystemTime::UNIX_EPOCH,
        }
    }

    fn labels(rows: &[TreeRow]) -> Vec<(usize, &str)> {
        rows.iter().map(|r| (r.depth, r.label.as_str())).collect()
    }

    fn all_expanded(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn segments_trim_and_drop_empties() {
        assert_eq!(segments("Work > TODOs"), vec!["Work", "TODOs"]);
        assert_eq!(segments("Work>"), vec!["Work"]);
        assert_eq!(segments("plain"), vec!["plain"]);
        assert_eq!(segments(">"), vec![">"]);
    }

    #[test]
    fn ancestor_paths_normalize_spacing() {
        assert_eq!(ancestor_paths("A > B>C"), vec!["A", "A>B"]);
        assert!(ancestor_paths("solo").is_empty());
    }

    #[test]
    fn collapsed_groups_hide_children_and_count_them() {
        let notes = vec![note("Work>TODOs"), note("Shopping"), note("Work>Meetings")];
        let rows = visible_rows(&notes, &HashSet::new());
        assert_eq!(labels(&rows), vec![(0, "Work"), (0, "Shopping")]);
        assert!(rows[0].has_children && !rows[0].expanded);
        assert_eq!(rows[0].note, None);
        assert_eq!(rows[0].descendant_notes, 2);
    }

    #[test]
    fn expanded_group_lists_children_in_input_order() {
        let notes = vec![note("Work>TODOs"), note("Shopping"), note("Work>Meetings")];
        let rows = visible_rows(&notes, &all_expanded(&["Work"]));
        assert_eq!(
            labels(&rows),
            vec![(0, "Work"), (1, "TODOs"), (1, "Meetings"), (0, "Shopping")]
        );
        assert_eq!(rows[1].note, Some(0));
        assert_eq!(rows[2].note, Some(2));
    }

    #[test]
    fn note_matching_group_path_becomes_group_row() {
        let notes = vec![note("Work>TODOs"), note("Work")];
        let rows = visible_rows(&notes, &all_expanded(&["Work"]));
        assert_eq!(labels(&rows), vec![(0, "Work"), (1, "TODOs")]);
        assert_eq!(rows[0].note, Some(1));
        assert!(rows[0].has_children);
    }

    #[test]
    fn duplicate_titles_stay_separate_rows() {
        let notes = vec![note("Work>TODOs"), note("Work>TODOs")];
        let rows = visible_rows(&notes, &all_expanded(&["Work"]));
        assert_eq!(labels(&rows), vec![(0, "Work"), (1, "TODOs"), (1, "TODOs")]);
        assert_eq!(rows[1].note, Some(0));
        assert_eq!(rows[2].note, Some(1));
    }

    #[test]
    fn nested_groups_need_every_ancestor_expanded() {
        let notes = vec![note("A>B>C")];
        let only_outer = visible_rows(&notes, &all_expanded(&["A"]));
        assert_eq!(labels(&only_outer), vec![(0, "A"), (1, "B")]);
        let both = visible_rows(&notes, &all_expanded(&["A", "A>B"]));
        assert_eq!(labels(&both), vec![(0, "A"), (1, "B"), (2, "C")]);
        assert_eq!(parent_row(&both, 2), Some(1));
        assert_eq!(parent_row(&both, 1), Some(0));
        assert_eq!(parent_row(&both, 0), None);
    }
}
