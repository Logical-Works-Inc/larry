//! Seeded simulations of one session turn loop step (larry#271): a generated task,
//! a scripted person who approves, declines or cancels, and scripted edits and
//! checks. The decision the step returns and the tree it leaves must match the
//! person's answer: a declined or cancelled command runs nothing. Split from
//! `session_verify.rs` so each file stays one responsibility; the gate's own
//! handshake sim stays beside the gate.

use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;

use crate::model::sim_config;
use crate::sim_error::SimError;

use super::super::session_gate::{CheckProof, fingerprint_tree, first_change};
use super::{Approval, CommandDecision, Observer, Turn, word};

/// The person's scripted answer for one turn-loop step: whether the command
/// runs, and whether the client has cancelled the session.
#[derive(Debug)]
struct ScriptedPerson {
    /// Whether the person's answer runs the command.
    approve: bool,
    /// Whether the client cancelled before or while the command runs.
    cancelled: bool,
}

impl Observer for ScriptedPerson {
    fn handed(&mut self, _line: &str, _applied: bool) {}
    fn approve(&mut self, _step: usize, _command: &str) -> Approval {
        if self.approve {
            Approval::Run
        } else {
            Approval::Decline
        }
    }
    fn started(&mut self, _step: usize, _command: &str) {}
    fn finished(&mut self, _step: usize, _observation: &str, _passed: bool) {}
    fn message(&mut self, _text: &str) {}
    fn cancelled(&self) -> bool {
        self.cancelled
    }
}

/// One scenario driving the turn loop's own command step: a generated task, a
/// scripted person who approves, declines or cancels, and scripted edits and
/// checks. The decision the step returns and the tree it leaves must match the
/// person's answer: a declined or cancelled command runs nothing.
fn scenario_person(seed: u64) -> Result<(String, usize, usize, usize), SimError> {
    let mut rng = Seeded::from_seed(seed);
    let root = super::super::sim_tree::scratch("verify-gate-person")?;
    let tree = root.to_path_buf();
    std::fs::write(tree.join("work.txt"), "seed\n")?;
    let state = super::super::sim_tree::scratch("verify-gate-person-state")?;
    let settings = super::super::sim_tree::settings(&state, None)?;
    let ledger = super::super::Ledger::open(&state)?;
    let client = crate::model::ModelClient::new(
        sim_config(None)?,
        lgwks_bot::kernel::clock::Clock::virtual_at(std::time::Duration::ZERO),
        |_| false,
    );
    let task_id = format!("sim-person-{seed}");
    let mut trace = Hasher::new();
    trace.update(&seed.to_le_bytes());
    let mut declined = 0_usize;
    let mut ran = 0_usize;
    let mut cancelled = 0_usize;
    for step in 0..16_usize {
        let approve = rng.index(2)? == 0;
        let stop = rng.index(8)? == 0;
        let edit = rng.index(2)? == 0;
        let command = if edit {
            format!("printf '{}' > 'p{step}.txt'", word(&mut rng)?)
        } else {
            String::from("true")
        };
        let before = fingerprint_tree(&tree)?;
        let mut person = ScriptedPerson {
            approve,
            cancelled: stop,
        };
        let mut turn = Turn {
            client: &client,
            settings: &settings,
            ledger: &ledger,
            task_id: &task_id,
            shown: String::from("."),
            place: super::super::run::Place::Open(&tree),
            artifact_root: &tree,
            last_proof: None,
            observer: &mut person,
        };
        let outcome = turn.run_one_command(step, &command)?;
        let after = fingerprint_tree(&tree)?;
        let unchanged = first_change(&before, &after).is_none();
        trace.update(&[u8::from(approve), u8::from(stop), u8::from(edit)]);
        trace.update(command.as_bytes());
        if !approve {
            // A decline wins over a cancel: the command never runs, so the
            // tree is unchanged whatever the client did.
            assert!(
                matches!(outcome, CommandDecision::Declined(_)),
                "seed {seed:#x} step {step}: a declined command runs nothing"
            );
            assert!(
                unchanged,
                "seed {seed:#x} step {step}: a declined step leaves the tree"
            );
            trace.update(&[0]);
            declined = declined.saturating_add(1);
        } else if stop {
            assert!(
                matches!(outcome, CommandDecision::Cancelled),
                "seed {seed:#x} step {step}: a cancelled client ends the loop"
            );
            assert!(
                unchanged,
                "seed {seed:#x} step {step}: a cancelled step runs nothing"
            );
            trace.update(&[3]);
            cancelled = cancelled.saturating_add(1);
        } else {
            match outcome {
                CommandDecision::Ran(_, proof) => {
                    let changed = first_change(&before, &after);
                    match (proof, changed) {
                        (CheckProof::Clean, None) => {}
                        (CheckProof::Mutated { path }, Some(first)) => {
                            assert_eq!(
                                path, first,
                                "seed {seed:#x} step {step}: the proof names its change"
                            );
                        }
                        _ => {
                            return Err(SimError::Missing("the stored proof matches its own run"));
                        }
                    }
                    trace.update(&[1]);
                    ran = ran.saturating_add(1);
                }
                _ => {
                    return Err(SimError::Missing("an approved command reaches its owner"));
                }
            }
        }
    }
    Ok((trace.finalize().to_hex(), declined, ran, cancelled))
}

#[test]
fn sim_run_one_command_honors_the_persons_answer_for_every_seed() -> Result<(), SimError> {
    let mut declined = 0_usize;
    let mut ran = 0_usize;
    let mut cancelled = 0_usize;
    for seed in [3_u64, 13, 29, 271] {
        let (first, one_declined, one_ran, one_cancelled) = scenario_person(seed)?;
        let (second, _, _, _) = scenario_person(seed)?;
        assert_eq!(
            first, second,
            "seed {seed:#x}: the same seed replays the same trace hash"
        );
        declined = declined.saturating_add(one_declined);
        ran = ran.saturating_add(one_ran);
        cancelled = cancelled.saturating_add(one_cancelled);
    }
    assert_eq!(
        (declined, ran, cancelled),
        (31, 29, 4),
        "the scripted person declines, runs and cancels"
    );
    Ok(())
}
