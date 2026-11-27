//! The first user's journey through the shipped `larry` binary (larry#228): the
//! help he reads before he knows a subcommand, the refusal he gets when a task
//! document carries `acceptance` in the other door's shape, and the route error
//! that names its field once.
//!
//! Source: POSIX exit status (0 for a request answered, 1 for a refusal) and the
//! executor's own task document, which reads `acceptance` as argv arrays
//! (larry#85). Every case starts the real binary with a cleared environment, so
//! no developer variable reaches it and no case makes a paid model call.
//! Requirement proved: `larry help`, `--help` and `-h` exit 0 and print every
//! subcommand with the `LARRY_*` variables it reads; a wrong-typed `acceptance`
//! names the argv-array form with an example; `route add` errors carry one
//! `route` word. The MCP door's own form is proved in `tests/mcp_stdio_journey.rs`.

#[path = "support/larry_cli.rs"]
mod larry_cli;
#[path = "support/temp_dir.rs"]
mod temp_dir;

use std::error::Error;
use std::process::Output;

use larry_cli::{spawn_larry_args, stderr_of};
use temp_dir::TempDir;

/// What every journey returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Every subcommand the binary dispatches, as its arms name them (larry#228).
/// A new arm that help forgets fails `help_lists_every_subcommand_with_its_env`.
const SUBCOMMANDS: [&str; 13] = [
    "help",
    "mcp",
    "acp",
    "model",
    "exec",
    "verify",
    "key",
    "route",
    "eval",
    "governance",
    "tracker",
    "mascot",
    "serve",
];

/// The `LARRY_*` variables a user must be able to find in help: one per command
/// that reads one, and the two wildcards the executor and the routes use.
const VARIABLES: [&str; 12] = [
    "LARRY_STATE_DIR",
    "LARRY_POLICY",
    "LARRY_EXEC_TIMEOUT_MS",
    "LARRY_FENCE_CONTROL",
    "LARRY_MODEL_URL",
    "LARRY_MODEL_KEY",
    "LARRY_RECEIPT_KEY",
    "LARRY_SERVE_ADDR",
    "LARRY_SERVE_PLAINTEXT",
    "LARRY_EVAL_PRICE_IN",
    "LARRY_EVAL_PRICE_OUT",
    "LARRY_KEY_*",
];

/// Runs `larry <word>` to its end with no environment beyond PATH.
fn run_word(word: &str) -> Result<Output, Box<dyn Error>> {
    Ok(spawn_larry_args(&[word], &[], b"")?.wait_with_output()?)
}

/// The first word of each help line indented by two spaces, in order.
fn help_rows(help: &str) -> Vec<&str> {
    help.lines()
        .filter_map(|line| line.strip_prefix("  "))
        .filter_map(|rest| rest.split_once(' ').map(|(name, _)| name))
        .collect()
}

/// `help`, `--help` and `-h` each exit 0 and print every subcommand once with
/// the `LARRY_*` variables it reads.
#[test]
fn help_lists_every_subcommand_with_its_env() -> TestResult {
    for spelling in ["help", "--help", "-h"] {
        let output = run_word(spelling)?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "{spelling} exits 0: {}",
            stderr_of(&output)
        );
        assert!(
            output.stderr.is_empty(),
            "{spelling} writes nothing to stderr: {output:?}"
        );
        let help = String::from_utf8(output.stdout)?;
        let rows = help_rows(&help);
        for name in SUBCOMMANDS {
            assert!(
                rows.contains(&name),
                "{spelling} names {name} on its own line: {help}"
            );
        }
        for variable in VARIABLES {
            assert!(
                help.contains(variable),
                "{spelling} names {variable}: {help}"
            );
        }
        for line in help.lines().filter(|line| line.starts_with("  ")) {
            assert!(
                line.contains("(env: "),
                "every subcommand line names its env: {line}"
            );
        }
    }
    Ok(())
}

/// An argument that is no subcommand exits 1 and points at help, so a
/// first-time user with a typo is never left without a next step.
#[test]
fn an_unknown_word_is_refused_and_names_help() -> TestResult {
    let output = run_word("nonsense")?;
    assert_eq!(output.status.code(), Some(1), "a refusal exits 1");
    let stderr = stderr_of(&output);
    assert_eq!(stderr.lines().count(), 1, "one refusal line: {stderr:?}");
    assert!(
        stderr.contains("larry help"),
        "the refusal names `larry help`: {stderr:?}"
    );
    Ok(())
}

/// A listed word followed by an argument that word does not take exits 1 with
/// the one refusal line, and runs nothing: the dispatcher finds the word's row
/// in the help table and that row's handler owns its arity (larry#295).
#[test]
fn a_listed_word_with_arguments_it_does_not_take_is_refused() -> TestResult {
    let cases: [&[&str]; 9] = [
        &["mcp", "--extra"],
        &["acp", "--extra"],
        &["model", "--extra"],
        &["exec", "--extra"],
        &["verify", "--extra"],
        &["key", "--extra"],
        &["eval", "--extra"],
        &["serve", "--extra"],
        &["serve", "key", "t1", "--extra"],
    ];
    for words in cases {
        let output = spawn_larry_args(words, &[], b"")?.wait_with_output()?;
        assert_eq!(
            output.status.code(),
            Some(1),
            "a refusal exits 1: {words:?}"
        );
        let stderr = stderr_of(&output);
        assert_eq!(stderr.lines().count(), 1, "one refusal line: {stderr:?}");
        assert!(
            stderr.contains("larry help"),
            "the refusal names `larry help`: {stderr:?}"
        );
        assert!(output.stdout.is_empty(), "nothing ran: {words:?}");
    }
    Ok(())
}

/// `larry route remove` on an id no route has is refused with the `route` word
/// first and the id named, like every other route refusal (larry#295).
#[test]
fn route_remove_of_an_unknown_id_names_route() -> TestResult {
    let state = TempDir::new()?;
    let output = spawn_larry_args(
        &["route", "remove", "r9"],
        &[("LARRY_STATE_DIR", state.path().as_os_str())],
        b"",
    )?
    .wait_with_output()?;
    assert_eq!(output.status.code(), Some(1), "a refusal exits 1");
    let stderr = stderr_of(&output);
    assert_eq!(stderr.lines().count(), 1, "one refusal line: {stderr:?}");
    assert!(
        stderr.starts_with("route: "),
        "the refusal says route first: {stderr:?}"
    );
    assert!(
        stderr.contains("r9"),
        "the refusal names the id: {stderr:?}"
    );
    assert_eq!(
        stderr.matches("route").count(),
        2,
        "`route` once as the prefix and once in the cause: {stderr:?}"
    );
    Ok(())
}

/// `larry route` usage refusals print the synopsis once, and every
/// subcommand's refusal is exactly one stderr line ending in one newline with
/// nothing on stdout (larry#543). A doubled synopsis or a missing newline
/// joins the next shell line to the refusal.
#[test]
fn route_usage_ends_with_one_newline() -> TestResult {
    use std::ffi::OsStr;

    /// The synopsis every route usage refusal ends with, as
    /// `executor::routes` prints it.
    const SYNOPSIS: &str = "usage: larry route add <id> --url <url> --model <model> --key-env <LARRY_KEY_NAME> --residency <CC> [--kind <openai-chat|anthropic-messages|openai-responses>] [--temperature <t>] [--request-bytes <n>] [--describe <text>] [--default] | residency <CC,...> | residency --clear | price <id> --in <dollars> --out <dollars> | list [--json] | remove <id> | default <id> | resolve [<id>]";
    /// The refusal for arguments no row of the help table describes.
    const UNKNOWN: &str = "larry: unknown arguments; run `larry help` for every subcommand and its LARRY_* variables\n";
    /// The shared `governance`/`tracker` usage refusal for `--bogus`.
    const GOVERNANCE: &str = "usage: larry governance [--report] [--root DIR] | larry tracker [--check] [--root DIR] (got [\"--bogus\"])\n";
    /// `larry mutants` with no judged run to read.
    const MUTANTS: &str = "larry mutants verdict: usage: larry mutants verdict <settings> <artifacts> <jobs.json> (fail closed)\n";
    /// `larry mascot --bogus`.
    const MASCOT: &str = "usage: larry mascot [--preview] [--pet DIR] [--svg PATH] [--check [--root DIR]] (got [\"--bogus\"])\n";

    /// Runs `larry <words>` and asserts the exact refusal bytes: exit 1,
    /// nothing on stdout, `expected` on stderr.
    fn check(words: &[&str], env: &[(&str, &OsStr)], expected: &str) -> TestResult {
        let output = spawn_larry_args(words, env, b"")?.wait_with_output()?;
        assert_eq!(output.status.code(), Some(1), "{words:?} exits 1");
        assert!(
            output.stdout.is_empty(),
            "{words:?} prints nothing on stdout: {:?}",
            output.stdout
        );
        let stderr = stderr_of(&output);
        assert_eq!(
            stderr, expected,
            "{words:?} prints exactly one line ending in one newline"
        );
        Ok(())
    }

    let state = TempDir::new()?;
    let state_env = [("LARRY_STATE_DIR", state.path().as_os_str())];
    // The route refusals: the usage synopsis once with its reason, the price
    // refusal, and the unknown id, each exactly one line.
    let bogus = format!("route: not a route command; {SYNOPSIS}\n");
    check(&["route", "bogus"], &state_env, &bogus)?;
    assert_eq!(
        bogus.matches("usage: larry route").count(),
        1,
        "the synopsis appears once"
    );
    check(
        &["route", "price", "r1", "--in", "-1", "--out", "1"],
        &state_env,
        "route price --in must be dollars per million tokens like 0.30, at most 10000\n",
    )?;
    check(
        &["route", "price", "nope", "--in", "1", "--out", "1"],
        &state_env,
        "route: no route has the id nope\n",
    )?;
    check(
        &["route", "add", "main", "--kind", "bogus"],
        &state_env,
        "route kind must be one of openai-chat, anthropic-messages, openai-responses\n",
    )?;
    // One refusal from every other subcommand that prints one.
    check(&["nonsense"], &[], UNKNOWN)?;
    for words in [
        vec!["mcp", "--extra"],
        vec!["acp", "--extra"],
        vec!["model", "--extra"],
        vec!["exec", "--extra"],
        vec!["verify", "--extra"],
        vec!["key", "--extra"],
        vec!["eval", "--extra"],
        vec!["serve", "--extra"],
        vec!["serve", "key", "t1", "--extra"],
    ] {
        check(&words, &[], UNKNOWN)?;
    }
    check(&["governance", "--bogus"], &[], GOVERNANCE)?;
    check(&["tracker", "--bogus"], &[], GOVERNANCE)?;
    check(&["mutants"], &[], MUTANTS)?;
    check(&["mascot", "--bogus"], &[], MASCOT)?;
    Ok(())
}

/// `larry exec` reads `acceptance` as argv arrays. A document that sends a
/// string, a bare string per command, or a non-string item is refused with the
/// form and an example, never with a bare "wrong type" (larry#228).
#[test]
fn exec_a_wrong_typed_acceptance_names_argv_arrays() -> TestResult {
    let state = TempDir::new()?;
    // The repository path is the test's own temp directory: no document that reaches
    // further than the parser ever names a host path.
    let repo = state.path().display();
    let malformed = [
        format!(r#"{{"repo":"{repo}","base":"HEAD","acceptance":"run"}}"#),
        format!(r#"{{"repo":"{repo}","base":"HEAD","acceptance":["run"]}}"#),
        format!(r#"{{"repo":"{repo}","base":"HEAD","acceptance":[["ok"],7]}}"#),
    ];
    for task in malformed {
        let output = spawn_larry_args(
            &["exec"],
            &[("LARRY_STATE_DIR", state.path().as_os_str())],
            task.as_bytes(),
        )?
        .wait_with_output()?;
        assert_eq!(output.status.code(), Some(1), "a refusal exits 1: {task}");
        let stderr = stderr_of(&output);
        assert_eq!(stderr.lines().count(), 1, "one refusal line: {stderr:?}");
        assert!(
            stderr.contains("argv arrays"),
            "the refusal names the expected form: {stderr:?}"
        );
        assert!(
            stderr.contains(r#"[["true"]]"#),
            "the refusal shows an example: {stderr:?}"
        );
    }
    Ok(())
}

/// `route add` errors name the field once: the route check already says
/// `route`, so the CLI adds no second prefix (larry#228).
#[test]
fn route_add_errors_name_the_field_once() -> TestResult {
    let state = TempDir::new()?;
    // larry#295: the reserved id `env` is a field refusal too, and says `route` once.
    let cases: [(&str, &str, &str); 3] = [
        ("r1", "bad", "route key-env must name a variable"),
        (
            "r1",
            "OTHER_KEY",
            "route key-env must start with LARRY_KEY_",
        ),
        ("env", "LARRY_KEY_ROUTE", "route id env is reserved"),
    ];
    for (id, key_env, expected) in cases {
        let output = spawn_larry_args(
            &[
                "route",
                "add",
                id,
                "--url",
                "https://gateway.test:8443/v1",
                "--model",
                "model-1",
                "--key-env",
                key_env,
                "--residency",
                "CA",
            ],
            &[("LARRY_STATE_DIR", state.path().as_os_str())],
            b"",
        )?
        .wait_with_output()?;
        assert_eq!(
            output.status.code(),
            Some(1),
            "a refusal exits 1: {key_env}"
        );
        let stderr = stderr_of(&output);
        assert_eq!(stderr.lines().count(), 1, "one refusal line: {stderr:?}");
        assert!(
            stderr.contains(expected),
            "the refusal names the field once: {stderr:?}"
        );
        assert_eq!(
            stderr.matches("route").count(),
            1,
            "the refusal carries one `route` word: {stderr:?}"
        );
    }
    Ok(())
}

/// More arguments than the command line takes is a typed refusal naming the bound,
/// never a silent truncation (larry#338).
#[test]
fn more_arguments_than_the_bound_is_refused() -> TestResult {
    let bound = 32_usize;
    let mut words: Vec<&str> = vec!["help"];
    words.extend(std::iter::repeat_n("unused", bound));
    let output = spawn_larry_args(&words, &[], b"")?.wait_with_output()?;
    assert_eq!(
        output.status.code(),
        Some(1),
        "one argument past the bound is a refusal"
    );
    let stderr = stderr_of(&output);
    assert_eq!(stderr.lines().count(), 1, "one refusal line: {stderr:?}");
    assert!(
        stderr.contains(&bound.to_string()),
        "the refusal names the bound: {stderr:?}"
    );
    assert!(output.stdout.is_empty(), "nothing runs past the bound");
    // At the bound itself the word still runs: the refusal starts one past it.
    let mut at_bound: Vec<&str> = vec!["help"];
    at_bound.extend(std::iter::repeat_n("unused", bound - 1));
    let output = spawn_larry_args(&at_bound, &[], b"")?.wait_with_output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "the bound is admitted: {}",
        stderr_of(&output)
    );
    assert!(!output.stdout.is_empty(), "help prints at the bound");
    Ok(())
}
