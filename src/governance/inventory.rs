//! Requirement, decision, falsifier, cell and axis inventory checks, ported from the Python
//! `inspect_inventories`, `traceability_errors` and the inventory half of `validate`. Each
//! inventory must be contiguous from its first ID, meet its minimum count and carry no duplicate.

use std::collections::{BTreeMap, BTreeSet};

use super::documents::Documents;
use super::patterns::{INVENTORIES, Patterns};

/// The counts and the defined IDs of every inventory.
pub(crate) struct Inventories {
    /// Count of matches per label, in the Python insertion order the log check walks.
    pub(crate) counts: Vec<(&'static str, usize)>,
    /// Defined IDs per prefix.
    defined: BTreeMap<&'static str, BTreeSet<String>>,
}

impl Inventories {
    /// The count of one label, `0` when absent.
    pub(crate) fn count(&self, label: &str) -> usize {
        for &(name, count) in &self.counts {
            if name == label {
                return count;
            }
        }
        0
    }

    /// The defined IDs of one prefix, the empty set when absent.
    pub(crate) fn defined(&self, prefix: &str) -> &BTreeSet<String> {
        match self.defined.get(prefix) {
            Some(ids) => ids,
            None => &NONE_DEFINED,
        }
    }
}

/// The defined set of a prefix no inventory declares.
static NONE_DEFINED: BTreeSet<String> = BTreeSet::new();

/// The expected ID for position `number` of an inventory with prefix `prefix`.
fn expected_id(prefix: &str, number: usize) -> String {
    if prefix == "A" {
        format!("A{number}")
    } else {
        format!("{prefix}{number:02}")
    }
}

/// Inspects every inventory in `spec`, pushing one error per violated inventory.
pub(crate) fn inspect_inventories(
    spec: &str,
    patterns: &Patterns,
    errors: &mut Vec<String>,
) -> Inventories {
    let mut counts = Vec::with_capacity(INVENTORIES.len());
    let mut defined = BTreeMap::new();
    for (inventory, row) in INVENTORIES.iter().zip(&patterns.rows) {
        let ids = patterns.groups(row, spec, 1);
        let actual: BTreeSet<String> = ids.iter().map(|id| String::from(*id)).collect();
        let expected: BTreeSet<String> = (inventory.start..)
            .take(ids.len())
            .map(|number| expected_id(inventory.prefix, number))
            .collect();
        if ids.len() < inventory.minimum || actual.len() != ids.len() || actual != expected {
            errors.push(format!(
                "{}: incomplete, duplicate or noncontiguous inventory",
                inventory.label
            ));
        }
        counts.push((inventory.label, ids.len()));
        defined.insert(inventory.prefix, actual);
    }
    Inventories { counts, defined }
}

/// Every undefined ID reference, and every requirement row that lacks a decision or falsifier.
pub(crate) fn traceability_errors(
    documents: &Documents,
    inventories: &Inventories,
    patterns: &Patterns,
) -> Vec<String> {
    let mut errors = Vec::new();
    for (name, text) in documents.iter() {
        for (inventory, reference) in INVENTORIES.iter().zip(&patterns.references) {
            let known = inventories.defined(inventory.prefix);
            let unresolved: BTreeSet<String> = patterns
                .matches(reference, text)
                .into_iter()
                .filter(|id| !known.contains(*id))
                .map(String::from)
                .collect();
            if !unresolved.is_empty() {
                let listed: Vec<String> = unresolved.into_iter().collect();
                errors.push(format!(
                    "{name}: undefined {} references {}",
                    inventory.prefix,
                    patterns.classes.list(&listed)
                ));
            }
        }
    }
    let spec = documents.whole("specification.md");
    for line in patterns.matches(&patterns.requirement_row, &spec) {
        if !patterns.is_match(&patterns.decision, line)
            || !patterns.is_match(&patterns.falsifier, line)
        {
            errors.push(format!("unmapped requirement: {line}"));
        }
    }
    errors
}
