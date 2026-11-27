//! Runs a terminal session end to end (larry#93): the authorization door, the
//! route and model, the kernel-confined copy, the turn loop and the sealed
//! report. Split from `session.rs` so each file stays one responsibility; the
//! turn loop, its gate and its proofs stay beside the session they drive.

use std::path::Path;

use lgwks_std::json::{self, json};

use super::gate::{self, Subject};
use super::identity::fresh_id;
use super::island::{Island, Isolation, Outcome};
use super::prepare;
use super::run::Place;
use super::session::{Ended, Observer, Stop, Turn, check};
use super::session_gate::SessionIdentity;
use super::session_report::body;
use super::{ExecError, Ledger, Report, Settings, backoff, digest_hex};
use crate::model::ModelClient;

/// Which authorization a terminal session runs under: the tenant door or the
/// narrower evaluator route beneath `<state>/eval` (larry#207).
#[derive(Debug, Clone, Copy)]
pub(crate) enum SessionDoor {
    /// A person's door: the tenant allowlist, open until its first root row.
    Tenant,
    /// An evaluator-owned origin: only beneath `<state>/eval`.
    Eval,
}

/// Runs a terminal session on `instruction` in `cwd`, reporting through `observer`.
/// A refusal by the gate seals a report that ran nothing; a fault outside the
/// session's doing (the ledger, a process group) is a typed error.
pub(crate) fn run_session(
    instruction: &str,
    cwd: &Path,
    settings: &Settings,
    isolation: Isolation,
    observer: &mut dyn Observer,
    door: SessionDoor,
) -> Result<Ended, ExecError> {
    check(instruction, cwd).map_err(ExecError::Session)?;
    // A host that cannot confine refuses here, before the ledger, the model or a copy.
    isolation.check()?;
    let (ledger, key) = prepare(settings)?;
    let repository = match door {
        SessionDoor::Tenant => ledger.authorized_repo(
            &super::tenant_key(settings.state_dir()),
            settings.state_dir(),
            cwd,
            settings.is_local(),
        )?,
        SessionDoor::Eval => Ledger::authorized_eval_repo(settings.state_dir(), cwd)?,
    };
    run_session_prepared(
        instruction,
        &repository,
        settings,
        isolation,
        observer,
        &ledger,
        &key,
    )
}

/// Runs the session in an already authorized directory: the route, the model,
/// the gated proposal and the sealed report.
fn run_session_prepared(
    instruction: &str,
    cwd: &Path,
    settings: &Settings,
    isolation: Isolation,
    observer: &mut dyn Observer,
    ledger: &Ledger,
    key: &lgwks_std::sign::SecretKey,
) -> Result<Ended, ExecError> {
    let route = settings.route(ledger, None)?;
    let config = settings.model_config(&route)?;
    let document = json::to_vec(&json!({
        "session": {
            "cwd": cwd.to_string_lossy(),
            "instruction": instruction,
            "isolation": isolation.name(),
        }
    }))
    .map_err(|cause| ExecError::Gate(cause.into()))?;
    let task_id = fresh_id()?;
    let task_digest = digest_hex(&document);
    let mut stop = None;
    let subject = Subject {
        required: isolation.required(),
        path: cwd,
        bytes: &document,
    };
    let report = gate::admit_and_run(subject, settings, &task_id, &task_digest, key, || {
        // The retry budget's timeline, built before the copy so a host whose clock refuses
        // copies nothing (larry#436).
        let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
        // Opened inside the gate: a denied session copies nothing.
        let island = match isolation {
            Isolation::Host => None,
            Isolation::Workspace(platform) => {
                Some(Island::open(platform, settings, &task_id, cwd)?)
            }
        };
        let client = ModelClient::new(config, clock, backoff);
        let mut turn = Turn {
            client: &client,
            settings,
            ledger,
            task_id: &task_id,
            place: match island {
                Some(ref isle) => isle.place(),
                None => Place::Open(cwd),
            },
            artifact_root: match island {
                Some(ref isle) => isle.path(),
                None => cwd,
            },
            // A check must change nothing: the proof starts empty and each command
            // replaces it with its own before and after fingerprints.
            last_proof: None,
            // The model is told its directory is `.` in a copy, so the request carries
            // no run id and no path of the person's.
            shown: match island {
                Some(_) => String::from("."),
                None => cwd.display().to_string(),
            },
            observer: &mut *observer,
        };
        let done = turn.run(instruction)?;
        let done_stop = done.stop.clone();
        let mut outcome = Outcome::None;
        let mut diff = None;
        if let Some(ref isle) = island
            && done.stop == Stop::EndTurn
        {
            diff = isle.diff()?;
            outcome = isle.hand_off(cwd, diff.as_ref(), &mut *observer);
        }
        let body = body(
            &SessionIdentity {
                task_id: &task_id,
                task_digest: &task_digest,
            },
            done,
            &route.seal(),
            isolation,
            island.as_ref(),
            outcome,
            diff,
        );
        stop = Some(done_stop);
        Ok(Report::sealed(body, key))
    })?;
    let stop = match stop {
        Some(stop) => stop,
        None => {
            let line = denial(&report);
            observer.message(&line);
            Stop::Refusal(line)
        }
    };
    Ok(Ended { stop, report })
}

/// The one denial line of a report the gate sealed without running anything.
fn denial(report: &Report) -> String {
    let first = report
        .json()
        .get("denied")
        .and_then(lgwks_std::json::Value::as_array)
        .and_then(|lines| lines.first())
        .and_then(lgwks_std::json::Value::as_str);
    match first {
        Some(line) => line.to_owned(),
        None => String::from("gate: the session was denied"),
    }
}

#[cfg(test)]
mod issue207_tests {
    use super::super::session::{Approval, Observer};
    use super::{SessionDoor, run_session};
    use crate::executor::gate;
    use crate::executor::sim_tree::scratch;
    use crate::executor::{Isolation, Ledger, Settings, tenant_key};

    /// An observer that must never be reached by a refusal before model setup.
    struct QuietObserver;

    impl Observer for QuietObserver {
        fn approve(&mut self, _step: usize, _command: &str) -> Approval {
            Approval::Decline
        }
    }

    /// A remote ACP session must refuse an unallowlisted cwd before route or model resolution.
    #[test]
    fn sim_session_refuses_an_unallowlisted_cwd_before_model()
    -> Result<(), Box<dyn std::error::Error>> {
        let state = scratch("issue207-session-state")?;
        let work = scratch("issue207-session-work")?;
        let policy = gate::session_scopes();
        Ledger::open(&state)?.set_tenant_policy(&tenant_key(&state), &policy)?;
        let state_text = state.to_string_lossy().into_owned();
        let path = std::env::var("PATH")?;
        let settings = Settings::remote_from_env(move |name| match name {
            "LARRY_STATE_DIR" => Some(state_text.clone()),
            "PATH" => Some(path.clone()),
            _ => None,
        })?;
        let mut observer = QuietObserver;
        assert!(matches!(
            run_session(
                "read this repository",
                &work,
                &settings,
                Isolation::Host,
                &mut observer,
                SessionDoor::Tenant,
            ),
            Err(super::super::ExecError::RepoNotAllowed { .. })
        ));
        Ok(())
    }
}
