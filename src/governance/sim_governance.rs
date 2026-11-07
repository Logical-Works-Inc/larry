//! Seeded controls of `larry governance`'s inventory, log, crash-plan and anchor rules over the
//! real documents, in memory only. Each ports one control of the Python suite the checker
//! replaced (`tests/test_governance.py`, larry#53), named in its doc line. The Python control
//! mutated one fixed place; each simulation here mutates a seeded place, so the rule is shown
//! for the whole inventory. Source: GOVERNANCE.md, "What the executable gates check".
//! Requirement: every mutation is refused with its exact error, and every legal change passes.

use std::fmt::Write as _;

use lgwks_std::seeded::Seeded;

use super::documents::Documents;
use super::patterns::{INVENTORIES, Inventory, Patterns};
use super::sim_support::{
    SimError, SimResult, assert_refused, count_of, errors_of, loaded, pick, replace_once, text_of,
    top_level,
};
use super::{ANCHORS, execution_section};

/// Seeds each control runs; each seed validates the whole tree once.
const SEEDS: u64 = 16;

/// Every ASCII-case-insensitive byte offset of `needle` in `text`.
fn occurrences(text: &str, needle: &str) -> Vec<usize> {
    let lower = text.to_ascii_lowercase();
    lower
        .match_indices(&needle.to_ascii_lowercase())
        .map(|(offset, _)| offset)
        .collect()
}

/// `text` with the `len` bytes at each offset replaced by `apply` of the original slice.
fn rewrite_at(
    text: &str,
    offsets: &[usize],
    len: usize,
    apply: &mut dyn FnMut(&str) -> String,
) -> Result<String, SimError> {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for &offset in offsets {
        let end = offset.saturating_add(len);
        let (Some(before), Some(found)) = (text.get(cursor..offset), text.get(offset..end)) else {
            return Err(SimError::Absent(String::from("a character boundary")));
        };
        out.push_str(before);
        out.push_str(&apply(found));
        cursor = end;
    }
    out.push_str(
        text.get(cursor..)
            .ok_or_else(|| SimError::Absent(String::from("a tail")))?,
    );
    Ok(out)
}

/// The marker text that starts a row of inventory `prefix` for `id`.
fn row_marker(prefix: &str, id: &str) -> String {
    match prefix {
        "D" => format!("### {id} "),
        "A" => format!("| {id} "),
        _ => format!("| {id} |"),
    }
}

/// The row-rename a control makes: from the ID it picks to the text it writes in its place.
type RowChoice = dyn Fn(&mut Seeded, &[String]) -> Result<(String, String), SimError>;

/// Renames one row of a seeded inventory as `choose` decides; returns the inventory, the
/// renamed ID and the errors `validate` then reports.
fn renamed_row(
    rng: &mut Seeded,
    base: &Documents,
    patterns: &Patterns,
    choose: &RowChoice,
) -> Result<(&'static Inventory, String, Vec<String>), SimError> {
    let spec = text_of(base, "specification.md")?;
    let index = rng.index(INVENTORIES.len())?;
    let (Some(inventory), Some(row)) = (INVENTORIES.get(index), patterns.rows.get(index)) else {
        return Err(SimError::Absent(String::from("an inventory")));
    };
    let ids: Vec<String> = patterns
        .groups(row, &spec, 1)
        .into_iter()
        .map(String::from)
        .collect();
    let (from, to) = choose(rng, &ids)?;
    let mut documents = base.clone();
    replace_once(
        &mut documents,
        "specification.md",
        &row_marker(inventory.prefix, &from),
        &row_marker(inventory.prefix, &to),
    )?;
    Ok((inventory, from, errors_of(&documents)?))
}

/// Ports `test_missing_document`: dropping any top-level document names it.
#[test]
fn sim_missing_document_is_named_for_every_seed() -> SimResult {
    let base = loaded()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let mut documents = base.clone();
        let name = pick(&mut rng, &top_level())?.clone();
        documents.remove(&name);
        let errors = errors_of(&documents)?;
        assert!(
            errors.contains(&format!("missing document: {name}")),
            "seed {seed}: {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_anchor_case_does_not_change_meaning`: re-casing every occurrence of an anchor
/// letter by letter never loses it.
#[test]
fn sim_anchor_case_does_not_change_meaning() -> SimResult {
    let base = loaded()?;
    let agents = text_of(&base, "AGENTS.md")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let anchor = *pick(&mut rng, &ANCHORS)?;
        let mut flips = Vec::new();
        for _ in 0..anchor.len().saturating_mul(4) {
            flips.push(rng.below(2)? == 1);
        }
        let mut flip = flips.into_iter().cycle();
        let recased = rewrite_at(
            &agents,
            &occurrences(&agents, anchor),
            anchor.len(),
            &mut |found| {
                found
                    .chars()
                    .map(|character| match flip.next() {
                        Some(true) => character.to_ascii_uppercase(),
                        Some(false) | None => character.to_ascii_lowercase(),
                    })
                    .collect()
            },
        )?;
        let mut documents = base.clone();
        documents.set("AGENTS.md", recased);
        let errors = errors_of(&documents)?;
        assert!(
            !errors
                .iter()
                .any(|error| error.contains("missing governance anchor")),
            "seed {seed}: {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_missing_owner_anchor`: removing every occurrence of any anchor names it.
#[test]
fn sim_missing_owner_anchor_is_named() -> SimResult {
    let base = loaded()?;
    let agents = text_of(&base, "AGENTS.md")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let anchor = *pick(&mut rng, &ANCHORS)?;
        let stripped = rewrite_at(
            &agents,
            &occurrences(&agents, anchor),
            anchor.len(),
            &mut |_| String::from("unnamed owner"),
        )?;
        let mut documents = base.clone();
        documents.set("AGENTS.md", stripped);
        assert_refused(
            &errors_of(&documents)?,
            &format!("AGENTS.md: missing governance anchor {anchor}"),
            seed,
        );
    }
    Ok(())
}

/// Ports `test_removed_requirement`, for every inventory: a removed row breaks the inventory, or
/// for its last row the log count it must repeat.
#[test]
fn sim_removed_inventory_row_is_refused() -> SimResult {
    let base = loaded()?;
    let patterns = Patterns::new()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let remove = |rng: &mut Seeded, ids: &[String]| -> Result<(String, String), SimError> {
            Ok((pick(rng, ids)?.clone(), String::from("removed")))
        };
        let (inventory, id, errors) = renamed_row(&mut rng, &base, &patterns, &remove)?;
        let label = inventory.label;
        let broken = errors
            .iter()
            .any(|error| error.starts_with(&format!("{label}: incomplete")))
            || errors
                .iter()
                .any(|error| error.starts_with(&format!("log inventory drift: {label},")));
        assert!(
            broken,
            "seed {seed}: removing {id} must break {label}: {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_duplicate_requirement`, for every inventory: a row renamed to another ID's is refused.
#[test]
fn sim_duplicate_inventory_row_is_refused() -> SimResult {
    let base = loaded()?;
    let patterns = Patterns::new()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let duplicate = |rng: &mut Seeded, ids: &[String]| -> Result<(String, String), SimError> {
            let kept = pick(rng, ids)?.clone();
            let others: Vec<String> = ids.iter().filter(|id| **id != kept).cloned().collect();
            Ok((pick(rng, &others)?.clone(), kept))
        };
        let (inventory, _renamed, errors) = renamed_row(&mut rng, &base, &patterns, &duplicate)?;
        let expected = format!(
            "{}: incomplete, duplicate or noncontiguous inventory",
            inventory.label
        );
        assert_refused(&errors, &expected, seed);
    }
    Ok(())
}

/// Every requirement row of `spec`, owned.
fn requirement_rows(spec: &str, patterns: &Patterns) -> Vec<String> {
    patterns
        .matches(&patterns.requirement_row, spec)
        .into_iter()
        .map(String::from)
        .collect()
}

/// Ports `test_undefined_decision`: a requirement row citing a decision nobody defined is named.
#[test]
fn sim_undefined_decision_is_named_exactly() -> SimResult {
    let base = loaded()?;
    let patterns = Patterns::new()?;
    let spec = text_of(&base, "specification.md")?;
    let rows = requirement_rows(&spec, &patterns);
    let decisions = count_of(&base, "decisions")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let row = pick(&mut rng, &rows)?;
        let cited = patterns.matches(&patterns.decision, row);
        let old = *pick(&mut rng, &cited)?;
        let undefined = format!(
            "D{:02}",
            decisions.saturating_add(1).saturating_add(rng.index(800)?)
        );
        let mut documents = base.clone();
        documents.set(
            "specification.md",
            spec.replacen(row.as_str(), &row.replacen(old, &undefined, 1), 1),
        );
        let errors = errors_of(&documents)?;
        let expected = format!("specification.md: undefined D references ['{undefined}']");
        assert!(
            errors.contains(&expected),
            "seed {seed}: expected {expected:?} in {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_missing_acceptance_mapping`: a requirement row stripped of its decisions or its
/// falsifiers is reported with its new text.
#[test]
fn sim_missing_acceptance_mapping_is_reported_with_its_row() -> SimResult {
    let base = loaded()?;
    let patterns = Patterns::new()?;
    let spec = text_of(&base, "specification.md")?;
    let rows = requirement_rows(&spec, &patterns);
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let row = pick(&mut rng, &rows)?;
        let regex = if rng.below(2)? == 0 {
            &patterns.decision
        } else {
            &patterns.falsifier
        };
        let mut stripped = row.clone();
        for token in patterns.matches(regex, row) {
            stripped = stripped.replace(token, "none");
        }
        let mut documents = base.clone();
        documents.set(
            "specification.md",
            spec.replacen(row.as_str(), &stripped, 1),
        );
        let errors = errors_of(&documents)?;
        assert!(
            errors.contains(&format!("unmapped requirement: {stripped}")),
            "seed {seed}: {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_stale_inventory`, for every label: a log count that drifts or repeats is refused.
#[test]
fn sim_stale_log_inventory_is_refused() -> SimResult {
    let base = loaded()?;
    let log = text_of(&base, "log.md")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let label = pick(&mut rng, &INVENTORIES)?.label;
        let count = count_of(&base, label)?;
        let line = format!("\n{label}: {count}\n");
        let stale = match rng.below(3)? {
            0 => format!(
                "\n{label}: {}\n",
                count.saturating_add(1).saturating_add(rng.index(5)?)
            ),
            1 => format!("\n{label}: {}\n", count.saturating_sub(1)),
            _ => format!("{line}{label}: {count}\n"),
        };
        let mut documents = base.clone();
        documents.set("log.md", log.replacen(&line, &stale, 1));
        assert_refused(
            &errors_of(&documents)?,
            &format!("log inventory drift: {label}, observed {count}"),
            seed,
        );
    }
    Ok(())
}

/// `section` with every whole-token occurrence of `id` replaced by `Exx`, which no ID pattern reads.
fn without_token(section: &str, id: &str) -> Result<String, SimError> {
    let offsets: Vec<usize> = section
        .match_indices(id)
        .map(|(offset, _)| offset)
        .filter(|&offset| {
            let after = section
                .get(offset.saturating_add(id.len())..)
                .and_then(|rest| rest.chars().next());
            !after.is_some_and(|character| character.is_ascii_digit())
        })
        .collect();
    rewrite_at(section, &offsets, id.len(), &mut |_| String::from("Exx"))
}

/// One seeded owned falsifier unallocated from the execution section, with or without a prose
/// mention of it, checked against the errors: the case the two Python allocation controls built.
fn unallocated_control(mention: bool) -> SimResult {
    let base = loaded()?;
    let patterns = Patterns::new()?;
    let spec = text_of(&base, "specification.md")?;
    let section =
        execution_section(&spec).ok_or_else(|| SimError::Absent(String::from("Cell 11")))?;
    let owned: Vec<String> = patterns
        .groups(&patterns.owns, section, 1)
        .into_iter()
        .flat_map(|cases| cases.split('/'))
        .map(String::from)
        .collect();
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let id = pick(&mut rng, &owned)?.clone();
        let mut changed = spec.replacen(section, &without_token(section, &id)?, 1);
        if mention {
            changed = changed.replacen(
                "## Cell 12",
                &format!("Unallocated case: {id}.\n\n## Cell 12"),
                1,
            );
        }
        let mut documents = base.clone();
        documents.set("specification.md", changed);
        let errors = errors_of(&documents)?;
        let named = errors.iter().any(|error| {
            error.starts_with("unallocated execution cases") && error.contains(&format!("'{id}'"))
        });
        assert!(named, "seed {seed}: {id} must be unallocated: {errors:?}");
    }
    Ok(())
}

/// Ports `test_missing_execution_case_allocation`: a falsifier no cell owns is unallocated.
#[test]
fn sim_missing_execution_case_allocation_is_named() -> SimResult {
    unallocated_control(false)
}

/// Ports `test_unallocated_mention_is_not_assignment`: naming the case in prose does not own it.
#[test]
fn sim_unallocated_mention_is_not_assignment() -> SimResult {
    unallocated_control(true)
}

/// The log's planned crash line and its cases.
fn crash_line(log: &str) -> Result<(String, Vec<String>), SimError> {
    let line = log
        .lines()
        .find(|line| line.starts_with("planned_crash_cases: "))
        .ok_or_else(|| SimError::Absent(String::from("planned_crash_cases")))?;
    let cases = line
        .trim_start_matches("planned_crash_cases: ")
        .split_whitespace()
        .map(String::from)
        .collect();
    Ok((String::from(line), cases))
}

/// `count` distinct seeded falsifier IDs out of `E01..=E{total}`.
fn distinct_falsifiers(
    rng: &mut Seeded,
    total: usize,
    count: usize,
) -> Result<Vec<String>, SimError> {
    let mut all: Vec<String> = (1..=total).map(|number| format!("E{number:02}")).collect();
    let mut chosen = Vec::with_capacity(count);
    for _ in 0..count.min(all.len()) {
        let index = rng.index(all.len())?;
        chosen.push(all.swap_remove(index));
    }
    Ok(chosen)
}

/// Ports `test_insufficient_crash_cases`: fewer distinct cases than a tenth of the falsifiers.
#[test]
fn sim_insufficient_crash_cases_are_refused() -> SimResult {
    let base = loaded()?;
    let log = text_of(&base, "log.md")?;
    let (line, _cases) = crash_line(&log)?;
    let falsifiers = count_of(&base, "falsifiers")?;
    let ceiling = falsifiers
        .checked_div(10)
        .ok_or_else(|| SimError::Absent(String::from("falsifiers")))?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let count = rng.index(ceiling.max(1))?.max(1);
        let cases = distinct_falsifiers(&mut rng, falsifiers, count)?;
        let mut documents = base.clone();
        documents.set(
            "log.md",
            log.replacen(
                &line,
                &format!("planned_crash_cases: {}", cases.join(" ")),
                1,
            ),
        );
        assert_refused(
            &errors_of(&documents)?,
            "planned crash fraction below 10 percent",
            seed,
        );
    }
    Ok(())
}

/// Ports `test_duplicate_crash_case`: a repeated or undefined case invalidates the plan.
#[test]
fn sim_duplicate_or_unknown_crash_case_is_refused() -> SimResult {
    let base = loaded()?;
    let log = text_of(&base, "log.md")?;
    let (line, cases) = crash_line(&log)?;
    let falsifiers = count_of(&base, "falsifiers")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let mut changed = cases.clone();
        if rng.below(2)? == 0 {
            changed.push(pick(&mut rng, &cases)?.clone());
        } else {
            let unknown = format!(
                "E{:02}",
                falsifiers.saturating_add(1).saturating_add(rng.index(50)?)
            );
            let slot = rng.index(changed.len())?;
            if let Some(case) = changed.get_mut(slot) {
                *case = unknown;
            }
        }
        let mut documents = base.clone();
        documents.set(
            "log.md",
            log.replacen(
                &line,
                &format!("planned_crash_cases: {}", changed.join(" ")),
                1,
            ),
        );
        assert_refused(
            &errors_of(&documents)?,
            "invalid planned crash inventory",
            seed,
        );
    }
    Ok(())
}

/// The reference fault test: a crash, kill or panic word, by word split, case-insensitive.
fn names_a_fault(text: &str) -> bool {
    const WORDS: [&str; 12] = [
        "crash",
        "crashes",
        "crashed",
        "crashing",
        "kill",
        "kills",
        "killed",
        "killing",
        "panic",
        "panics",
        "panicked",
        "panicking",
    ];
    text.to_lowercase()
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .any(|word| WORDS.contains(&word))
}

/// The body of the last falsifier row of `id`: the text after `| id | `, as Python's `dict` kept.
fn falsifier_body<'spec>(spec: &'spec str, id: &str) -> Option<&'spec str> {
    let marker = format!("| {id} | ");
    spec.split('\n')
        .filter_map(|line| line.strip_prefix(marker.as_str()))
        .rfind(|body| !body.is_empty())
}

/// Ports `test_skill_is_not_a_kill`: a case is accepted only when its own row names a fault,
/// judged against a word-split reference.
#[test]
fn sim_skill_is_not_a_kill() -> SimResult {
    let base = loaded()?;
    let log = text_of(&base, "log.md")?;
    let spec = text_of(&base, "specification.md")?;
    let (line, _cases) = crash_line(&log)?;
    let falsifiers = count_of(&base, "falsifiers")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let count = rng.index(5)?.saturating_add(8);
        let cases = distinct_falsifiers(&mut rng, falsifiers, count)?;
        let mut documents = base.clone();
        documents.set(
            "log.md",
            log.replacen(
                &line,
                &format!("planned_crash_cases: {}", cases.join(" ")),
                1,
            ),
        );
        let errors = errors_of(&documents)?;
        for case in &cases {
            let lacking = !falsifier_body(&spec, case).is_some_and(names_a_fault);
            let reported =
                errors.contains(&format!("planned crash case lacks fault action: {case}"));
            assert_eq!(
                reported, lacking,
                "seed {seed}: {case} fault verdict must match the reference"
            );
        }
    }
    Ok(())
}

/// Ports `test_inventory_can_grow`: appended mapped rows with a matching log count pass.
#[test]
fn sim_inventory_can_grow() -> SimResult {
    let base = loaded()?;
    let spec = text_of(&base, "specification.md")?;
    let log = text_of(&base, "log.md")?;
    let requirements = count_of(&base, "requirements")?;
    let decisions = count_of(&base, "decisions")?;
    let falsifiers = count_of(&base, "falsifiers")?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let mut grown = spec.clone();
        let added = rng.index(3)?.saturating_add(1);
        for step in 1..=added {
            let number = requirements.saturating_add(step);
            let decision = rng.index(decisions)?.saturating_add(1);
            let falsifier = rng.index(falsifiers)?.saturating_add(1);
            write!(
                grown,
                "\n| R{number:02} | extension | Additional mapped requirement | D{decision:02} | E{falsifier:02} |\n"
            )?;
        }
        let total = requirements.saturating_add(added);
        let mut documents = base.clone();
        documents.set("specification.md", grown);
        documents.set(
            "log.md",
            log.replacen(
                &format!("\nrequirements: {requirements}\n"),
                &format!("\nrequirements: {total}\n"),
                1,
            ),
        );
        let errors = errors_of(&documents)?;
        assert!(
            errors.is_empty(),
            "seed {seed}: a grown inventory must pass: {errors:?}"
        );
        assert_eq!(
            count_of(&documents, "requirements")?,
            total,
            "seed {seed}: the count follows the rows"
        );
    }
    Ok(())
}
