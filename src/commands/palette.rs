//! The command palette's entries and search, as VS Code's: each command by its name, its id
//! and what it does. The radio items of a family (CRTC 0 to 4, the speeds, the renderings...)
//! are one command whose value is chosen in a second step.

use std::sync::OnceLock;

use super::{CommandSpec, Kind, COMMANDS};

/// A family of radio items: one command of the palette, its value chosen in a second step.
#[derive(Debug)]
pub struct Choice {
    /// The family's id, the items' common prefix (`machine-crtc` for `machine-crtc-0`...).
    pub id: String,
    /// Its name (`CRTC`, `Drive A: Kind`).
    pub label: String,
    /// What it does.
    pub aim: String,
    /// Its values, in menu order.
    pub items: Vec<&'static CommandSpec>,
}

/// An entry of the palette.
#[derive(Clone, Copy, Debug)]
pub enum Entry {
    Command(&'static CommandSpec),
    Choice(&'static Choice),
}

impl Entry {
    pub fn id(&self) -> &str {
        match self {
            Entry::Command(spec) => spec.id,
            Entry::Choice(choice) => &choice.id,
        }
    }

    /// Its name in the palette: the menu item's, its menu first when another command has the
    /// same (`View: CRTC`, `Drive B: Insert disc image`), as VS Code's categories.
    pub fn label(&self) -> &str {
        match self {
            Entry::Command(spec) => names()
                .iter()
                .find(|(id, _)| *id == spec.id)
                .map_or(spec.label(), |(_, name)| name.as_str()),
            Entry::Choice(choice) => &choice.label,
        }
    }

    pub fn aim(&self) -> &str {
        match self {
            Entry::Command(spec) => spec.aim,
            Entry::Choice(choice) => &choice.aim,
        }
    }

    pub fn shortcut(&self) -> Option<&'static str> {
        match self {
            Entry::Command(spec) => spec.shortcut,
            Entry::Choice(_) => None,
        }
    }

    /// The text searched: the name, the id, what it does (and a family's values).
    fn haystack(&self) -> String {
        match self {
            Entry::Command(spec) => format!("{} {} {}", spec.label(), spec.id, spec.aim),
            Entry::Choice(choice) => {
                let values: Vec<String> = choice
                    .items
                    .iter()
                    .map(|item| format!("{} {}", item.label(), item.id))
                    .collect();
                format!(
                    "{} {} {} {}",
                    choice.label,
                    choice.id,
                    choice.aim,
                    values.join(" ")
                )
            }
        }
    }
}

/// The palette's names of the commands whose menu item's name is not unique.
fn names() -> &'static [(&'static str, String)] {
    static NAMES: OnceLock<Vec<(&'static str, String)>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut taken: Vec<String> = choices().iter().map(|c| c.label.clone()).collect();
        taken.extend(
            COMMANDS
                .iter()
                .filter(|spec| spec.kind != Kind::Radio)
                .map(|spec| spec.label().to_owned()),
        );
        COMMANDS
            .iter()
            .filter(|spec| spec.kind != Kind::Radio)
            .filter(|spec| taken.iter().filter(|name| *name == spec.label()).count() > 1)
            .map(|spec| {
                let menu = spec.menus().last().copied().unwrap_or_default();
                (spec.id, format!("{menu}: {}", spec.label()))
            })
            .collect()
    })
}

/// The id without its last `-word` (`machine-crtc` of `machine-crtc-1`).
fn family(id: &str) -> &str {
    id.rsplit_once('-').map_or(id, |(family, _)| family)
}

/// The labels' common start, up to a `: ` (`Character set` of `Character set: USA`).
fn common_label(items: &[&CommandSpec]) -> Option<String> {
    let first = items.first()?.label();
    let (prefix, _) = first.split_once(": ")?;
    items
        .iter()
        .all(|item| item.label().starts_with(&format!("{prefix}: ")))
        .then(|| prefix.to_owned())
}

/// The radio families, built once from the registry.
pub fn choices() -> &'static [Choice] {
    static CHOICES: OnceLock<Vec<Choice>> = OnceLock::new();
    CHOICES.get_or_init(|| {
        let mut choices: Vec<Choice> = Vec::new();
        for spec in COMMANDS.iter().filter(|spec| spec.kind == Kind::Radio) {
            let id = family(spec.id);
            match choices
                .iter_mut()
                .find(|choice| choice.id == id && choice.items[0].menus() == spec.menus())
            {
                Some(choice) => choice.items.push(spec),
                None => choices.push(Choice {
                    id: id.to_owned(),
                    label: String::new(),
                    aim: String::new(),
                    items: vec![spec],
                }),
            }
        }
        let families_in = |menus: &[&str], choices: &[Choice]| {
            choices
                .iter()
                .filter(|c| c.items[0].menus() == menus)
                .count()
        };
        let counts: Vec<usize> = choices
            .iter()
            .map(|c| families_in(c.items[0].menus(), &choices))
            .collect();
        for (choice, shared) in choices.iter_mut().zip(counts) {
            let menus = choice.items[0].menus();
            let place = menus[menus.len().min(2) - 1..].join(": ");
            choice.label = match (common_label(&choice.items), menus.len(), shared) {
                (Some(common), _, _) => format!("{}: {common}", menus.last().unwrap_or(&"")),
                // Audio > Stereo / Mono
                (None, 1, _) => choice
                    .items
                    .iter()
                    .map(|item| item.label())
                    .collect::<Vec<_>>()
                    .join(" / "),
                (None, _, 1) => place,
                (None, _, _) => {
                    let word = choice.id.rsplit('-').next().unwrap_or_default();
                    format!("{place}: {word}")
                }
            };
            let values: Vec<&str> = choice.items.iter().map(|item| item.label()).collect();
            choice.aim = format!("Choose: {}", values.join(", "));
        }
        choices
    })
}

/// Every entry: the commands, a radio family as one entry.
pub fn entries() -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut seen: Vec<*const Choice> = Vec::new();
    for spec in COMMANDS {
        if spec.kind != Kind::Radio {
            entries.push(Entry::Command(spec));
            continue;
        }
        if let Some(choice) = choices()
            .iter()
            .find(|choice| choice.items.iter().any(|item| item.id == spec.id))
        {
            if !seen.contains(&(choice as *const Choice)) {
                seen.push(choice);
                entries.push(Entry::Choice(choice));
            }
        }
    }
    entries
}

/// How well `query` matches `text`: its characters in order (a subsequence), better when they
/// start words and follow each other; `None` when they are not all there.
fn score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        // a letter far from the previous one is not part of the same search
        let end = if previous.is_some() {
            (at + 40).min(text.len())
        } else {
            text.len()
        };
        let found = (at..end).find(|&i| text[i] == q)?;
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

/// How well `query` matches an entry: its name and id count most.
fn entry_score(query: &str, entry: &Entry) -> Option<i32> {
    let name = score(query, &format!("{} {}", entry.label(), entry.id())).map(|s| s + 20);
    name.max(score(query, &entry.haystack()))
}

/// The entries matching `query`, the best first; `recent` (the commands run lately, the last
/// first) come first when the query is empty, and break ties otherwise.
pub fn search(query: &str, recent: &[&str]) -> Vec<Entry> {
    let query = query.trim();
    let mut found: Vec<(i32, usize, Entry)> = entries()
        .into_iter()
        .enumerate()
        .filter_map(|(order, entry)| {
            let recency = recent.iter().position(|id| match entry {
                Entry::Command(spec) => *id == spec.id,
                Entry::Choice(choice) => choice.items.iter().any(|item| item.id == *id),
            });
            let score = if query.is_empty() {
                0
            } else {
                entry_score(query, &entry)?
            };
            let bonus = match (recency, query.is_empty()) {
                (Some(r), true) => 40 - r as i32,
                (Some(_), false) => 2,
                (None, _) => 0,
            };
            Some((score + bonus, order, entry))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, _, entry)| entry).collect()
}

/// The values of `choice` matching `query`, in their order (the second step).
pub fn search_values(choice: &Choice, query: &str) -> Vec<&'static CommandSpec> {
    let query = query.trim();
    choice
        .items
        .iter()
        .copied()
        .filter(|item| {
            query.is_empty()
                || score(query, &format!("{} {} {}", item.label(), item.id, item.aim)).is_some()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(query: &str, recent: &[&str]) -> Vec<String> {
        search(query, recent)
            .iter()
            .map(|e| e.id().to_owned())
            .collect()
    }

    #[test]
    fn radio_families_are_one_command() {
        let crtc = choices().iter().find(|c| c.id == "machine-crtc").unwrap();
        assert_eq!(crtc.label, "CRTC");
        assert_eq!(crtc.items.len(), 5);
        let kind = choices().iter().find(|c| c.id == "drive-a").unwrap();
        assert_eq!(kind.label, "Drive A: Kind");
        let charset = choices()
            .iter()
            .find(|c| c.id == "printer-charset")
            .unwrap();
        assert_eq!(charset.label, "Printer: Character set");
        let layout = choices()
            .iter()
            .find(|c| c.id == "keyboard-host-layout")
            .unwrap();
        assert_eq!(layout.label, "Keyboard: Host layout");
        let mapping = choices()
            .iter()
            .find(|c| c.id == "keyboard-mapping")
            .unwrap();
        assert_eq!(mapping.label, "Keyboard: mapping");
        // no radio item is an entry of its own
        assert!(entries().iter().all(|e| match e {
            Entry::Command(spec) => spec.kind != Kind::Radio,
            Entry::Choice(_) => true,
        }));
    }

    #[test]
    fn names_ids_and_values_are_found_by_their_letters() {
        assert_eq!(ids("crtc", &[])[0], "machine-crtc");
        assert_eq!(ids("view: crtc", &[])[0], "view-crtc");
        assert_eq!(ids("crtc 1", &[])[0], "machine-crtc");
        assert_eq!(ids("flip b", &[])[0], "drive-b-flip-disc");
        assert_eq!(ids("snapshot-load", &[])[0], "snapshot-load");
        assert_eq!(ids("trinitron", &[])[0], "video-rendering");
        assert!(search("zzzzzz", &[]).is_empty());
        let name = |id: &str| {
            entries()
                .into_iter()
                .find(|e| e.id() == id)
                .map(|e| e.label().to_owned())
                .unwrap()
        };
        assert_eq!(name("view-crtc"), "View: CRTC");
        assert_eq!(name("drive-b-select-disc"), "Drive B: Insert disc image");
        assert_eq!(name("help-about"), "About");
        let crtc = choices().iter().find(|c| c.id == "machine-crtc").unwrap();
        let values: Vec<_> = search_values(crtc, "2").iter().map(|s| s.id).collect();
        assert_eq!(values, ["machine-crtc-2"]);
    }

    #[test]
    fn recent_commands_come_first() {
        assert_eq!(ids("", &["help-about"])[0], "help-about");
        assert_eq!(ids("", &["machine-crtc-1"])[0], "machine-crtc");
        // but not before what the query names
        assert_eq!(ids("set pc", &["view-disassembly"])[0], "debug-set-pc");
    }
}
