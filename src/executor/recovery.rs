//! The recovery of the task journals a killed executor left (larry#75,
//! larry#182 defect 3).
//!
//! Each task's runtime journal is `journals/<task>.journal` under the state
//! directory. A file the executor returns from is removed; one a killed
//! executor left is recovered by the next start: replayed (a torn tail taken
//! back), restored with an owner that reaches nothing, ticked once so the
//! reservation it proves settles as `OutcomeUnknown` under `RequiresHuman`, and
//! kept as `<task>.recovered`, or `<task>.refused` when its bytes are refused.
//!
//! Liveness is the journal's own lock, never a process id: a task name is one
//! canonical UUID version 7 text that a container cannot reuse, so no pid can
//! claim another run's evidence (larry#182 defect 3). Recovery's owner reaches
//! nothing, so a recovered task's body never runs again.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use larry_contracts::authority::Scope;
use larry_contracts::binding::{BindingError, ResolvedEffect};
use larry_contracts::effect::{EffectIntent, EffectOutcome};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::Journal;
use larry_runtime::owner::EffectOwner;
use larry_runtime::projection::FoldProjection;
use larry_runtime::runtime::restore;
use lgwks_bot::journal::JournalError;

use super::ExecError;
use super::gate::{lease_end, task_settings};

/// The directory under the state directory that holds the task journals.
pub(super) const JOURNALS_DIR: &str = "journals";
/// The extension of a running task's journal.
const JOURNAL_EXT: &str = "journal";
/// The extension a recovered journal is kept under.
const RECOVERED_EXT: &str = "recovered";
/// The extension a journal whose bytes the store refused is kept under.
const REFUSED_EXT: &str = "refused";
/// The most journals one start recovers, in name order; the rest wait for the
/// next start, so a backlog never stalls a task without bound.
const MAX_RECOVERED: usize = 1024;

/// The journal file of `task_id` under `state`.
pub(super) fn journal_path(state: &Path, task_id: &str) -> PathBuf {
    state
        .join(JOURNALS_DIR)
        .join(format!("{task_id}.{JOURNAL_EXT}"))
}

/// Whether another executor still holds `task_id`'s journal, so its evidence is
/// live and must be left alone. The journal file's lock is the estate's own proof
/// of a live writer (`lgwks_bot::journal::JournalError::Locked`), so liveness needs
/// no process id at all: the name is one canonical UUID text that a container
/// cannot reuse (larry#182 defect 3).
///
/// A run whose journal the store or a replay refuses is nobody's: it is not live,
/// and the recovery sets its evidence aside. The workspace sweep asks this before
/// it removes a run's workspace, so a live run keeps it between its clone and its
/// first command (larry#182 defect 3).
///
/// # Errors
///
/// [`ExecError::Gate`] when the lease instant cannot be read.
pub(super) fn journal_live(
    state: &Path,
    policy: &BTreeSet<Scope>,
    task_id: &str,
) -> Result<bool, ExecError> {
    let path = journal_path(state, task_id);
    if !path.exists() {
        return Ok(false);
    }
    let capacity = task_settings(policy, lease_end(SystemTime::now())?).journal_capacity;
    match Journal::open(&path, capacity) {
        Ok(_) => Ok(false),
        Err(RuntimeError::Durable(JournalError::Locked { .. })) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Whether `task_id`'s run already has evidence this state directory kept: a
/// journal a killed executor left, which the recovery set aside as `<task>.recovered`.
/// A retry that derives this run id is answered from that evidence and never runs
/// the body again, because the earlier run's outcome is exactly what nobody observed
/// (larry#182 defect 3).
pub(super) fn recovered_evidence(state: &Path, task_id: &str) -> bool {
    journal_path(state, task_id)
        .with_extension(RECOVERED_EXT)
        .exists()
}

/// The owner a recovered run is restored with. The task body died with the
/// executor that held it, so nothing here can reach the world: binding
/// resolves nothing and every reach is `Unknown`. A `RequiresHuman` effect
/// never asks it anything; it settles as unknown on the first tick.
struct Recovered;

impl EffectOwner for Recovered {
    fn resolve(&mut self, _intent: &EffectIntent) -> Result<Option<ResolvedEffect>, BindingError> {
        Ok(None)
    }

    fn dispatch_bound(
        &mut self,
        intent: &EffectIntent,
        _at: SystemTime,
    ) -> Result<EffectOutcome, BindingError> {
        Ok(unknown(intent))
    }

    fn dispatch(&mut self, intent: &EffectIntent, at: SystemTime) -> EffectOutcome {
        self.reconcile(intent, at)
    }

    fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
        unknown(intent)
    }
}

/// The only truthful answer about an effect nobody here can observe.
const fn unknown(intent: &EffectIntent) -> EffectOutcome {
    EffectOutcome::Unknown {
        operation: intent.operation,
    }
}

/// Replays, restores and ticks one journal a dead executor left, so the
/// reservation it proves is recorded as settled before the file is set aside.
fn recover_one(path: &Path, policy: &BTreeSet<Scope>) -> Result<(), RuntimeError> {
    let run_settings = task_settings(policy, lease_end(SystemTime::now())?);
    let journal = Journal::open(path, run_settings.journal_capacity)?;
    let mut runtime = restore::<FoldProjection, _>(
        run_settings,
        Recovered,
        larry_runtime::live_clock()?,
        journal,
    )?;
    runtime.tick()?;
    Ok(())
}

/// Recovers every task journal a killed executor left under `state`, at most
/// [`MAX_RECOVERED`] in name order, and returns how many were set aside.
///
/// A journal another executor still holds is skipped: its lock is the proof
/// it is live. A recovered journal is kept as `<task>.recovered`; one whose
/// bytes the store or the replay refuses is kept as `<task>.refused`, so a
/// damaged file is evidence, never a reason every later task fails.
///
/// # Errors
///
/// [`ExecError::Journals`] when the directory cannot be listed or a journal
/// cannot be renamed, and [`ExecError::Gate`] when a lease instant cannot be
/// read.
pub(super) fn recover(state: &Path, policy: &BTreeSet<Scope>) -> Result<usize, ExecError> {
    let listing = match std::fs::read_dir(state.join(JOURNALS_DIR)) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(ExecError::Journals(error)),
    };
    let mut paths: Vec<PathBuf> = listing
        .map(|entry| entry.map_err(ExecError::Journals).map(|entry| entry.path()))
        .collect::<Result<Vec<PathBuf>, ExecError>>()?;
    // The recovery's own open is the liveness proof, so a lock another executor
    // takes between the listing and the open is still respected.
    paths.retain(|path| path.extension() == Some(OsStr::new(JOURNAL_EXT)));
    paths.sort();
    paths.truncate(MAX_RECOVERED);
    let mut set_aside: usize = 0;
    for path in paths {
        let kept_as = match recover_one(&path, policy) {
            Ok(()) => RECOVERED_EXT,
            Err(RuntimeError::Durable(JournalError::Locked { .. })) => continue,
            Err(_) => REFUSED_EXT,
        };
        std::fs::rename(&path, path.with_extension(kept_as)).map_err(ExecError::Journals)?;
        set_aside = set_aside.saturating_add(1);
    }
    Ok(set_aside)
}

/// Seeded adversarial simulations of the recovery (larry#182 defect 3).
#[cfg(test)]
#[path = "sim_recovery.rs"]
mod sim;
