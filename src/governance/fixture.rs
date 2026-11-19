//! A governed fixture tree a test writes for itself (larry#270): every file `larry governance`
//! reads, built from the constants below, so no unit test depends on the live checkout's
//! documents. The retained receipts are copied from the product, which owns their pinned
//! identity. The live tree is still checked by CI's `governance` job and by `larry governance`
//! at pre-commit.

use std::fs;
use std::io;
use std::path::Path;

/// How many requirements the fixture declares.
const REQUIREMENTS: usize = 70;
/// How many decisions the fixture declares.
const DECISIONS: usize = 35;
/// How many falsifiers the fixture declares.
const FALSIFIERS: usize = 54;
/// How many execution cells the fixture declares, from `C00`.
const CELLS: usize = 13;
/// How many axes the fixture declares.
const AXES: usize = 9;
/// How many falsifiers the log's crash plan names; those rows say "kill".
const CRASH_CASES: usize = 6;
/// The nine governance anchors `AGENTS.md` must name.
const ANCHORS: &str = "# Larry agent rules\n\nLogical-DB, Sentinel, Secure-Authority, Nova, Moo, \
Logicalworks-Crates, Self-maintenance, memory and NOT RUN are the governance anchors.\n";
/// The whole log: one line per inventory, and the crash plan the checker reads.
const LOG: &str = "# Larry log\n\nrequirements: 70\ndecisions: 35\nfalsifiers: 54\n\
execution_cells: 13\naxes: 9\nplanned_crash_cases: E01 E02 E03 E04 E05 E06\n";
/// The specification index; the rows live in its parts under `docs/specification/`.
const SPECIFICATION: &str = "# Larry specification\n\nThe parts of this document hold the \
requirements, decisions, falsifiers, cells and axes.\n";
/// The tracker template; the page is its render, which the caller writes.
const TEMPLATE: &str = "<!doctype html>\n<script>const tracker = __TRACKER_DATA__;</script>\n";

/// Writes a complete governed fixture tree under `root`, without `tools/tracker/index.html`:
/// the caller writes that page with the product's one renderer (`larry tracker` or
/// `tracker::write`), so the fixture never carries a second one. `retained` is the product's
/// `tools/receipts` directory, copied whole so the checker's pinned identity keeps one home.
///
/// # Errors
/// Returns the first failure to create a directory, read a retained receipt or write a file.
pub(crate) fn governed(root: &Path, retained: &Path) -> io::Result<()> {
    for (name, text) in documents() {
        write(root, name, &text)?;
    }
    for (name, text) in parts() {
        write(root, &name, &text)?;
    }
    copy_receipts(root, retained)?;
    write(root, "tools/tracker/tracker.json", &tracker_data())?;
    write(root, "tools/tracker/template.html", TEMPLATE)
}

/// The seven top-level documents, in the checker's load order.
fn documents() -> [(&'static str, String); 7] {
    [
        (
            "README.md",
            document(
                "README",
                "# Larry readme\n\nThe governed fixture readme opens [the specification](/specification.md).\n",
            ),
        ),
        (
            "index.md",
            document("index", "# Larry index\n\nSee [the readme](/README.md).\n"),
        ),
        (
            "research-ledger.md",
            document(
                "research-ledger",
                "# Larry research ledger\n\nThe fixture claims no source.\n",
            ),
        ),
        ("AGENTS.md", document("AGENTS", ANCHORS)),
        (
            "GOVERNANCE.md",
            document(
                "GOVERNANCE",
                "# Larry governance\n\nThe fixture governs itself.\n",
            ),
        ),
        ("log.md", document("log", LOG)),
        ("specification.md", document("specification", SPECIFICATION)),
    ]
}

/// The specification's parts, each under 120 lines so every file meets the size law.
fn parts() -> Vec<(String, String)> {
    vec![
        (
            String::from("docs/specification/part-01-requirements.md"),
            requirements(),
        ),
        (
            String::from("docs/specification/part-02-decisions.md"),
            decisions(),
        ),
        (
            String::from("docs/specification/part-03-falsifiers.md"),
            falsifiers(),
        ),
        (String::from("docs/specification/part-04-cells.md"), cells()),
    ]
}

/// A document: its frontmatter and its body.
fn document(title: &str, body: &str) -> String {
    format!("{}{body}", frontmatter(title))
}

/// The frontmatter envelope every fixture document carries: the ten keys the checker requires.
fn frontmatter(title: &str) -> String {
    format!(
        "---\ntype: Document\ntitle: Larry {title}\ndescription: A governed fixture document.\n\
         resource: /{title}.md\ntags: [larry, fixture]\n\
         generated: {{by: fixture, at: 2026-11-16}}\n\
         verified: {{by: fixture, method: the checker}}\nstatus: active\n\
         stale_after: 2026-12-16\nsources: []\n---\n"
    )
}

/// The requirement rows: each names a decision and a falsifier, as the checker requires.
fn requirements() -> String {
    let rows: Vec<String> = (1..=REQUIREMENTS)
        .map(|number| format!("| R{number:02} | Requirement {number} | D01 | E01 |\n"))
        .collect();
    document(
        "requirements",
        &format!("# Larry requirements\n\n{}", rows.join("")),
    )
}

/// The decision headings, `D01` to `D35`.
fn decisions() -> String {
    let rows: Vec<String> = (1..=DECISIONS)
        .map(|number| format!("### D{number:02} - Decision {number}\n"))
        .collect();
    document(
        "decisions",
        &format!("# Larry decisions\n\n{}", rows.join("")),
    )
}

/// The falsifier rows; the first six are the crash plan's and name the fault they inject.
fn falsifiers() -> String {
    let rows: Vec<String> = (1..=FALSIFIERS)
        .map(|number| {
            let body = if number <= CRASH_CASES {
                "Kill the run at its journal sync."
            } else {
                "The run returns its exact refusal."
            };
            format!("| E{number:02} | {body} |\n")
        })
        .collect();
    document(
        "falsifiers",
        &format!("# Larry falsifiers\n\n{}", rows.join("")),
    )
}

/// Cell `C00` owns every falsifier; the other cells follow; then the axes, the proof obligations
/// and four diagrams.
fn cells() -> String {
    let owned: Vec<String> = (1..=FALSIFIERS)
        .map(|number| format!("E{number:02}"))
        .collect();
    let mut lines: Vec<String> = vec![
        String::from("# Larry cells\n\n## Cell 11 — execution\n\n"),
        format!("| C00 | Owns every falsifier. | {} |\n", owned.join("/")),
    ];
    for number in 1..CELLS {
        lines.push(format!("| C{number:02} | Later work. | |\n"));
    }
    lines.push(String::from("\n## Cell 12 — closed\n\n"));
    for number in 1..=AXES {
        lines.push(format!("| A{number} | Axis {number}. |\n"));
    }
    lines.push(String::from(
        "\nThe proof obligations are P1 P2 P3 P4 P5 P6 P7 P8.\n\n",
    ));
    for label in ["start", "middle", "join", "end"] {
        lines.push(format!(
            "```mermaid\nflowchart TD\n  {label} --> next\n```\n"
        ));
    }
    document("cells", &lines.join(""))
}

/// Copies the product's retained receipts into the fixture, creating `tools/receipts`.
fn copy_receipts(root: &Path, retained: &Path) -> io::Result<()> {
    let target = root.join("tools").join("receipts");
    fs::create_dir_all(&target)?;
    for entry in fs::read_dir(retained)? {
        let entry = entry?;
        let name = entry.file_name();
        let kind = entry.file_type()?;
        // A directory or a link is never a receipt; anything else must copy or fail loudly.
        if !kind.is_dir() && !kind.is_symlink() && name.to_string_lossy().ends_with(".json") {
            fs::copy(entry.path(), target.join(name))?;
        }
    }
    Ok(())
}

/// The tracker data: one cell per execution cell, in spec order, and the falsifier total.
fn tracker_data() -> String {
    let cells: Vec<String> = (0..CELLS)
        .map(|number| format!("{{\"id\":\"C{number:02}\"}}"))
        .collect();
    format!(
        "{{\"falsifier_total\":{FALSIFIERS},\"cells\":[{}],\"falsifiers\":[]}}",
        cells.join(",")
    )
}

/// Writes `text` at `relative` under `root`, creating its parents.
fn write(root: &Path, relative: &str, text: &str) -> io::Result<()> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)
}
