//! Adversarial controls of the pieces `validate` reads a section with (larry#554): the heading
//! search, the execution section and its allocation, the crash plan and the report body.
//! Source: GOVERNANCE.md, "What the executable gates check", and the Python checker larry#53
//! replaced. Requirement: a lookalike heading, a second crash line, a repeated case or an
//! unknown case is read exactly as the checker names it.

use std::collections::BTreeSet;

use lgwks_std::hash;

use super::inventory::inspect_inventories;
use super::sim_support::{SimResult, loaded, whole_of, with_patterns};
use super::{
    crash_cases, crash_errors, execution_allocated, execution_section, find_heading, report_body,
};

/// The IDs named, as the owned set `execution_allocated` returns.
fn ids(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| String::from(*name)).collect()
}

/// A heading counts only when no word character follows it, and the section runs from the first
/// `## Cell 11` to the first `## Cell 12` after it.
#[test]
fn sim_a_lookalike_heading_never_bounds_the_execution_section() {
    let found = |text| find_heading(text, "## Cell 11", 0);
    assert_eq!(found("## Cell 11\n"), Some(0));
    assert_eq!(found("## Cell 11"), Some(0));
    assert_eq!(found("## Cell 110\n## Cell 11.\n"), Some(12));
    assert_eq!(found("## Cell 11x\n"), None);
    assert_eq!(found("## Cell 11_\n"), None);
    assert_eq!(found("## Cell 11é\n"), None);
    assert_eq!(found(""), None);
    assert_eq!(
        find_heading("## Cell 11\n## Cell 11\n", "## Cell 11", 1),
        Some(11)
    );
    assert_eq!(find_heading("short", "## Cell 11", 99), None);

    assert_eq!(execution_section("no cells"), None);
    assert_eq!(execution_section("## Cell 11\n"), None);
    assert_eq!(execution_section("## Cell 12\n## Cell 11\n"), None);
    assert_eq!(execution_section("## Cell 11\n## Cell 120\n"), None);
    assert_eq!(
        execution_section("a\n## Cell 11\nb\n## Cell 12\n"),
        Some("## Cell 11\nb\n")
    );
}

/// A cell allocates the falsifiers in its row's last column and the list it says it owns, and
/// nothing outside the section or in another column.
#[test]
fn sim_only_a_cells_own_column_and_list_allocate_a_falsifier() -> SimResult {
    let spec = "| C09 | before | E09 |\n## Cell 11\n| C00 | E08 in prose | E01 E02 |\nC01 owns E03/E04. C02 also owns E05; E06 is only named.\n## Cell 12\n| C10 | after | E10 |\nC03 owns E11.\n";
    with_patterns(|patterns| {
        assert_eq!(
            execution_allocated(spec, patterns),
            ids(&["E01", "E02", "E03", "E04", "E05"])
        );
        assert_eq!(execution_allocated("no cells", patterns), ids(&[]));
        Ok(())
    })?;
    Ok(())
}

/// The crash plan is one line; none or two read as an empty plan. A repeated, unknown or empty
/// plan is invalid, a tenth of the falsifiers is the floor, and each case must name a fault.
#[test]
fn sim_a_doctored_crash_plan_is_refused_by_each_rule() -> SimResult {
    const INVALID: &str = "invalid planned crash inventory";
    const FRACTION: &str = "planned crash fraction below 10 percent";
    let mut spec = String::new();
    for number in 1..=60 {
        let action = if number <= 6 {
            "Kill the process"
        } else {
            "Read a skill"
        };
        let row = format!("| E{number:02} | {action} |\n");
        spec.push_str(&row);
    }
    with_patterns(|patterns| {
        let split = crash_cases("planned_crash_cases: E01  E02\n", patterns);
        assert_eq!(split, ["E01", "E02"]);
        for log in [
            "",
            "planned_crash_cases: E01\nplanned_crash_cases: E02\n",
            "  planned_crash_cases: E01\n",
        ] {
            assert_eq!(crash_cases(log, patterns).len(), 0, "{log:?}");
        }

        let inventories = inspect_inventories(&spec, patterns, &mut Vec::new());
        let judged = |cases: &[&str]| {
            let owned: Vec<String> = cases.iter().map(|case| String::from(*case)).collect();
            let mut errors = Vec::new();
            crash_errors(&spec, &owned, &inventories, patterns, &mut errors);
            errors
        };
        assert_eq!(
            judged(&["E01", "E02", "E03", "E04", "E05", "E06"]),
            Vec::<String>::new()
        );
        assert_eq!(judged(&["E01", "E02", "E03", "E04", "E05"]), [FRACTION]);
        assert_eq!(
            judged(&["E01", "E02", "E03", "E04", "E05", "E05", "E06"]),
            [INVALID]
        );
        assert_eq!(judged(&[]), [INVALID, FRACTION]);
        assert_eq!(
            judged(&["E01", "E02", "E03", "E04", "E05", "E99"]),
            [
                String::from(INVALID),
                String::from("planned crash case lacks fault action: E99")
            ]
        );
        assert_eq!(
            judged(&["E01", "E02", "E03", "E04", "E05", "E07"]),
            ["planned crash case lacks fault action: E07"]
        );
        Ok(())
    })?;
    Ok(())
}

/// The report body names its scope, every count, the crash plan and the BLAKE3 digest of each
/// document's exact bytes.
#[test]
fn sim_the_report_body_digests_the_documents_it_judged() -> SimResult {
    let documents = loaded()?;
    let spec = whole_of(&documents, "specification.md")?;
    let readme = documents.get("README.md").ok_or("README.md")?;
    let digest = hash::blake3(readme.as_bytes()).to_hex();
    let crash = [String::from("E01")];
    with_patterns(|patterns| {
        let inventories = inspect_inventories(&spec, patterns, &mut Vec::new());
        let body = report_body(&documents, &inventories, &crash);
        assert_eq!(
            body.get("scope").and_then(|value| value.as_str()),
            Some("document structure only; runtime and human results NOT RUN")
        );
        for &(label, count) in &inventories.counts {
            let recorded = body
                .get("counts")
                .and_then(|counts| counts.get(label))
                .and_then(|value| value.as_u64());
            assert_eq!(recorded, u64::try_from(count).ok(), "{label}");
        }
        assert_eq!(
            body.get("planned_crash_cases"),
            Some(&lgwks_std::json::json!(["E01"]))
        );
        assert_eq!(
            body.get("blake3")
                .and_then(|digests| digests.get("README.md"))
                .and_then(|value| value.as_str()),
            Some(digest.as_str())
        );
        Ok(())
    })?;
    Ok(())
}
