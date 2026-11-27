//! Unit and seeded tests for the typed verification gate (larry#271, ADR 0130):
//! the legacy `done` token is refused, a finish needs the last fresh passing
//! check receipt over a tree the check did not change, plus present artifacts,
//! the first finish returns the checklist, the repeated identical finish ends
//! the session, and any later command voids the handshake. The seeded sim drives
//! the session turn loop's production verdict (`Turn::finish_reply`) over a
//! generated task with a scripted model, and an independent reference rule
//! replays the same steps from its own content map; both verdicts match for
//! every finish of every seed, and the same seed replays the same trace hash.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use lgwks_bot::kernel::clock::Clock;
use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;

use crate::model::{ModelClient, sim_config};
use crate::sim_error::SimError;

use super::super::run::Place;
use super::super::session_gate::{
    CheckProof, VerificationFault, fingerprint_tree, first_change, invalidate_finish, is_special,
};
use super::super::session_reply::{ReplyFault, parse_reply};
use super::super::{Ledger, Settings};
use super::{
    FinishRequest, Observer, Turn, command_passed, has_fresh_check, same_finish,
    verification_receipt, word,
};

#[test]
fn finish_refuses_without_fresh_passing_check() {
    let passed = true;
    let failed = false;
    assert_eq!(
        parse_reply(r#"{"done":"the task is complete"}"#),
        Err(ReplyFault::Keys),
        "the legacy model-written done token must not bypass verification"
    );
    assert!(
        !has_fresh_check(Some("cargo test"), failed, "cargo test"),
        "a failed check has no receipt"
    );
    assert!(
        !has_fresh_check(None, passed, "cargo test"),
        "a finish without a receipt is refused"
    );
    assert!(
        !has_fresh_check(Some("cargo fmt"), passed, "cargo test"),
        "a different passing command is not the named check"
    );
    assert!(
        has_fresh_check(Some("cargo test"), passed, "cargo test"),
        "a passing check is the receipt"
    );
    assert!(
        !has_fresh_check(Some("cargo test"), failed, "cargo test"),
        "the same check failed is no receipt"
    );
}

fn finish_request(summary: &str) -> FinishRequest {
    FinishRequest {
        summary: summary.to_owned(),
        check: String::from("true"),
        artifacts: vec![String::from("out")],
        state: vec![String::from("clean")],
        servers: vec![String::from("none")],
    }
}

#[test]
fn repeated_finish_requires_the_identical_request() {
    let first = finish_request("ready");
    assert!(same_finish(Some(&first), &first));
    assert!(!same_finish(None, &first));
    assert!(!same_finish(Some(&finish_request("different")), &first));
}

/// One finish verdict both the gate and the reference rule can give.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// The first acceptance: the checklist goes back to the model.
    Checklist,
    /// The repeated identical finish: the session ends.
    End,
    /// The finish is refused; the reason class and, for a mutated check, its path.
    Refused(&'static str, Option<String>),
}

/// A person nobody asks: the production verdict needs an observer, and the sim
/// answers finishes without asking anyone, so every command is approved and the
/// client never cancels.
struct NullObserver;

impl Observer for NullObserver {
    fn approve(&mut self, _step: usize, _command: &str) -> super::Approval {
        super::Approval::Run
    }
}

/// The sim never waits out a retry backoff: a `true` here would sleep the test
/// thread on a retryable refusal, breaking the seeded simulations' determinism
/// (SD-4). Named so the policy is pinned by a test, not hidden in a closure.
fn no_retry_wait(_wait: Duration) -> bool {
    false
}

#[test]
fn the_sim_client_never_waits_out_a_retry() {
    assert!(!no_retry_wait(Duration::ZERO));
    assert!(!no_retry_wait(Duration::from_secs(60)));
}

/// What the production verdict needs beyond the scripted steps: the client, the
/// settings, the ledger and the run identity a session turn borrows.
struct GateCtx {
    /// The model gateway the turn borrows; the sim never sends on it.
    client: ModelClient,
    /// The executor's settings the turn borrows.
    settings: Settings,
    /// The ledger the turn's proofs record in.
    ledger: Ledger,
    /// The run identity the turn reports under.
    task_id: String,
}

/// The production verdict for one scripted finish: the session turn loop's own
/// `finish_reply` over the scenario tree, so the sim exercises the shipped
/// composition (fresh receipt, unchanged tree, present artifacts, handshake),
/// never a reimplementation of it. Maps its typed fault to the sim's classes.
fn production_verdict(
    ctx: &GateCtx,
    tree: &Path,
    last_command: Option<&str>,
    last_passed: bool,
    last_proof: Option<&CheckProof>,
    first: &mut Option<FinishRequest>,
    request: &FinishRequest,
) -> Verdict {
    let mut null = NullObserver;
    let mut turn = Turn {
        client: &ctx.client,
        settings: &ctx.settings,
        ledger: &ctx.ledger,
        task_id: &ctx.task_id,
        place: Place::Open(tree),
        shown: String::from("."),
        artifact_root: tree,
        last_proof: None,
        observer: &mut null,
    };
    match turn.finish_reply(request, last_command, last_passed, last_proof, first) {
        Ok(Some(_)) => Verdict::End,
        Ok(None) => Verdict::Checklist,
        Err(VerificationFault::NoFreshCheck) => Verdict::Refused("nofresh", None),
        Err(VerificationFault::CheckMutated { path }) => Verdict::Refused("mutated", Some(path)),
        Err(VerificationFault::FingerprintTooLarge { .. }) => Verdict::Refused("toolarge", None),
        Err(VerificationFault::FingerprintIo { .. }) => Verdict::Refused("unreadable", None),
        Err(VerificationFault::BadArtifactPath { .. }) => Verdict::Refused("badpath", None),
        Err(VerificationFault::MissingArtifact { .. }) => Verdict::Refused("missing", None),
        Err(VerificationFault::SymlinkArtifact { .. }) => Verdict::Refused("symlink", None),
    }
}

/// One entry of the reference rule's own content map: the whole bytes, not a digest.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RefEntry {
    /// A directory; its members are entries of their own.
    Dir,
    /// A symlink, never followed; its target text is the entry.
    Link(String),
    /// A regular file with its whole bytes.
    File(Vec<u8>),
    /// Anything with no bytes to compare: a fifo, a socket or a device.
    Other,
}

/// The reference rule's recursive walk of the tree under `root`: every entry's
/// relative path to its whole content. Written apart from the production
/// fingerprint on purpose (recursion, whole bytes, no bounds), so a shared bug
/// in one walk cannot hide in the other.
fn ref_snapshot(
    root: &Path,
    relative: &Path,
    map: &mut BTreeMap<String, RefEntry>,
) -> Result<(), SimError> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(root.join(relative))? {
        names.push(entry?.file_name());
    }
    names.sort();
    for name in names {
        let path = relative.join(&name);
        let key = path.to_string_lossy().into_owned();
        let full = root.join(&path);
        let kind = std::fs::symlink_metadata(&full)?.file_type();
        if kind.is_dir() {
            map.insert(key, RefEntry::Dir);
            ref_snapshot(root, &path, map)?;
        } else if kind.is_symlink() {
            map.insert(
                key,
                RefEntry::Link(std::fs::read_link(&full)?.to_string_lossy().into_owned()),
            );
        } else if is_special(&kind) {
            map.insert(key, RefEntry::Other);
        } else {
            map.insert(key, RefEntry::File(std::fs::read(&full)?));
        }
    }
    Ok(())
}

/// The reference rule's own first difference between two content maps, in sorted order.
fn ref_first_change(
    before: &BTreeMap<String, RefEntry>,
    after: &BTreeMap<String, RefEntry>,
) -> Option<String> {
    let mut keys: Vec<&String> = Vec::new();
    keys.extend(before.keys());
    keys.extend(after.keys());
    keys.sort();
    keys.dedup();
    for key in keys {
        if before.get(key) != after.get(key) {
            return Some(key.clone());
        }
    }
    None
}

/// The reference rule's own writable-path check, stated apart from the author's:
/// non-empty, bounded, NUL-free, relative, normal components only, no `.git`.
fn ref_writable(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.contains('\0')
        && Path::new(path).components().all(|part| match part {
            Component::Normal(name) => !name.eq_ignore_ascii_case(".git"),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => false,
        })
}

/// Runs `program` with `args` in `tree` and returns its exit code, `None` when a
/// signal ended it. The program runs with an argument vector, never through a
/// shell, so no generated text is ever reparsed. When `redirect` names a file,
/// the command's standard output is written there, like the shell `>` the
/// scripted model spells. One run feeds both the gate and the reference rule.
fn run_argv(
    tree: &Path,
    program: &str,
    args: &[String],
    redirect: Option<&str>,
) -> Result<Option<i32>, SimError> {
    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(tree)
        .output()?;
    if let Some(dest) = redirect {
        std::fs::write(tree.join(dest), &output.stdout)?;
    }
    Ok(output.status.code())
}

/// One scenario's live state: the generated task, the scripted model's place in
/// it, and what both verdicts saw. The bytes every step feeds into the trace hash
/// accumulate in `trace`; the scenario hashes them once at the end. No `Debug`:
/// the gate context holds the client, the settings and the ledger, none of which
/// the trace ever prints.
struct Driver {
    /// The generated task's tree.
    tree: PathBuf,
    /// Files the scripted model may name.
    pool: Vec<String>,
    /// Every step's tag, text and outcome, hashed once at the end.
    trace: Vec<u8>,
    /// The last command's text.
    last_command: Option<String>,
    /// Whether it passed without timeout or interruption.
    last_passed: bool,
    /// What it proved about the tree.
    last_proof: Option<CheckProof>,
    /// The gate's first-finish slot.
    first: Option<FinishRequest>,
    /// What the production verdict borrows: the client, the settings, the
    /// ledger and the run identity.
    gate: GateCtx,
    /// The reference rule's own first-finish slot, voided by the same commands.
    ref_first: Option<FinishRequest>,
    /// The reference content map before the last command ran.
    ref_before: BTreeMap<String, RefEntry>,
    /// The reference content map after the last command ran.
    ref_after: BTreeMap<String, RefEntry>,
    /// The verdict classes this run saw.
    counts: Counts,
}

impl Driver {
    /// The production gate's verdict for one finish: the session turn loop's own
    /// `finish_reply` over the scenario tree, so the sim answers from the
    /// shipped verdict, never a reimplementation of it.
    fn gate(&mut self, request: &FinishRequest) -> Verdict {
        production_verdict(
            &self.gate,
            &self.tree,
            self.last_command.as_deref(),
            self.last_passed,
            self.last_proof.as_ref(),
            &mut self.first,
            request,
        )
    }

    /// The reference rule's verdict for the same finish from its own content maps.
    fn reference(&mut self, request: &FinishRequest, changed: Option<&str>) -> Verdict {
        if !(self.last_passed && self.last_command.as_deref() == Some(request.check.as_str())) {
            return Verdict::Refused("nofresh", None);
        }
        let expected = ref_first_change(&self.ref_before, &self.ref_after);
        if expected.as_deref() != changed {
            return Verdict::Refused("reference-mismatch", expected);
        }
        if let Some(path) = expected {
            return Verdict::Refused("mutated", Some(path));
        }
        for path in &request.artifacts {
            if !ref_writable(path) {
                return Verdict::Refused("badpath", None);
            }
            match std::fs::symlink_metadata(self.tree.join(path)) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Verdict::Refused("symlink", None);
                }
                Ok(_) => {}
                Err(_) => return Verdict::Refused("missing", None),
            }
        }
        if self.ref_first.as_ref() == Some(request) {
            Verdict::End
        } else {
            self.ref_first = Some(request.clone());
            Verdict::Checklist
        }
    }

    /// Runs one scripted command and records what both rules need: an edit when
    /// `edit`, a read-only check otherwise. Each command spells what the model
    /// would send, but runs as a program with an argument vector, never through
    /// a shell. One shell-free run feeds both rules, and the turn loop's order
    /// (fingerprint, run, fingerprint, proof) is kept. Names carry the step so
    /// two steps never collide without a draw.
    fn command(&mut self, rng: &mut Seeded, edit: bool, step: usize) -> Result<(), SimError> {
        let file = if rng.index(4)? == 0 {
            format!("n{step}_{}.txt", rng.index(6)?)
        } else {
            let at = rng.index(self.pool.len())?;
            self.pool
                .get(at)
                .ok_or(SimError::Missing("the file pool"))?
                .clone()
        };
        let (program, args, redirect, command) = if edit {
            match rng.index(5)? {
                0 => ("touch", vec![file.clone()], None, format!("touch '{file}'")),
                1 => (
                    "rm",
                    vec![String::from("-f"), file.clone()],
                    None,
                    format!("rm -f '{file}'"),
                ),
                2 => (
                    "ln",
                    vec![String::from("-sf"), String::from("a.txt"), file.clone()],
                    None,
                    format!("ln -sf a.txt '{file}'"),
                ),
                _ => (
                    "printf",
                    vec![String::from("%s"), word(rng)?],
                    Some(file.clone()),
                    format!("printf %s '{file}'"),
                ),
            }
        } else {
            match rng.index(4)? {
                0 => (
                    "test",
                    vec![String::from("-f"), file.clone()],
                    None,
                    format!("test -f '{file}'"),
                ),
                1 => (
                    "grep",
                    vec![String::from("-q"), word(rng)?, file.clone()],
                    None,
                    format!("grep -q word '{file}'"),
                ),
                2 => (
                    "test",
                    vec![
                        String::from("!"),
                        String::from("-e"),
                        String::from("absent.txt"),
                    ],
                    None,
                    String::from("test ! -e 'absent.txt'"),
                ),
                _ => ("true", Vec::new(), None, String::from("true")),
            }
        };
        self.run_text(
            &command,
            program,
            &args,
            redirect.as_deref(),
            if edit { 0 } else { 1 },
        )
    }

    /// Runs the given program in the task tree and records what both rules need:
    /// the before and after fingerprints, the exit code and the trace bytes
    /// under `tag`. When `redirect` names a file, standard output lands there.
    fn run_text(
        &mut self,
        command: &str,
        program: &str,
        args: &[String],
        redirect: Option<&str>,
        tag: u8,
    ) -> Result<(), SimError> {
        let before = fingerprint_tree(&self.tree);
        self.ref_before.clear();
        ref_snapshot(&self.tree, Path::new(""), &mut self.ref_before)?;
        let code = run_argv(&self.tree, program, args, redirect)?;
        let proof = match (before, fingerprint_tree(&self.tree)) {
            (Ok(before), Ok(after)) => match first_change(&before, &after) {
                Some(path) => CheckProof::Mutated { path },
                None => CheckProof::Clean,
            },
            (Err(fault), _) | (_, Err(fault)) => CheckProof::Fault(fault),
        };
        self.ref_after.clear();
        ref_snapshot(&self.tree, Path::new(""), &mut self.ref_after)?;
        self.last_command = Some(command.to_owned());
        self.last_passed = command_passed(code, false, false);
        self.last_proof = Some(proof);
        invalidate_finish(&mut self.first);
        invalidate_finish(&mut self.ref_first);
        self.trace.push(tag);
        self.trace.extend_from_slice(command.as_bytes());
        match code {
            Some(exit) => {
                self.trace.push(0);
                self.trace.extend_from_slice(&exit.to_le_bytes());
            }
            None => {
                self.trace.push(1);
            }
        }
        Ok(())
    }

    /// Answers one scripted finish from both verdicts and counts the gate's.
    /// The two verdicts must agree, and the trace hash covers the gate's. With
    /// a pending checklist the script often repeats it verbatim, so the
    /// second-identical-finish path is exercised, not only refusals.
    fn finish(&mut self, rng: &mut Seeded, seed: u64, step: usize) -> Result<(), SimError> {
        let pick = rng.index(2)?;
        let check = if pick == 0 {
            match self.last_command.clone() {
                Some(last) => last,
                None => word(rng)?,
            }
        } else {
            word(rng)?
        };
        let request = if self.first.is_some() && rng.index(3)? == 0 {
            self.first
                .clone()
                .ok_or(SimError::Missing("the first finish"))?
        } else {
            let mut artifacts = Vec::new();
            let count = rng.index(2)?.saturating_add(1);
            for _ in 0..count {
                let pick = rng.index(6)?;
                if pick == 0 {
                    // An empty artifact name: the parser would refuse it, so the
                    // gate's own path rule must refuse it first.
                    artifacts.push(String::new());
                } else if pick == 1 {
                    artifacts.push(String::from("absent.txt"));
                } else {
                    let at = rng.index(self.pool.len())?;
                    artifacts.push(
                        self.pool
                            .get(at)
                            .ok_or(SimError::Missing("the file pool"))?
                            .clone(),
                    );
                }
            }
            FinishRequest {
                summary: word(rng)?,
                check: check.clone(),
                artifacts,
                state: vec![word(rng)?],
                servers: vec![word(rng)?],
            }
        };
        self.answer(&request, &check, seed, step);
        Ok(())
    }

    /// Answers the given finish request from both verdicts and counts the
    /// gate's: the two must agree, and the trace hash covers the gate's.
    fn answer(&mut self, request: &FinishRequest, check: &str, seed: u64, step: usize) {
        let changed: Option<String> = match self.last_proof.clone() {
            Some(CheckProof::Mutated { path }) => Some(path),
            Some(_) | None => None,
        };
        let gate = self.gate(request);
        let expected = self.reference(request, changed.as_deref());
        assert_eq!(
            gate, expected,
            "seed {seed:#x} step {step}: the gate and the reference rule agree"
        );
        self.trace.push(2);
        self.trace.extend_from_slice(check.as_bytes());
        self.trace.extend_from_slice(format!("{gate:?}").as_bytes());
        match gate {
            Verdict::Checklist => {
                self.counts.checklists = self.counts.checklists.saturating_add(1);
            }
            Verdict::End => self.counts.ends = self.counts.ends.saturating_add(1),
            Verdict::Refused("mutated", _) => {
                self.counts.mutated = self.counts.mutated.saturating_add(1);
            }
            Verdict::Refused("nofresh", _) => {
                self.counts.nofresh = self.counts.nofresh.saturating_add(1);
            }
            Verdict::Refused(_, _) => {
                self.counts.artifact = self.counts.artifact.saturating_add(1);
            }
        }
    }
}

/// Counts of the verdict classes one scenario run saw.
#[derive(Debug, Default)]
struct Counts {
    checklists: usize,
    ends: usize,
    mutated: usize,
    nofresh: usize,
    artifact: usize,
}

/// One scenario: a generated task, a scripted model of edits, checks and finishes.
/// Drives the production gate and the reference rule over the same runs and feeds
/// every step into the trace hash. Returns the trace hash and the verdict counts.
fn scenario(seed: u64) -> Result<(String, Counts), SimError> {
    let mut rng = Seeded::from_seed(seed);
    let root = super::super::sim_tree::scratch("verify-gate-scenario")?;
    std::fs::create_dir_all(root.join("sub"))?;
    let state = super::super::sim_tree::scratch("verify-gate-state")?;
    let gate = GateCtx {
        client: ModelClient::new(
            sim_config(None)?,
            Clock::virtual_at(Duration::ZERO),
            no_retry_wait,
        ),
        settings: super::super::sim_tree::settings(&state, None)?,
        ledger: super::super::Ledger::open(&state)?,
        task_id: format!("verify-gate-{seed}"),
    };
    let mut driver = Driver {
        tree: root.to_path_buf(),
        pool: vec![
            String::from("a.txt"),
            String::from("b.txt"),
            String::from("sub/d.txt"),
        ],
        trace: seed.to_le_bytes().to_vec(),
        last_command: None,
        last_passed: false,
        last_proof: None,
        first: None,
        gate,
        ref_first: None,
        ref_before: BTreeMap::new(),
        ref_after: BTreeMap::new(),
        counts: Counts::default(),
    };
    let initial = rng.index(3)?.saturating_add(1);
    for (index, name) in driver.pool.clone().into_iter().enumerate().take(initial) {
        let filler = word(&mut rng)?;
        std::fs::write(
            driver.tree.join(&name),
            format!("seed line {index} {filler}\n"),
        )?;
    }
    ref_snapshot(&driver.tree, Path::new(""), &mut driver.ref_before)?;
    let steps = rng.index(9)?.saturating_add(16);
    for step in 0..steps {
        let kind = rng.index(10)?;
        if kind < 4 {
            driver.command(&mut rng, true, step)?;
        } else if kind < 7 {
            driver.command(&mut rng, false, step)?;
        } else {
            driver.finish(&mut rng, seed, step)?;
        }
    }
    // The scripted model then does the task properly: an edit, its read-only
    // check, and the finish pair, so the checklist and the ending repeat are
    // covered every seed while the random play above covers the refusals.
    driver.run_text(
        "printf %s 'tail.txt'",
        "printf",
        &[String::from("%s"), String::from("tail")],
        Some("tail.txt"),
        0,
    )?;
    driver.run_text(
        "test -f 'tail.txt'",
        "test",
        &[String::from("-f"), String::from("tail.txt")],
        None,
        1,
    )?;
    let tail = FinishRequest {
        summary: String::from("the tail is checked"),
        check: String::from("test -f 'tail.txt'"),
        artifacts: vec![String::from("tail.txt")],
        state: vec![String::from("ready")],
        servers: vec![String::from("none")],
    };
    driver.answer(&tail, &tail.check, seed, steps);
    driver.answer(&tail, &tail.check, seed, steps.saturating_add(1));
    let mut trace = Hasher::new();
    trace.update(&driver.trace);
    Ok((trace.finalize().to_hex(), driver.counts))
}

#[test]
fn sim_generated_tasks_match_the_reference_verdict_for_every_seed() -> Result<(), SimError> {
    let mut checklists = 0_usize;
    let mut ends = 0_usize;
    let mut mutated = 0_usize;
    let mut nofresh = 0_usize;
    let mut artifact = 0_usize;
    for seed in [7_u64, 19, 271, 0x5EED_0089, 11, 23, 41, 101] {
        let (first, counts) = scenario(seed)?;
        let (second, _) = scenario(seed)?;
        assert_eq!(
            first, second,
            "seed {seed:#x}: the same seed replays the same trace hash"
        );
        checklists = checklists.saturating_add(counts.checklists);
        ends = ends.saturating_add(counts.ends);
        mutated = mutated.saturating_add(counts.mutated);
        nofresh = nofresh.saturating_add(counts.nofresh);
        artifact = artifact.saturating_add(counts.artifact);
    }
    assert_ne!(checklists, 0, "a first finish returns its checklist");
    assert_ne!(ends, 0, "a repeated identical finish ends the session");
    assert_ne!(mutated, 0, "a check that writes the tree is refused");
    assert_ne!(nofresh, 0, "a finish without a fresh receipt is refused");
    assert_ne!(artifact, 0, "a finish naming a missing artifact is refused");
    Ok(())
}

#[test]
fn receipt_marks_the_check_as_fresh_shell_evidence() {
    let receipt = verification_receipt(
        "true",
        &[String::from("out")],
        &[String::from("clean")],
        &[String::from("none")],
    );
    assert_eq!(
        receipt
            .get("fresh_shell")
            .and_then(lgwks_std::json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn command_passes_only_for_a_clean_zero_exit() {
    let off = false;
    let on = true;
    assert!(command_passed(Some(0), off, off));
    assert!(!command_passed(Some(1), off, off));
    assert!(!command_passed(Some(0), on, off));
    assert!(!command_passed(Some(0), off, on));
    assert!(!command_passed(Some(1), on, on));
}
