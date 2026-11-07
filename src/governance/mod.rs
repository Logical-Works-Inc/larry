//! `larry governance`: the read-only structural checker over the governed documents, the Rust
//! port of the Python checker larry#53 replaced (ADR 0010). It checks document structure only,
//! never runtime behaviour.

pub mod tracker;

mod documents;
mod error;
mod inventory;
mod markdown;
mod patterns;
mod pyjson;
mod pystr;
pub(crate) mod receipts;

#[cfg(test)]
mod sim_governance;
#[cfg(test)]
mod sim_markdown;
#[cfg(test)]
mod sim_parsers;
#[cfg(test)]
pub(crate) mod sim_support;
#[cfg(test)]
mod sim_tracker;
#[cfg(test)]
mod tests;

pub use tracker::TrackerError;

use std::collections::BTreeSet;
use std::path::Path;

use lgwks_std::hash;
use lgwks_std::json::{self, Map, Value};

use documents::Documents;
use error::GovernanceError;
use inventory::Inventories;
use markdown::HeadingCache;
use patterns::Patterns;

/// The governance anchors AGENTS.md must name, case-insensitively.
const ANCHORS: [&str; 9] = [
    "Logical-DB",
    "Sentinel",
    "Secure-Authority",
    "Nova",
    "Moo",
    "Logicalworks-Crates",
    "Self-maintenance",
    "memory",
    "NOT RUN",
];

/// The result of one governance check: a JSON object of the Python report shape, with BLAKE3
/// document digests under `blake3`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Report {
    /// The report value: `scope`, `counts`, `planned_crash_cases`, `blake3`, `errors` and `ok`.
    value: Value,
}

impl Report {
    /// Whether the check passed with no error.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.value.get("ok").and_then(Value::as_bool) == Some(true)
    }

    /// Every error the check raised, in order.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        match self.value.get("errors").and_then(Value::as_array) {
            Some(items) => items
                .iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect(),
            None => Vec::new(),
        }
    }

    /// The full report as `json.dumps(report, indent=2, sort_keys=True)`, the `--report` output.
    #[must_use]
    pub fn pretty(&self) -> String {
        pyjson::dumps(&self.value, Some(2))
    }

    /// The counts as `json.dumps(counts, sort_keys=True)`, the line a passing run prints.
    #[must_use]
    pub fn counts_line(&self) -> String {
        match self.value.get("counts") {
            Some(counts) => pyjson::dumps(counts, None),
            None => String::from("{}"),
        }
    }
}

/// Runs every governance check against the repository at `root`; a refusal to load or parse
/// becomes the whole report, `{"ok": false, "errors": [refusal]}`, as the Python `main` made it.
#[must_use]
pub fn check(root: &Path) -> Report {
    let value = match run(root) {
        Ok(value) => value,
        Err(error) => json::json!({"ok": false, "errors": [error.to_string()]}),
    };
    Report { value }
}

/// Loads, validates, then adds the receipt and tracker errors and recomputes `ok`.
fn run(root: &Path) -> Result<Value, GovernanceError> {
    let documents = documents::load(root)?;
    let patterns = Patterns::new()?;
    let mut errors = Vec::new();
    let mut report = validate(&documents, &patterns, &mut errors)?;
    errors.extend(receipts::receipt_errors(root));
    errors.extend(tracker::drift_errors(
        root,
        &documents.whole("specification.md"),
        &patterns,
    ));
    report.insert("ok", Value::from(errors.is_empty()));
    report.insert("errors", Value::from(errors));
    Ok(Value::Object(report))
}

/// `validate(documents)` for in-memory documents: the report body and every error, as the
/// Python controls called it.
#[cfg(test)]
fn validated(documents: &Documents) -> Result<(Map, Vec<String>), GovernanceError> {
    let patterns = Patterns::new()?;
    let mut errors = Vec::new();
    let body = validate(documents, &patterns, &mut errors)?;
    Ok((body, errors))
}

/// Port of `validate`: the document-structure and inventory checks, in the Python order. Errors
/// are pushed onto `errors`; the report carries everything else.
fn validate(
    documents: &Documents,
    patterns: &Patterns,
    errors: &mut Vec<String>,
) -> Result<Map, GovernanceError> {
    for name in documents::DOCUMENTS {
        if documents.get(name).is_none() {
            errors.push(format!("missing document: {name}"));
        }
    }
    let mut headings = HeadingCache::new();
    for (name, text) in documents.iter() {
        errors.extend(markdown::document_errors(
            name,
            text,
            documents,
            patterns,
            &mut headings,
        )?);
    }
    let whole_spec = documents.whole("specification.md");
    let spec = whole_spec.as_str();
    let inventories = inventory::inspect_inventories(spec, patterns, errors);
    errors.extend(inventory::traceability_errors(
        documents,
        &inventories,
        patterns,
    ));
    let defined_e = inventories.defined("E");
    let allocated = execution_allocated(spec, patterns);
    let unallocated: Vec<String> = defined_e.difference(&allocated).cloned().collect();
    if !unallocated.is_empty() {
        errors.push(format!(
            "unallocated execution cases: {}",
            patterns.classes.list(&unallocated)
        ));
    }
    let whole_log = documents.whole("log.md");
    let log = whole_log.as_str();
    for (&(label, count), regex) in inventories.counts.iter().zip(&patterns.log_lines) {
        if patterns.groups(regex, log, 1) != [count.to_string()] {
            errors.push(format!("log inventory drift: {label}, observed {count}"));
        }
    }
    let crash = crash_cases(log, patterns);
    crash_errors(spec, &crash, &inventories, patterns, errors);
    let mermaid = markdown::markdown_structure(spec, patterns)
        .languages()
        .iter()
        .filter(|language| language.as_str() == "mermaid")
        .count();
    if mermaid < 4 {
        errors.push(String::from("specification lacks required visual diagrams"));
    }
    let proof: BTreeSet<&str> = patterns
        .matches(&patterns.proof, spec)
        .into_iter()
        .collect();
    let expected_proof: BTreeSet<String> = (1..=8).map(|index| format!("P{index}")).collect();
    if !proof
        .iter()
        .copied()
        .eq(expected_proof.iter().map(String::as_str))
    {
        errors.push(String::from("missing proof obligation IDs"));
    }
    let agents = documents.text("AGENTS.md");
    for anchor in ANCHORS {
        if !pystr::casefold_contains(agents, anchor) {
            errors.push(format!("AGENTS.md: missing governance anchor {anchor}"));
        }
    }
    Ok(report_body(documents, &inventories, &crash))
}

/// The report keys other than `errors` and `ok`.
fn report_body(documents: &Documents, inventories: &Inventories, crash: &[String]) -> Map {
    let counts: Map = inventories
        .counts
        .iter()
        .map(|&(label, count)| (String::from(label), Value::from(count)))
        .collect();
    let digests: Map = documents
        .sorted()
        .map(|(name, text)| {
            (
                String::from(name),
                Value::from(hash::blake3(text.as_bytes()).to_hex()),
            )
        })
        .collect();
    let mut body = Map::new();
    body.insert(
        "scope",
        Value::from("document structure only; runtime and human results NOT RUN"),
    );
    body.insert("counts", Value::Object(counts));
    body.insert("planned_crash_cases", Value::from(crash.to_vec()));
    body.insert("blake3", Value::Object(digests));
    body
}

/// The E-IDs the execution cells allocate. `re.search(r"## Cell 11\b.*?(?=## Cell 12\b)", spec,
/// re.S)`: no section unless a `## Cell 12` heading follows the first `## Cell 11`.
fn execution_allocated(spec: &str, patterns: &Patterns) -> BTreeSet<String> {
    let mut allocated = BTreeSet::new();
    let Some(section) = execution_section(spec) else {
        return allocated;
    };
    for row in patterns.matches(&patterns.cell_row, section) {
        let segments: Vec<&str> = row.split('|').collect();
        let Some(segment) = segments
            .len()
            .checked_sub(2)
            .and_then(|index| segments.get(index))
        else {
            continue;
        };
        allocated.extend(
            patterns
                .matches(&patterns.falsifier, segment)
                .into_iter()
                .map(String::from),
        );
    }
    for cases in patterns.groups(&patterns.owns, section, 1) {
        allocated.extend(cases.split('/').map(String::from));
    }
    allocated
}

/// The text from the first `## Cell 11` up to the first `## Cell 12` after it, each followed by
/// a Python word boundary; `None` when either heading is absent.
fn execution_section(spec: &str) -> Option<&str> {
    let start = find_heading(spec, "## Cell 11", 0)?;
    let end = find_heading(spec, "## Cell 12", start)?;
    spec.get(start..end)
}

/// The byte offset of the first `heading` at or after `from` followed by a non-word character
/// or the end of the text.
fn find_heading(text: &str, heading: &str, from: usize) -> Option<usize> {
    let mut search = from;
    loop {
        let position = search.checked_add(text.get(search..)?.find(heading)?)?;
        let after = position.checked_add(heading.len())?;
        let boundary = match text.get(after..).and_then(|rest| rest.chars().next()) {
            Some(character) => !(character.is_alphanumeric() || character == '_'),
            None => true,
        };
        if boundary {
            return Some(position);
        }
        search = after;
    }
}

/// `crash_lines[0].split() if len(crash_lines) == 1 else []`.
fn crash_cases(log: &str, patterns: &Patterns) -> Vec<String> {
    match *patterns.groups(&patterns.crash_line, log, 1).as_slice() {
        [only] => pystr::split(only).into_iter().map(String::from).collect(),
        _ => Vec::new(),
    }
}

/// The crash-plan checks: the list is valid, covers 10 percent of falsifiers, and each case
/// names a fault action in its falsifier row (the last row of that ID, as Python's `dict` keeps).
fn crash_errors(
    spec: &str,
    crash: &[String],
    inventories: &Inventories,
    patterns: &Patterns,
    errors: &mut Vec<String>,
) {
    let distinct: BTreeSet<&String> = crash.iter().collect();
    let defined_e = inventories.defined("E");
    if crash.is_empty()
        || distinct.len() != crash.len()
        || !crash.iter().all(|case| defined_e.contains(case))
    {
        errors.push(String::from("invalid planned crash inventory"));
    }
    if distinct.len().saturating_mul(10) < inventories.count("falsifiers") {
        errors.push(String::from("planned crash fraction below 10 percent"));
    }
    let ids = patterns.groups(&patterns.falsifier_row, spec, 1);
    let bodies = patterns.groups(&patterns.falsifier_row, spec, 2);
    for case in crash {
        let body = ids
            .iter()
            .zip(&bodies)
            .filter(|&(&id, _)| id == case.as_str())
            .map(|(_, &body)| body)
            .next_back();
        let faulted = match body {
            Some(text) => patterns.is_match(&patterns.fault, text),
            None => patterns.is_match(&patterns.fault, ""),
        };
        if !faulted {
            errors.push(format!("planned crash case lacks fault action: {case}"));
        }
    }
}
