//! Fixed-case tests of the write guards (larry#123); the seeded simulations live
//! in `sim_author.rs`. Source: CWE-59 (link following). Requirement: a symlink in
//! the clone is never written through, and a fenced reply is written as its body.

use std::error::Error;
use std::fs;

use super::super::{Ledger, Settings, digest_hex, identity};
use super::model_step;
use super::sim::scratch;
use super::{AuthorFault, checked_target, file_text};
use crate::executor::ExecError;
use crate::executor::Task;
use crate::model::{Message, ModelError, Role};

/// What every test returns.
type TestResult = Result<(), Box<dyn Error>>;

#[cfg(unix)]
#[test]
fn a_symlink_anywhere_on_the_path_is_never_written_through() -> TestResult {
    let tree = scratch("tree")?;
    let outside = scratch("outside")?;
    std::os::unix::fs::symlink(&outside, tree.join("escape"))?;
    std::os::unix::fs::symlink(outside.join("victim"), tree.join("leaf"))?;
    for path in ["escape/x.rs", "escape", "leaf"] {
        assert!(
            matches!(
                checked_target(&tree, path),
                Err(ExecError::Author {
                    fault: AuthorFault::Symlink,
                    ..
                })
            ),
            "{path} crosses a symlink"
        );
    }
    let fresh = checked_target(&tree, "src/new/mod.rs")?;
    assert_eq!(fresh, tree.join("src/new/mod.rs"), "a new path is joined");
    assert_eq!(
        fs::read_dir(&outside)?.count(),
        0,
        "nothing was written outside the tree"
    );
    fs::remove_dir_all(&tree)?;
    fs::remove_dir_all(&outside)?;
    Ok(())
}

#[test]
fn a_fenced_reply_is_written_as_its_body_with_a_final_newline() {
    assert_eq!(file_text("```rust\nfn a() {}\n```"), "fn a() {}\n");
    assert_eq!(file_text("fn a() {}"), "fn a() {}\n", "a newline is added");
    assert_eq!(file_text("```"), "```\n", "a lone fence is kept as text");
    assert_eq!(
        file_text("```rust\nopen"),
        "```rust\nopen\n",
        "an unclosed fence is kept"
    );
    assert_eq!(file_text(""), "", "an empty reply stays empty");
}

/// A brief task spends from the executor's shared tenant-route limiter: once
/// its tenant's token is spent, the next completion is refused before any
/// gateway byte. A step built on its own limiter would answer Transport from
/// the dead gateway instead (larry#204).
#[test]
fn a_brief_task_is_refused_once_its_tenant_spends_its_budget() -> TestResult {
    let state = scratch("brief-budget")?;
    let state_dir = state.join("state");
    fs::create_dir_all(&state_dir)?;
    let table = [
        ("LARRY_STATE_DIR", state_dir.to_string_lossy().into_owned()),
        ("PATH", String::from("/usr/bin:/bin")),
        ("LARRY_MODEL_URL", String::from("http://127.0.0.1:9/v1")),
        ("LARRY_MODEL", String::from("spent-model")),
        ("LARRY_MODEL_KEY", String::from("spent-key")),
        ("LARRY_MODEL_TOKEN_BUDGET", String::from("1")),
    ];
    let settings = Settings::from_env(move |name: &str| {
        table
            .iter()
            .find(|entry| entry.0 == name)
            .map(|entry| entry.1.clone())
    })?;
    let ledger = Ledger::open(&state_dir)?;
    let task = Task::parse(
        format!(
            r#"{{"repo":"{}","base":"HEAD","goal":"write the note","files":["note.txt"],"acceptance":[["true"]]}}"#,
            state_dir.display()
        )
        .as_bytes(),
    )?;
    let Some(step) = model_step(&task, &settings, &ledger)? else {
        return Err("a brief task builds a model step".into());
    };
    let brief = || Message::new(Role::User, "over budget");
    assert!(
        matches!(
            step.client.complete(&[brief()]),
            Err(ModelError::Transport(_))
        ),
        "the dead gateway answers Transport while the budget is whole"
    );
    let tenant = digest_hex(identity::tenant_of(&state_dir).as_bytes());
    settings.limiter.record(&tenant, "env", 0, 0, 1, 0)?;
    assert!(
        matches!(
            step.client.complete(&[brief()]),
            Err(ModelError::BudgetExceeded {
                tokens: 1,
                limit: 1
            })
        ),
        "the spent tenant is refused before any gateway byte"
    );
    fs::remove_dir_all(&state)?;
    Ok(())
}
