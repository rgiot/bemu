//! The command palette's search: every command, matched by its id, its label and its menus.

use super::{CommandSpec, COMMANDS};

/// How well `query` matches `text`: its characters in order (a subsequence), better when they
/// start words and follow each other; `None` when they are not all there.
fn score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = (at..text.len()).find(|&i| text[i] == q)?;
        let word_start = found == 0 || !text[found - 1].is_alphanumeric();
        score += if word_start { 8 } else { 1 };
        if previous == Some(found.wrapping_sub(1)) {
            score += 5;
        }
        // matched early is better
        score -= (found - at).min(10) as i32 / 4;
        previous = Some(found);
        at = found + 1;
    }
    Some(score)
}

/// What the palette searches in a command.
fn haystack(spec: &CommandSpec) -> String {
    format!("{} {} {}", spec.path.join(" "), spec.id, spec.aim)
}

/// The commands matching `query`, the best first; `recent` (the commands run lately, the last
/// first) come first when the query is empty, and score higher otherwise.
pub fn search(query: &str, recent: &[&str]) -> Vec<&'static CommandSpec> {
    let query = query.trim();
    let mut found: Vec<(i32, usize, &'static CommandSpec)> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(order, spec)| {
            let recency = recent.iter().position(|id| *id == spec.id);
            let score = if query.is_empty() {
                Some(0)
            } else {
                // the label and the id are what people type: they count most
                let label = score(query, &format!("{} {}", spec.label(), spec.id)).map(|s| s + 20);
                label.max(score(query, &haystack(spec)))
            }?;
            // first when nothing is typed; with a query, a tie-breaker only
            let bonus = match (recency, query.is_empty()) {
                (Some(r), true) => 40 - r as i32,
                (Some(_), false) => 2,
                (None, _) => 0,
            };
            Some((score + bonus, order, spec))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, _, spec)| spec).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_ids_are_found_by_their_letters() {
        assert_eq!(search("crtc 1", &[])[0].id, "machine-crtc-1");
        assert_eq!(search("flip b", &[])[0].id, "drive-b-flip-disc");
        assert_eq!(search("snapshot-load", &[])[0].id, "snapshot-load");
        assert!(search("zzzzzz", &[]).is_empty());
    }

    #[test]
    fn recent_commands_come_first() {
        assert_eq!(search("", &["help-about"])[0].id, "help-about");
        // but not before what the query names
        assert_eq!(
            search("set pc", &["view-disassembly"])[0].id,
            "debug-set-pc"
        );
    }
}
