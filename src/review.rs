use serde::{Deserialize, Serialize};

use crate::document::{Document, RemoteDocument};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
    pub id: u64,
    pub action: String,
    pub local: Document,
    pub remote: Option<RemoteDocument>,
    pub selected_markdown: String,
}

/// A valid single-hunk unified diff. It need not find a minimal edit script:
/// unchanged lines between edits may be repeated as removals and additions.
pub fn unified_diff(old: &str, new: &str, old_label: &str, new_label: &str) -> String {
    if old == new {
        return String::new();
    }
    let old_lines: Vec<_> = old.split_inclusive('\n').collect();
    let new_lines: Vec<_> = new.split_inclusive('\n').collect();
    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let start = prefix.saturating_sub(3);
    let old_end = (old_lines.len() - suffix + 3).min(old_lines.len());
    let new_end = (new_lines.len() - suffix + 3).min(new_lines.len());
    let old_count = old_end - start;
    let new_count = new_end - start;
    let mut out = format!(
        "--- {old_label}\n+++ {new_label}\n@@ -{},{} +{},{} @@\n",
        if old_count == 0 { start } else { start + 1 },
        old_count,
        if new_count == 0 { start } else { start + 1 },
        new_count
    );
    let mut line = |prefix: char, text: &str| {
        out.push(prefix);
        out.push_str(text);
        if !text.ends_with('\n') {
            out.push_str("\n\\ No newline at end of file\n");
        }
    };
    for text in &old_lines[start..prefix] {
        line(' ', text);
    }
    for text in &old_lines[prefix..old_lines.len() - suffix] {
        line('-', text);
    }
    for text in &new_lines[prefix..new_lines.len() - suffix] {
        line('+', text);
    }
    for text in &old_lines[old_lines.len() - suffix..old_end] {
        line(' ', text);
    }
    out
}
