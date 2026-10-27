//! The model's plan for a walk, read off its opening answer: what the change
//! is for, the concerns it is made of and the files of each, the order the
//! files are best read in, how risky each is, and which are safe to skim.
//! Every walk mode orders the same files from it.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub fn word(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Concern {
    pub title: String,
    pub why: Option<String>,
    /// Paths of the change, each in one concern only.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub summary: Option<String>,
    pub concerns: Vec<Concern>,
    /// Every path of the change, the ones the model put in order first.
    pub order: Vec<String>,
    pub risk: HashMap<String, (Risk, String)>,
    pub skim: HashMap<String, String>,
    pub watch: Vec<String>,
    /// Lines that fit no part of the shape, kept so nothing said is lost.
    pub rest: Vec<String>,
}

/// The concern a file the model left out of every concern lands in.
pub const EVERYTHING_ELSE: &str = "Everything else";

/// The plan `text` gives for a change of `paths`, in path order. A path the
/// change does not have is dropped; one the answer missed still lands in a
/// concern and in the order, so the walk never loses a file.
pub fn read(text: &str, paths: &[&str]) -> Plan {
    let mut plan = Plan::default();
    let mut section = Section::None;
    let mut ordered: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = without_bullet(raw);
        if line.is_empty() {
            continue;
        }
        if let Some(summary) = after_key(line, "summary") {
            plan.summary = Some(summary.to_string());
            section = Section::None;
        } else if let Some(title) = after_key(line, "concern") {
            plan.concerns.push(Concern {
                title: title.to_string(),
                why: None,
                paths: Vec::new(),
            });
            section = Section::Concern;
        } else if let Some(why) = after_key(line, "why") {
            if let Some(concern) = plan.concerns.last_mut() {
                concern.why = Some(why.to_string());
            }
        } else if let Some(files) = after_key(line, "files") {
            if let Some(concern) = plan.concerns.last_mut() {
                concern.paths.extend(listed(files, paths));
            }
            section = Section::Concern;
        } else if let Some(order) = after_key(line, "order") {
            ordered.extend(listed(order, paths));
            section = Section::Order;
        } else if let Some(risk) = after_key(line, "risk") {
            if let Some((path, rest)) = path_first(risk, paths)
                && let Some((risk, why)) = risk_of(rest)
            {
                plan.risk.insert(path, (risk, why));
            }
            section = Section::None;
        } else if let Some(skim) = after_key(line, "skim") {
            if let Some((path, why)) = path_first(skim, paths) {
                plan.skim.insert(path, why.to_string());
            }
            section = Section::None;
        } else if let Some(watch) = after_key(line, "watch") {
            plan.watch.push(watch.to_string());
            section = Section::None;
        } else {
            // A bare path under `Files:` or `Order:` belongs to that list,
            // and one the change does not have is dropped like any other.
            let found = listed(line, paths);
            let bare_path = !line.contains(char::is_whitespace);
            match section {
                Section::Concern if !found.is_empty() || bare_path => {
                    if let Some(concern) = plan.concerns.last_mut() {
                        concern.paths.extend(found);
                    }
                }
                Section::Order if !found.is_empty() || bare_path => ordered.extend(found),
                _ => plan.rest.push(raw.to_string()),
            }
        }
    }
    settle(&mut plan, ordered, paths);
    plan
}

#[derive(Clone, Copy)]
enum Section {
    None,
    Concern,
    Order,
}

/// Each path in one concern and once in the order, and every path somewhere.
fn settle(plan: &mut Plan, ordered: Vec<String>, paths: &[&str]) {
    let mut placed = std::collections::HashSet::new();
    for concern in &mut plan.concerns {
        concern.paths.retain(|path| placed.insert(path.clone()));
    }
    plan.concerns.retain(|concern| !concern.paths.is_empty());
    let left: Vec<String> = paths
        .iter()
        .filter(|path| !placed.contains(**path))
        .map(|path| path.to_string())
        .collect();
    if !left.is_empty() {
        plan.concerns.push(Concern {
            title: EVERYTHING_ELSE.to_string(),
            why: None,
            paths: left,
        });
    }
    let mut seen = std::collections::HashSet::new();
    plan.order = ordered
        .into_iter()
        .chain(paths.iter().map(|path| path.to_string()))
        .filter(|path| seen.insert(path.clone()))
        .collect();
}

/// The change's paths named in `text`, split on commas or whitespace.
fn listed(text: &str, paths: &[&str]) -> Vec<String> {
    text.split([',', ' ', '\t'])
        .filter_map(|word| resolve(word, paths))
        .collect()
}

/// `path rest`, `path: rest` or `path — rest`, with the path resolved.
fn path_first<'a>(text: &'a str, paths: &[&str]) -> Option<(String, &'a str)> {
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let word = text[..end].trim_end_matches(':');
    let path = resolve(word, paths)?;
    let rest = text[end..]
        .trim_start()
        .trim_start_matches([':', '\u{2014}', '\u{2013}', '-'])
        .trim();
    Some((path, rest))
}

/// A word the model wrote for a path: the path itself, or the one path of the
/// change that ends with it.
fn resolve(word: &str, paths: &[&str]) -> Option<String> {
    let word = word.trim_matches(['`', '*', '"', '\'', '(', ')', '.', ':', ';']);
    if word.is_empty() {
        return None;
    }
    if let Some(path) = paths.iter().find(|path| **path == word) {
        return Some(path.to_string());
    }
    let mut ends = paths
        .iter()
        .filter(|path| path.ends_with(&format!("/{word}")));
    match (ends.next(), ends.next()) {
        (Some(path), None) => Some(path.to_string()),
        _ => None,
    }
}

fn risk_of(text: &str) -> Option<(Risk, String)> {
    let end = text
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let risk = match text[..end].to_ascii_lowercase().as_str() {
        "high" => Risk::High,
        "medium" | "med" => Risk::Medium,
        "low" => Risk::Low,
        _ => return None,
    };
    let why = text[end..]
        .trim_start_matches([':', ' ', '\u{2014}', '\u{2013}', '-'])
        .trim();
    Some((risk, why.to_string()))
}

fn without_bullet(line: &str) -> &str {
    let line = line.trim();
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("\u{2022} "))
        .unwrap_or(line)
        .trim_start_matches('*')
        .trim()
}

fn after_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let head = line.get(..key.len())?;
    if !head.eq_ignore_ascii_case(key) {
        return None;
    }
    let rest = line[key.len()..].trim_start_matches('*');
    Some(rest.strip_prefix(':')?.trim_start_matches('*').trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATHS: [&str; 5] = [
        "Cargo.lock",
        "src/cache.rs",
        "src/lookup.rs",
        "tests/cache.rs",
        "README.md",
    ];

    const ANSWER: &str = "\
Summary: Adds a cache in front of lookup.
Concern: The cache itself
Why: Keeps the last 100 lookups.
Files: src/cache.rs, `lookup.rs`
Concern: Tests for eviction
Files:
- tests/cache.rs
- src/cache.rs
- src/gone.rs
Order: tests/cache.rs, src/cache.rs
Risk: src/cache.rs high: eviction runs under the lock
Risk: `tests/cache.rs` low \u{2014} only adds cases
Skim: Cargo.lock: dependency bump
Watch: the lock held across the callback in src/cache.rs
Something that fits no part of the shape.
";

    #[test]
    fn every_file_lands_in_one_concern_and_the_order_has_them_all() {
        let plan = read(ANSWER, &PATHS);

        let concerns: Vec<(&str, Vec<&str>)> = plan
            .concerns
            .iter()
            .map(|c| {
                (
                    c.title.as_str(),
                    c.paths.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            concerns,
            [
                ("The cache itself", vec!["src/cache.rs", "src/lookup.rs"]),
                ("Tests for eviction", vec!["tests/cache.rs"]),
                (EVERYTHING_ELSE, vec!["Cargo.lock", "README.md"]),
            ]
        );
        assert_eq!(
            plan.concerns[0].why.as_deref(),
            Some("Keeps the last 100 lookups.")
        );
        assert_eq!(&plan.order[..2], ["tests/cache.rs", "src/cache.rs"]);
        let mut all = plan.order.clone();
        all.sort();
        let mut expected = PATHS.map(String::from).to_vec();
        expected.sort();
        assert_eq!(all, expected);
    }

    #[test]
    fn risk_skim_and_what_to_watch_are_read_per_file() {
        let plan = read(ANSWER, &PATHS);

        assert_eq!(
            plan.risk.get("src/cache.rs"),
            Some(&(Risk::High, "eviction runs under the lock".to_string()))
        );
        assert_eq!(
            plan.risk.get("tests/cache.rs").map(|(risk, _)| *risk),
            Some(Risk::Low)
        );
        assert!(plan.skim.contains_key("Cargo.lock"));
        assert_eq!(
            plan.summary.as_deref(),
            Some("Adds a cache in front of lookup.")
        );
        assert_eq!(plan.watch.len(), 1);
        assert_eq!(plan.rest, ["Something that fits no part of the shape."]);
    }

    #[test]
    fn an_answer_with_no_plan_still_walks_every_file() {
        let plan = read("I could not read the diff.", &PATHS);

        assert_eq!(plan.concerns.len(), 1);
        assert_eq!(plan.concerns[0].paths.len(), PATHS.len());
        assert_eq!(plan.order, PATHS.map(String::from));
    }
}
