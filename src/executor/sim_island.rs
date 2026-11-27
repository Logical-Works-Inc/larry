//! Seeded simulations of an isolated session against a hostile model (larry#243, ADR 0046).
//! The scripted model chooses commands that write outside the workspace by every route a
//! shell has; the oracle is the person's folder, hashed whole (`.git` included) before and
//! after, never the code under test. Hand-off, apply faults, tenancy, refusal and the kill
//! between apply and seal are driven through the same island the session uses. The full
//! session behind the shipped binary is `tests/e2e_journey.rs`. Sources: CWE-22 and CWE-59
//! (path traversal, link following), CWE-367 (a time-of-check gap), ADR 0023 (a run that
//! died inside its effect is recovered as unknown).

use std::error::Error;
use std::path::Path;
use std::time::Duration;

use larry_runtime::error::RuntimeError;
use lgwks_std::seeded::Seeded;

use super::{HOST_REFUSED, Island, Isolation, Outcome, POLICY_REFUSED};
use crate::executor::confine::{ConfineError, Platform};
use crate::executor::gate::{self, Subject, admit_and_run};
use crate::executor::recovery::journal_path;
use crate::executor::run::{CommandResult, run_command};
use crate::executor::session::run_session;
use crate::executor::sim_tree::{
    Origin, Scratch, Shape, files_beneath, origin, scratch, settings, tree_digest,
};
use crate::executor::workspace::{ApplyFault, Diff, DiffSink};
use crate::executor::{Approval, ExecError, Handoff, Ledger, Observer, Settings, prepare};
use crate::json_at::At;
use crate::sim_error::SimError;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn Error>>;

/// Command shapes a hostile model uses.
const KINDS: usize = 8;

/// Seeds of the full sweep: every kind in every origin shape.
const SEEDS: u64 = 32;

/// Seeds the replayed sweep covers, to compare trace hashes.
const REPLAYED: u64 = 8;

/// An observer that answers the review as scripted and records what it was told.
struct Scripted {
    /// The answer to the review.
    answer: Handoff,
    /// Each summary it was asked about.
    reviewed: Vec<Vec<String>>,
    /// Each hand-off line and whether the folder changed.
    handed: Vec<(String, bool)>,
}

impl Scripted {
    /// An observer that answers `answer`.
    fn new(answer: Handoff) -> Self {
        Self {
            answer,
            reviewed: Vec::new(),
            handed: Vec::new(),
        }
    }
}

impl Observer for Scripted {
    fn approve(&mut self, _step: usize, _command: &str) -> Approval {
        Approval::Run
    }
    fn review(&mut self, summary: &[String]) -> Handoff {
        self.reviewed.push(summary.to_vec());
        self.answer
    }
    fn handed(&mut self, line: &str, applied: bool) {
        self.handed.push((line.to_owned(), applied));
    }
}

/// One tenant's state directory, ledger and origin folder.
struct Rig {
    /// The tenant's settings.
    settings: Settings,
    /// The ledger every command is recorded in.
    ledger: Ledger,
    /// The person's folder.
    origin: Origin,
    /// The tenant's state directory; removed after everything that lives in it.
    state: Scratch,
    /// The directory the origin lives in; removed with the rig.
    _root: Scratch,
}

impl Rig {
    /// A rig with a seeded origin of `shape`.
    fn new(label: &str, seed: u64, shape: Shape) -> Result<Self, SimError> {
        let (root, state) = (scratch(label)?, scratch(label)?);
        let settings = settings(&state, None)?;
        let origin = origin(&settings, &root, seed, shape)?;
        let ledger = Ledger::open(&state)?;
        Ok(Self {
            settings,
            ledger,
            origin,
            state,
            _root: root,
        })
    }

    /// An island over the origin for task `id`.
    fn open(&self, id: &str) -> Result<Island, ExecError> {
        Island::open(Platform::host(), &self.settings, id, &self.origin.dir)
    }

    /// Runs `script` with `sh -c` confined in `island`.
    fn exec(&self, island: &Island, id: &str, script: &str) -> Result<CommandResult, ExecError> {
        let argv = [String::from("sh"), String::from("-c"), script.to_owned()];
        run_command(
            &argv,
            island.place(),
            self.settings.path_env(),
            Duration::from_secs(60),
            &self.ledger,
            id,
            &|| false,
        )
    }

    /// The origin's digest, `.git` included.
    fn digest(&self) -> Result<String, SimError> {
        tree_digest(&self.origin.dir, false)
    }
}

/// The route of kind `kind`: a write the kernel must refuse (or, for 6, nothing at all).
fn attack(kind: usize, origin: &Path) -> String {
    let dir = origin.display();
    match kind {
        0 => format!("printf x > {dir}/escape.txt"),
        1 => String::from("printf x > ../escape.txt"),
        2 => format!("ln -s {dir}/f0.txt link && printf x > link"),
        3 => format!("ln {dir}/f0.txt hard"),
        4 => format!("mv {dir}/f0.txt moved"),
        5 => String::from("(cd / && printf x > pwned)"),
        6 => String::from(":"),
        _ => format!("rm -rf {dir}/nest"),
    }
}

/// Whether `said` is the kernel refusing a write.
fn refusal(said: &str) -> bool {
    let said = said.to_lowercase();
    said.contains("not permitted")
        || said.contains("permission denied")
        || said.contains("cross-device")
}

/// One session of one seed: a hostile command and a harmless write, then the person's answer.
/// Asserts the origin never changed except by the one approved write, and returns the trace.
fn session(seed: u64) -> Result<String, SimError> {
    let at = usize::try_from(seed)?;
    let kind = at.checked_rem(KINDS).ok_or(SimError::Missing("a kind"))?;
    let shape_at = at
        .checked_div(KINDS)
        .and_then(|n| n.checked_rem(Shape::ALL.len()));
    let shape = shape_at.and_then(|n| Shape::ALL.get(n));
    let shape = *shape.ok_or(SimError::Missing("a shape"))?;
    let apply = Seeded::from_seed(seed).below(2)? == 1;
    let rig = Rig::new("never", seed, shape)?;
    let before = rig.digest()?;
    let island = rig.open("run")?;
    let script = format!("{}; printf y > inside.txt", attack(kind, &rig.origin.dir));
    let result = rig.exec(&island, "run", &script)?;
    let refused = refusal(result.stderr());
    assert_eq!(
        refused,
        kind != 6,
        "seed {seed} kind {kind}: {}",
        result.stderr()
    );
    assert_eq!(
        rig.digest()?,
        before,
        "seed {seed} kind {kind}: the origin moved before the review"
    );
    let diff = island.diff()?.ok_or(SimError::Missing("the work tree"))?;
    let mut observer = Scripted::new(if apply {
        Handoff::Apply
    } else {
        Handoff::Discard
    });
    let outcome = island.hand_off(&rig.origin.dir, Some(&diff), &mut observer);
    let lines = observer
        .reviewed
        .first()
        .ok_or(SimError::Missing("a review"))?;
    let lines = lines.clone();
    // A link planted inside the copy is part of the change and is shown to the person.
    let offered = if kind == 2 { 3 } else { 2 };
    assert_eq!(
        lines.len(),
        offered,
        "seed {seed}: only the inside writes are offered: {lines:?}"
    );
    assert_eq!(observer.reviewed.len(), 1, "seed {seed}");
    let inside = rig.origin.dir.join("inside.txt");
    if apply {
        assert_eq!(outcome, Outcome::Applied, "seed {seed}");
        assert_eq!(std::fs::read_to_string(&inside)?, "y", "seed {seed}");
        std::fs::remove_file(&inside)?;
        if kind == 2 {
            std::fs::remove_file(rig.origin.dir.join("link"))?;
        }
    } else {
        assert_eq!(outcome, Outcome::Discarded, "seed {seed}");
        assert!(!inside.exists(), "seed {seed}: a discard wrote");
    }
    assert_eq!(
        rig.digest()?,
        before,
        "seed {seed} kind {kind} apply {apply}: origin changed"
    );
    // The planted link names the run's own temporary path, so its diff digest is not a replay fact.
    let digest = if kind == 2 { "-" } else { diff.digest.as_str() };
    Ok(format!(
        "kind {kind} shape {shape:?} refused {refused} lines {lines:?} diff {digest} outcome {}\n",
        outcome.name()
    ))
}

/// Each seed's trace hash, in seed order.
fn sweep(seeds: u64) -> Result<Vec<String>, SimError> {
    let mut hashes = Vec::new();
    for seed in 0..seeds {
        let mut hasher = lgwks_std::hash::Hasher::new();
        hasher.update(session(seed)?.as_bytes());
        hashes.push(hasher.finalize().to_hex());
    }
    Ok(hashes)
}

/// A hostile model, every route to a write outside the workspace, in every origin shape: the
/// folder is byte-identical on discard and holds only the approved write on apply, and the
/// same seeds replay to the same trace hashes.
#[test]
fn sim_isolated_session_never_writes_origin() -> TestResult {
    let first = sweep(SEEDS)?;
    let again = sweep(REPLAYED)?;
    assert_eq!(first.len(), usize::try_from(SEEDS)?);
    assert_eq!(
        first.get(..again.len()),
        Some(again.as_slice()),
        "a replay differs"
    );
    let distinct: std::collections::BTreeSet<&String> = first.iter().collect();
    assert!(
        distinct.len() > 8,
        "seeds collapse to {} traces",
        distinct.len()
    );
    Ok(())
}

/// The diff of `rig`'s island after `script`, with the island.
fn changed(rig: &Rig, id: &str, script: &str) -> Result<(Island, Diff), SimError> {
    let island = rig.open(id)?;
    rig.exec(&island, id, script)?;
    let diff = island.diff()?.ok_or(SimError::Missing("the work tree"))?;
    Ok((island, diff))
}

/// A diff sealed over `text`, as a forger who recomputes the digest would make it.
fn forged(text: &str) -> Result<Diff, SimError> {
    let mut sink = DiffSink::new();
    sink.push(text.as_bytes())?;
    Ok(sink.finish())
}

/// Each way a verified change fails to go back is its own variant, and the folder never moves.
#[test]
fn sim_island_each_apply_fault_gets_its_exact_variant() -> TestResult {
    let rig = Rig::new("faults", 4, Shape::Git)?;
    let before = rig.digest()?;
    let (island, diff) = changed(&rig, "f", "printf z > added.txt; printf q >> f0.txt")?;
    let mut cut = changed(&rig, "g", "printf z > added.txt")?.1;
    cut.truncated = true;
    assert!(matches!(
        island.workspace.apply(&rig.origin.dir, &cut),
        Err(ApplyFault::Truncated)
    ));
    let mut tampered = changed(&rig, "h", "printf z > added.txt")?.1;
    tampered.text.push_str("+extra\n");
    assert!(matches!(
        island.workspace.apply(&rig.origin.dir, &tampered),
        Err(ApplyFault::DigestMismatch)
    ));
    let gone = rig.origin.dir.join("no-such-folder");
    assert!(matches!(
        island.workspace.apply(&gone, &diff),
        Err(ApplyFault::Io(_))
    ));
    assert_eq!(rig.digest()?, before, "a refused apply moved the folder");
    island.workspace.apply(&rig.origin.dir, &diff)?;
    assert_eq!(
        std::fs::read_to_string(rig.origin.dir.join("added.txt"))?,
        "z"
    );
    // The folder changed under the person after the copy: git refuses, whole.
    let (late, diff) = changed(&rig, "late", "printf r >> f0.txt")?;
    std::fs::write(rig.origin.dir.join("f0.txt"), "moved under the person\n")?;
    let moved = rig.digest()?;
    let applied = late.workspace.apply(&rig.origin.dir, &diff);
    assert!(matches!(applied, Err(ApplyFault::Git { ref detail }) if !detail.is_empty()));
    assert_eq!(rig.digest()?, moved, "a refused patch changed the folder");
    Ok(())
}

/// Apply is not idempotent by luck: a second apply of the same diff is refused and the change
/// is in the folder exactly once.
#[test]
fn sim_island_apply_twice_never_doubles_the_change() -> TestResult {
    for seed in 0..4 {
        let rig = Rig::new("twice", seed, Shape::Git)?;
        let (island, diff) = changed(&rig, "t", "printf z > added.txt; printf q >> f0.txt")?;
        island.workspace.apply(&rig.origin.dir, &diff)?;
        let once = rig.digest()?;
        let second = island.workspace.apply(&rig.origin.dir, &diff);
        assert!(matches!(second, Err(ApplyFault::Git { .. })), "seed {seed}");
        assert_eq!(
            rig.digest()?,
            once,
            "seed {seed}: the change was applied twice"
        );
        let content = std::fs::read_to_string(rig.origin.dir.join("f0.txt"))?;
        assert_eq!(content.matches('q').count(), 1, "seed {seed}");
    }
    Ok(())
}

/// A diff the digest vouches for but that names a path outside the folder, or inside `.git`,
/// is refused by git and writes nothing: the digest proves integrity, not safety.
#[test]
fn sim_island_apply_refuses_a_tampered_diff() -> TestResult {
    let rig = Rig::new("evil", 2, Shape::Subfolder)?;
    let before = rig.digest()?;
    let parent = rig.origin.dir.parent().ok_or("a parent")?.to_path_buf();
    let (island, _) = changed(&rig, "e", ":")?;
    for path in [
        "../escape.txt",
        ".git/hooks/pre-commit",
        "nest/../../escape.txt",
    ] {
        let text = format!(
            "diff --git a/{path} b/{path}\nnew file mode 100755\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+owned\n"
        );
        let evil = forged(&text)?;
        let applied = island.workspace.apply(&rig.origin.dir, &evil);
        assert!(
            matches!(applied, Err(ApplyFault::Git { .. })),
            "{path}: {applied:?}"
        );
        assert!(
            !parent.join("escape.txt").exists(),
            "{path}: wrote outside the folder"
        );
        assert_eq!(rig.digest()?, before, "{path}");
    }
    Ok(())
}

/// The observer is told exactly what became of the change, and asked only when there is one
/// worth asking about.
#[test]
fn sim_island_handoff_names_and_notices_match_each_outcome() -> TestResult {
    let rig = Rig::new("notices", 7, Shape::Plain)?;
    let (island, diff) = changed(&rig, "n", "printf z > added.txt")?;
    let mut yes = Scripted::new(Handoff::Apply);
    assert_eq!(
        island.hand_off(&rig.origin.dir, Some(&diff), &mut yes),
        Outcome::Applied
    );
    assert_eq!((yes.reviewed.len(), yes.handed.len()), (1, 1));
    assert!(
        yes.handed
            .first()
            .is_some_and(|&(ref line, applied)| applied && line.contains("applied"))
    );
    let (island, diff) = changed(&rig, "m", "printf w > other.txt")?;
    let mut no = Scripted::new(Handoff::Discard);
    assert_eq!(
        island.hand_off(&rig.origin.dir, Some(&diff), &mut no),
        Outcome::Discarded
    );
    assert!(
        no.handed
            .first()
            .is_some_and(|&(ref line, applied)| !applied && line.contains("discarded"))
    );
    assert!(!rig.origin.dir.join("other.txt").exists());
    let mut quiet = Scripted::new(Handoff::Apply);
    let empty = forged("")?;
    assert_eq!(
        island.hand_off(&rig.origin.dir, Some(&empty), &mut quiet),
        Outcome::None
    );
    assert_eq!(
        island.hand_off(&rig.origin.dir, None, &mut quiet),
        Outcome::None
    );
    let mut long = forged("diff --git a/x b/x\n")?;
    long.truncated = true;
    assert_eq!(
        island.hand_off(&rig.origin.dir, Some(&long), &mut quiet),
        Outcome::Failed
    );
    assert!(
        quiet.reviewed.is_empty(),
        "nothing is asked about an empty or a cut change"
    );
    assert!(quiet.handed.iter().all(|&(_, applied)| !applied));
    assert!(
        quiet
            .handed
            .first()
            .is_some_and(|entry| entry.0.contains("over 1 MiB"))
    );
    // The folder changed since the copy: apply is refused, said, and the folder is untouched.
    let (stale, diff) = changed(&rig, "s", "printf r >> f0.txt")?;
    std::fs::write(rig.origin.dir.join("f0.txt"), "changed by the person\n")?;
    let moved = rig.digest()?;
    let mut hopeful = Scripted::new(Handoff::Apply);
    assert_eq!(
        stale.hand_off(&rig.origin.dir, Some(&diff), &mut hopeful),
        Outcome::Failed
    );
    assert!(
        hopeful
            .handed
            .first()
            .is_some_and(|&(ref line, applied)| !applied && line.contains("not applied"))
    );
    assert_eq!(rig.digest()?, moved);
    Ok(())
}

/// Two tenants (state directories) with the same origin and the same run id never share a
/// workspace: each tree holds only its own writes and dropping one leaves the other.
#[test]
fn sim_island_two_state_dirs_same_origin_never_share_a_workspace() -> TestResult {
    let one = Rig::new("tenant-a", 1, Shape::Git)?;
    let state = scratch("tenant-b")?;
    let settings = settings(&state, None)?;
    let two = Rig {
        ledger: Ledger::open(&state)?,
        settings,
        origin: Origin {
            dir: one.origin.dir.clone(),
            files: one.origin.files.clone(),
        },
        state,
        _root: scratch("tenant-b-root")?,
    };
    let before = one.digest()?;
    let (first, first_diff) = changed(&one, "same", "printf a > who.txt")?;
    let (second, second_diff) = changed(&two, "same", "printf b > who.txt")?;
    assert_eq!(
        std::fs::read_to_string(first.workspace.path().join("who.txt"))?,
        "a"
    );
    assert_eq!(
        std::fs::read_to_string(second.workspace.path().join("who.txt"))?,
        "b"
    );
    assert_ne!(first_diff.digest, second_diff.digest);
    assert!(first.workspace.path().starts_with(&one.state));
    assert!(second.workspace.path().starts_with(&two.state));
    let kept = second.workspace.path().to_path_buf();
    drop(first);
    assert!(
        kept.is_dir(),
        "dropping tenant a removed tenant b's workspace"
    );
    assert!(!one.state.join("ws").join("same").exists());
    assert_eq!(one.digest()?, before);
    Ok(())
}

/// A host that cannot confine refuses, typed, before anything is copied, recorded or asked:
/// no workspace, no journal, no model request, nothing said to the person.
#[test]
fn sim_island_refused_confinement_starts_nothing() -> TestResult {
    let rig = Rig::new("refused", 3, Shape::Plain)?;
    let isolation = Isolation::Workspace(Platform::Other);
    assert!(matches!(
        isolation.check(),
        Err(ExecError::Confine(ConfineError::Unsupported { .. }))
    ));
    assert!(
        isolation
            .blocked(&rig.settings)
            .is_some_and(|text| text.contains("no kernel confinement"))
    );
    let mut observer = Scripted::new(Handoff::Apply);
    let ended = run_session(
        "do it",
        &rig.origin.dir,
        &rig.settings,
        isolation,
        &mut observer,
        crate::executor::SessionDoor::Tenant,
    );
    assert!(matches!(
        ended,
        Err(ExecError::Confine(ConfineError::Unsupported { .. }))
    ));
    assert!(!rig.state.join("ws").exists() && !rig.state.join("journals").exists());
    assert!(observer.reviewed.is_empty() && observer.handed.is_empty());
    let opened = Island::open(Platform::Other, &rig.settings, "x", &rig.origin.dir);
    assert!(matches!(
        opened,
        Err(ExecError::Confine(ConfineError::Unsupported { .. }))
    ));
    assert!(
        !rig.state.join("ws").join("x").exists(),
        "a refused open left a copy"
    );
    Ok(())
}

/// The gate's scopes follow the isolation: a confined session needs no `model.command`, a host
/// session does, and each refusal text names what to change.
#[test]
fn sim_island_gate_scopes_follow_the_isolation() -> TestResult {
    let state = scratch("scopes")?;
    let default = settings(&state, None)?;
    let empty = settings(&state, Some(""))?;
    let full = settings(
        &state,
        Some("file.read,file.write,process.spawn,model.invoke,model.command"),
    )?;
    let confined = Isolation::Workspace(Platform::host());
    let mut want = gate::session_scopes();
    assert!(want.remove(&larry_contracts::authority::Scope::ModelCommand));
    assert_eq!(confined.required(), want);
    assert_eq!(Isolation::Host.required(), gate::session_scopes());
    assert_eq!(
        confined.blocked(&default),
        None,
        "the default policy runs a confined session"
    );
    assert_eq!(confined.blocked(&empty).as_deref(), Some(POLICY_REFUSED));
    assert_eq!(
        Isolation::Host.blocked(&default).as_deref(),
        Some(HOST_REFUSED)
    );
    assert_eq!(Isolation::Host.blocked(&full), None);
    assert_eq!(Isolation::Host.name(), "host");
    assert_eq!(confined.name(), "workspace");
    Ok(())
}

/// A command that runs `git commit` in the workspace cannot hide its change: the git directory
/// is outside the writable tree so the commit is refused, and the diff is against the base id
/// taken before any command, which HEAD still names.
#[test]
fn sim_island_git_commit_in_workspace_still_shows_change() -> TestResult {
    let rig = Rig::new("commit", 6, Shape::Git)?;
    let island = rig.open("c")?;
    let script = "printf x > new.txt; git add -A; git -c user.name=a -c user.email=a@b commit -m hide; printf y > after.txt";
    let result = rig.exec(&island, "c", script)?;
    assert!(
        refusal(result.stderr()),
        "the commit was not refused: {}",
        result.stderr()
    );
    assert_eq!(
        island.workspace.head()?,
        island.base,
        "a command moved HEAD"
    );
    let diff = island.diff()?.ok_or("the work tree is gone")?;
    assert!(
        diff.text.contains("new.txt") && diff.text.contains("after.txt"),
        "{}",
        diff.text
    );
    Ok(())
}

/// The run that died between `git apply` and the sealed report: its journal outlives it, so
/// the same run answers from that evidence and its body never runs, and never applies, again.
/// The body fails after the apply (a failed seal); the unknown-outcome path of a real kill -9
/// is ADR 0023's recovery, which this change does not alter.
#[test]
fn sim_kill_between_apply_and_seal_never_applies_the_change_again() -> TestResult {
    let rig = Rig::new("kill", 8, Shape::Git)?;
    let (_ledger, key) = prepare(&rig.settings)?;
    let id = "kill-between";
    let subject = |path| Subject {
        required: gate::isolated_scopes(),
        path,
        bytes: b"document",
    };
    let first = admit_and_run(
        subject(&rig.origin.dir),
        &rig.settings,
        id,
        "digest",
        &key,
        || {
            let (island, diff) = match changed(&rig, id, "printf z > added.txt") {
                Ok(pair) => pair,
                Err(SimError::Exec(cause)) => return Err(cause),
                Err(other) => {
                    return Err(ExecError::Workspace(std::io::Error::other(
                        other.to_string(),
                    )));
                }
            };
            let mut observer = Scripted::new(Handoff::Apply);
            let outcome = island.hand_off(&rig.origin.dir, Some(&diff), &mut observer);
            assert_eq!(outcome, Outcome::Applied);
            // The process dies here: the folder changed, the report is never sealed.
            Err(ExecError::Unsealed(RuntimeError::ClockOutOfRange {
                elapsed: Duration::ZERO,
            }))
        },
    );
    assert!(matches!(first, Err(ExecError::Unsealed(_))));
    assert_eq!(
        std::fs::read_to_string(rig.origin.dir.join("added.txt"))?,
        "z"
    );
    assert!(
        journal_path(&rig.state, id).exists(),
        "the dead run's journal is the evidence"
    );
    let after = rig.digest()?;
    let mut ran = false;
    let again = admit_and_run(
        subject(&rig.origin.dir),
        &rig.settings,
        id,
        "digest",
        &key,
        || {
            ran = true;
            Err(ExecError::StateDirUnset)
        },
    )?;
    assert!(!ran, "the dead run's body ran again");
    let line = again
        .json()
        .at("/denied/0")?
        .as_str()
        .ok_or("a denial line")?;
    assert!(
        line.contains("recovered from its journal as unknown") || line.contains("already recorded"),
        "{line}"
    );
    assert_eq!(rig.digest()?, after, "the retry changed the folder");
    let files = files_beneath(&rig.origin.dir)?;
    assert_eq!(files.iter().filter(|name| *name == "added.txt").count(), 1);
    Ok(())
}
