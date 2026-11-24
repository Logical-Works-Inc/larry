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
use std::io::Write as _;
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
use lgwks_bot::kernel::journal::JournalError;

use super::ExecError;
use super::gate::{lease_start, task_settings, with_expiry};

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

/// The extension of a run's liveness lock (larry#456).
const LOCK_EXT: &str = "lock";

/// The journal file of `task_id` under `state`.
pub(super) fn journal_path(state: &Path, task_id: &str) -> PathBuf {
    state
        .join(JOURNALS_DIR)
        .join(format!("{task_id}.{JOURNAL_EXT}"))
}

/// A run's liveness lock, `<task>.lock`, held from before its journal exists until after
/// the run removes it (larry#456). The journal's own lock cannot say this alone: between a
/// run creating its journal and locking it, another executor's recovery could take the
/// journal, and a run that dropped its journal's lock before removing it left a window where
/// recovery set a finished run aside. A run removes its lock file last; nobody else creates
/// or removes one, so a lock taken on a file another run still holds is always refused.
pub(super) struct RunLock {
    /// The locked file; dropping it releases the lock.
    file: std::fs::File,
    /// Where it lives, removed by the run that made it.
    path: PathBuf,
}

impl RunLock {
    /// The run's own lock, created and taken before its journal is opened; `None` when a live
    /// run with this id already holds it (a keyed retry while the first still runs).
    pub(super) fn take(state: &Path, task_id: &str) -> Result<Option<Self>, ExecError> {
        let path = journal_path(state, task_id).with_extension(LOCK_EXT);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(ExecError::Journals)?;
        Self::held(file, path)
    }

    /// Another run's lock, never created: `Ok(None)` when that run is live, `Err` for a fault,
    /// and a held guard (or `Some(None)` when the run made no lock file) when it is not.
    fn probe(journal: &Path) -> Result<Option<Option<Self>>, ExecError> {
        let path = journal.with_extension(LOCK_EXT);
        match std::fs::OpenOptions::new().write(true).open(&path) {
            Ok(file) => Ok(Self::held(file, path)?.map(Some)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Some(None)),
            Err(error) => Err(ExecError::Journals(error)),
        }
    }

    /// `file` locked without waiting, or `None` when another process holds it.
    fn held(file: std::fs::File, path: PathBuf) -> Result<Option<Self>, ExecError> {
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { file, path })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(ExecError::Journals(error)),
        }
    }

    /// Ends the run: its lock file is removed while still held, then released.
    pub(super) fn release(self) -> Result<(), ExecError> {
        let removed = match std::fs::remove_file(&self.path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(ExecError::Journals(error))
            }
            _ => Ok(()),
        };
        drop(self.file);
        removed
    }
}

/// Whether a live run still holds `task_id`'s run lock: its recorded process groups are its own
/// and no start-up reap may signal them (larry#456).
///
/// # Errors
///
/// [`ExecError::Journals`] when the lock cannot be probed.
pub(super) fn run_held(state: &Path, task_id: &str) -> Result<bool, ExecError> {
    Ok(RunLock::probe(&journal_path(state, task_id))?.is_none())
}

/// Whether another executor still holds `task_id`'s journal, so its evidence is
/// live and must be left alone. The journal file's lock is the estate's own proof
/// of a live writer (`lgwks_bot::kernel::journal::JournalError::Locked`), so liveness needs
/// no process id at all: the name is one canonical UUID text that a container
/// cannot reuse (larry#182 defect 3).
///
/// A run whose journal the store or a replay refuses, or that cannot be opened for
/// any reason other than its lock, is nobody's: it is not live, and the recovery
/// sets its evidence aside. The workspace sweep asks this before it removes a
/// run's workspace, so a live run keeps it between its clone and its first
/// command (larry#182 defect 3).
///
/// # Errors
///
/// [`ExecError::Journals`] when the path cannot be stat'ed, and [`ExecError::Journal`]
/// when the lock cannot be probed: an unknown liveness is never reported as dead
/// (larry#334).
pub(super) fn journal_live(
    state: &Path,
    policy: &BTreeSet<Scope>,
    task_id: &str,
) -> Result<bool, ExecError> {
    let path = journal_path(state, task_id);
    if !path.try_exists().map_err(ExecError::Journals)? {
        return Ok(false);
    }
    // The run's own lock first: probing its journal while it starts would take that lock from
    // it (larry#456). The guard is held across the journal probe and then dropped, never removed.
    let Some(_guard) = RunLock::probe(&path)? else {
        return Ok(true);
    };
    let capacity = task_settings(policy).journal_capacity;
    match Journal::open(&path, capacity) {
        Err(RuntimeError::Durable(JournalError::Locked { .. })) => Ok(true),
        Ok(_) => Ok(false),
        // A probe that fails for any other reason proves nothing about the run, so it
        // is its own typed answer rather than "dead" (larry#334).
        Err(cause) => Err(ExecError::Journal(cause)),
    }
}

/// The denial line a retry is answered with when `task_id`'s run already has evidence this
/// state directory kept: a journal a killed executor left, which the recovery set aside as
/// `<task>.recovered`, or as `<task>.refused` when its bytes were refused. A retry that derives
/// this run id is answered from that evidence and never runs the body again, because the
/// earlier run's outcome is exactly what nobody observed (larry#182 defect 3, larry#394).
///
/// # Errors
///
/// [`ExecError::Journals`] when the evidence cannot be stat'ed, so an unreadable
/// directory is never read as absent (larry#334).
pub(super) fn recovered_evidence(
    state: &Path,
    task_id: &str,
) -> Result<Option<&'static str>, ExecError> {
    let path = journal_path(state, task_id);
    for (ext, line) in [
        (
            RECOVERED_EXT,
            "gate: the run was recovered from its journal as unknown",
        ),
        (
            REFUSED_EXT,
            "gate: the run's journal was refused and set aside; its outcome is unknown",
        ),
    ] {
        if path
            .with_extension(ext)
            .try_exists()
            .map_err(ExecError::Journals)?
        {
            return Ok(Some(line));
        }
    }
    Ok(None)
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
    // One clock: the lease starts at its reading and the restored runtime ticks on it (larry#436).
    let clock = larry_runtime::live_clock()?;
    let run_settings = with_expiry(task_settings(policy), lease_start(&clock)?)?;
    let journal = Journal::open(path, run_settings.journal_capacity)?;
    let mut runtime = restore::<FoldProjection, _>(run_settings, Recovered, clock, journal)?;
    runtime.tick()?;
    Ok(())
}

/// One journal the recovery left as `<task>.journal`: recovering it failed for a reason
/// other than its bytes, so the next start tries again (larry#394).
#[derive(Debug)]
pub(super) struct KeptJournal {
    /// The journal left in place.
    pub(super) path: PathBuf,
    /// Why it could not be recovered now.
    pub(super) cause: RuntimeError,
}

impl KeptJournal {
    /// Reports the journal as one typed JSON line on standard error, the operator's channel,
    /// in the shape the workspace sweep reports a kept entry (larry#334).
    fn report(&self) {
        let event = lgwks_std::json::json!({
            "recovery": "kept",
            "entry": format!("{}", self.path.display()),
            "refused": self.cause.to_string(),
        });
        let mut err = std::io::stderr().lock();
        let written = writeln!(err, "{event}");
        drop(written);
    }
}

/// What one recovery sweep did: how many journals it set aside, and the ones it kept.
#[derive(Debug, Default)]
pub(super) struct Recovery {
    /// Journals renamed to `<task>.recovered` or `<task>.refused`.
    pub(super) set_aside: usize,
    /// Journals left as `<task>.journal`, each reported once, for the next start.
    pub(super) kept: Vec<KeptJournal>,
}

/// Recovers every task journal a killed executor left under `state`, at most
/// [`MAX_RECOVERED`] in name order.
///
/// A journal another executor still holds is skipped: its lock is the proof
/// it is live. A recovered journal is kept as `<task>.recovered`; one whose
/// bytes the store or the replay refuses ([`RuntimeError::refuses_the_record`]) is
/// kept as `<task>.refused`, so a damaged file is evidence, never a reason every
/// later task fails. Any other fault (the device, a lock probe, the clock, a setting,
/// a capacity) leaves the file a journal and is returned as one typed fault, so the
/// next start recovers it and a keyed retry still opens it rather than running the
/// body again (larry#394).
///
/// # Errors
///
/// [`ExecError::Journals`] when the directory cannot be listed or a journal
/// cannot be renamed.
pub(super) fn recover(state: &Path, policy: &BTreeSet<Scope>) -> Result<Recovery, ExecError> {
    let mut recovery = Recovery::default();
    let listing = match std::fs::read_dir(state.join(JOURNALS_DIR)) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(recovery),
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
    for path in paths {
        // A live run holds its lock for its whole life; one that finished removed its journal
        // before its lock, so a journal gone once the lock is held settled on its own (larry#456).
        let Some(guard) = RunLock::probe(&path)? else {
            continue;
        };
        if !path.try_exists().map_err(ExecError::Journals)? {
            continue;
        }
        let kept_as = match recover_one(&path, policy) {
            Ok(()) => RECOVERED_EXT,
            Err(RuntimeError::Durable(JournalError::Locked { .. })) => continue,
            Err(refusal) if refusal.refuses_the_record() => REFUSED_EXT,
            Err(cause) => {
                let kept = KeptJournal { path, cause };
                kept.report();
                recovery.kept.push(kept);
                continue;
            }
        };
        std::fs::rename(&path, path.with_extension(kept_as)).map_err(ExecError::Journals)?;
        // The dead run's lock goes with its journal, so its evidence is one entry.
        if let Some(lock) = guard {
            lock.release()?;
        }
        recovery.set_aside = recovery.set_aside.saturating_add(1);
    }
    Ok(recovery)
}

/// Seeded adversarial simulations of the recovery (larry#182 defect 3).
#[cfg(test)]
#[path = "sim_recovery.rs"]
mod sim;
