//! Reading the rule tables in `docs/rules/*.md`.
//!
//! A table row is `| id | rule | source | check | sev |`. The id's prefix
//! names the pack (`RC` rust-core, `ST` structure, `CM` comments, `TS` tests,
//! `SE` security, `CS` change shape). A row that does not parse is an error,
//! so a malformed rule never silently disappears from a review.

use std::{collections::BTreeSet, fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::Error;

/// How a rule is checked.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    /// The compiler or Clippy finds it.
    Lint,
    /// A grep, ast-grep or xtask-style script finds it.
    Script,
    /// A tool such as cargo-deny, cargo-machete or a secrets scanner finds it.
    Tool,
    /// It needs judgment; a finding must carry evidence.
    Agent,
}

/// Ordered from most to least serious.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Nit,
}

impl Severity {
    /// # Errors
    /// Returns the text when it names no severity.
    pub fn parse(text: &str) -> Result<Self, String> {
        // "high for fix PRs" and similar notes keep their first word.
        match text
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "critical" => Ok(Self::Critical),
            "high" => Ok(Self::High),
            "medium" => Ok(Self::Medium),
            "low" => Ok(Self::Low),
            "nit" => Ok(Self::Nit),
            other => Err(format!("{other:?} is not a severity")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Rule {
    pub id: String,
    pub pack: String,
    pub rule: String,
    pub source: String,
    /// Every way the rule can be checked, in the order the table lists them.
    pub checks: Vec<CheckKind>,
    /// The check column as written, which holds the patterns and tool names.
    pub how: String,
    pub severity: Severity,
}

fn kinds(text: &str) -> Vec<CheckKind> {
    let lower = text.to_ascii_lowercase();
    let mut found: Vec<(usize, CheckKind)> = Vec::new();
    for (word, kind) in [
        ("lint", CheckKind::Lint),
        ("script", CheckKind::Script),
        ("tool", CheckKind::Tool),
        ("agent", CheckKind::Agent),
    ] {
        if let Some(position) = lower.find(word) {
            found.push((position, kind));
        }
    }
    found.sort();
    found.into_iter().map(|(_, kind)| kind).collect()
}

fn cells(line: &str) -> Vec<String> {
    // A cell may hold an escaped bar (`\|`); the table writes it that way.
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = line.trim().trim_matches('|').chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            current.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(c);
        }
    }
    cells.push(current.trim().to_owned());
    cells
}

fn is_rule_id(text: &str) -> bool {
    let mut parts = text.split('-');
    let (Some(prefix), Some(number), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    prefix.len() == 2
        && prefix.bytes().all(|b| b.is_ascii_uppercase())
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
}

/// Read the rules in one pack's Markdown.
///
/// # Errors
/// Returns an error for a rule row with the wrong number of cells, an unknown
/// severity or no check kind, or for an id that appears twice.
pub fn parse_pack(pack: &str, text: &str) -> Result<Vec<Rule>, Error> {
    let mut rules = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        if !line.trim_start().starts_with('|') {
            continue;
        }
        let row = cells(line);
        if !row.first().is_some_and(|id| is_rule_id(id)) {
            continue; // the header, the divider or a table that is not a rule table
        }
        let fail = |reason: String| Error::Row {
            pack: pack.to_owned(),
            line: index + 1,
            reason,
        };
        if row.len() != 5 {
            return Err(fail(format!("{} cells, need 5", row.len())));
        }
        let checks = kinds(&row[3]);
        if checks.is_empty() {
            return Err(fail(format!("{:?} names no check kind", row[3])));
        }
        if !seen.insert(row[0].clone()) {
            return Err(fail(format!("id {} appears twice", row[0])));
        }
        rules.push(Rule {
            id: row[0].clone(),
            pack: pack.to_owned(),
            rule: row[1].clone(),
            source: row[2].clone(),
            checks,
            how: row[3].clone(),
            severity: Severity::parse(&row[4]).map_err(fail)?,
        });
    }
    Ok(rules)
}

/// Read every `*.md` pack in a folder, in file-name order.
///
/// # Errors
/// Returns an error if the folder cannot be read, a pack does not parse, or an
/// id is used in two packs.
pub fn load_dir(directory: &Path) -> Result<Vec<Rule>, Error> {
    let mut files: Vec<_> = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    files.sort();
    let mut all: Vec<Rule> = Vec::new();
    for path in files {
        let pack = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = fs::read_to_string(&path)?;
        for rule in parse_pack(&pack, &text)? {
            if all.iter().any(|known| known.id == rule.id) {
                return Err(Error::Invalid(format!("rule id {} is used twice", rule.id)));
            }
            all.push(rule);
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
| id | rule | source | check | sev |
|---|---|---|---|---|
| RC-01 | No panics. | SC | lint + script (`unwrap`) | high |
| RC-02 | Escaped \\| bar kept. | SC | script (`a \\| b`) + lint | high for fix PRs |
";

    // Invariant: the table's columns land in the right fields, the severity
    // note keeps its first word, an escaped bar stays inside its cell and the
    // check kinds keep the table's order. A parser that split on every bar
    // would fail the second row.
    #[test]
    fn rows_become_typed_rules() {
        let rules = parse_pack("rust-core", TABLE).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].checks, vec![CheckKind::Lint, CheckKind::Script]);
        assert_eq!(rules[1].severity, Severity::High);
        assert!(rules[1].rule.contains('|'));
        assert_eq!(rules[1].checks, vec![CheckKind::Script, CheckKind::Lint]);
    }

    // Invariant: a row that cannot be read is an error, never a missing rule.
    #[test]
    fn a_bad_row_is_an_error_and_a_repeated_id_too() {
        let short = "| RC-01 | only | three |\n";
        assert!(parse_pack("p", short).is_err());
        let sev = "| RC-01 | r | s | lint | shouting |\n";
        assert!(parse_pack("p", sev).is_err());
        let twice = "| RC-01 | r | s | lint | low |\n| RC-01 | r | s | lint | low |\n";
        assert!(parse_pack("p", twice).is_err());
    }

    // Invariant: the real packs in the repository all parse. This catches a
    // table edit that breaks the review pipeline's input.
    #[test]
    fn the_repository_rule_packs_parse() {
        let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/rules");
        let rules = load_dir(&docs).unwrap();
        assert!(rules.len() > 40, "{}", rules.len());
    }
}
