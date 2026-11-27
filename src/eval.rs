//! `larry eval` (larry#87): one arm of the matched comparison, Claude alone
//! against Claude plus Flash. Every task of one set runs through the executor
//! exactly as `larry_submit_task` runs it (ADR 0008): its origin repository is
//! written and committed under the state directory, the arm's model writes the
//! named files, the acceptance commands run and the report is sealed. The arm is
//! the model `LARRY_MODEL_*` pins, priced by `LARRY_EVAL_PRICE_IN` and
//! `LARRY_EVAL_PRICE_OUT`; the scorecard prints cost per accepted task, the
//! acceptance count and p50/p95/p99 wall time, and the comparison is two arms
//! over one set (ADR 0009). Model money is spent only by the arm an operator runs.

mod score;
mod set;

use std::fs::{self, File};
use std::io::{self, Write as _};
use std::num::TryFromIntError;
use std::path::PathBuf;
use std::time::Instant;

use lgwks_macros::Error;

use crate::executor::{
    Approval, AuthorFault, ExecError, Isolation, Ledger, Observer, Place, Report, RunHooks,
    SessionDoor, Settings, Stop, Task, execute_eval, run_command, run_session,
};
use crate::executor::{hold_origin, origin_lock_path};
use crate::model::RouteError;
#[cfg(test)]
pub(crate) use score::percentile;
use score::{Ending, Outcome, Spent, card};
pub use score::{PriceError, PriceFault, Prices, Scorecard};
use set::{EvalPath, EvalTask, TaskSet};
pub use set::{MAX_SET_BYTES, SetError};

/// Why `larry eval` stopped without a scorecard.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EvalError {
    /// The task set was refused before any task ran.
    #[error("eval set: {0}")]
    Set(#[source] SetError),
    /// No model route is configured (no tenant default and no `LARRY_MODEL_*`
    /// environment route), so no arm is configured.
    #[error(
        "eval has no model route (larry route add, or LARRY_MODEL_URL, LARRY_MODEL and LARRY_MODEL_KEY): an arm needs its model"
    )]
    ModelUnconfigured,
    /// The state directory is not UTF-8, so no task document can name an origin in it.
    #[error("eval state directory is not UTF-8: {}", __field_0.display())]
    StatePath(PathBuf),
    /// An origin directory or file could not be written or removed.
    #[error("eval origin of {id}: {cause}")]
    Origin {
        /// The task.
        id: String,
        /// The cause.
        #[source]
        cause: io::Error,
    },
    /// The executor failed on something that is not the arm's doing (the ledger, git,
    /// a process group): the run stops rather than score infrastructure as the model.
    #[error("eval task {id}: {cause}")]
    Exec {
        /// The task.
        id: String,
        /// The cause.
        #[source]
        cause: ExecError,
    },
    /// A task ran longer than `u64::MAX` microseconds.
    #[error("eval task {id} ran too long to time: {cause}")]
    Overlong {
        /// The task.
        id: String,
        /// The conversion that refused the duration.
        #[source]
        cause: TryFromIntError,
    },
    /// A fault outside the arm's doing stopped it partway (larry#399): the card of the
    /// outcomes scored before it is kept, marked partial, and `cause` is why it stopped.
    #[error("{cause}; the partial card keeps the outcomes before it")]
    Stopped {
        /// The partial card: `complete` false, `stopped` naming the task and the cause.
        card: Box<Scorecard>,
        /// The task's own failure.
        #[source]
        cause: Box<EvalError>,
    },
}

impl EvalError {
    /// The partial card of an arm a fault stopped partway, `None` for a run that scored
    /// nothing (larry#399).
    #[must_use]
    pub fn card(&self) -> Option<&Scorecard> {
        match *self {
            Self::Stopped { ref card, .. } => Some(card),
            Self::Set(_)
            | Self::ModelUnconfigured
            | Self::StatePath(_)
            | Self::Origin { .. }
            | Self::Exec { .. }
            | Self::Overlong { .. } => None,
        }
    }
}

/// One task's origin repository; removed by `remove` once the run ends. The origin's
/// lock is held for its whole life, so a sweep by another `larry eval` keeps it (larry#214);
/// an origin a crashed or failed run left behind is unlocked and the next sweep collects it.
struct Origin {
    /// `<state>/eval/origin-<pid>-<index>`.
    dir: PathBuf,
    /// The lock file beside `dir`, removed with it.
    lock: PathBuf,
    /// The held lock; the kernel releases it when this process dies.
    _held: File,
    /// Whether `remove` ran, so `Drop` does not attempt the cleanup twice.
    cleaned: bool,
}

impl Origin {
    /// Writes `task`'s files into a fresh directory and commits them.
    fn create(settings: &Settings, index: usize, task: &EvalTask) -> Result<Origin, EvalError> {
        let failed = |cause: io::Error| EvalError::Origin {
            id: task.id.clone(),
            cause,
        };
        let eval = settings.state_dir().join("eval");
        let name = format!("origin-{}-{index}", std::process::id());
        let dir = eval.join(&name);
        fs::create_dir_all(&eval).map_err(failed)?;
        // The lock is taken before the directory exists, so no sweep can see an unlocked origin.
        let held = hold_origin(&eval, &name).map_err(|cause| EvalError::Exec {
            id: task.id.clone(),
            cause,
        })?;
        match fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => return Err(failed(cause)),
        }
        let origin = Origin {
            lock: origin_lock_path(&eval, &name),
            dir,
            _held: held,
            cleaned: false,
        };
        for file in &task.files {
            let target = origin.dir.join(&file.0);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(failed)?;
            }
            fs::write(&target, &file.1).map_err(failed)?;
        }
        settings
            .seed_origin(&origin.dir)
            .map_err(|cause| EvalError::Exec {
                id: task.id.clone(),
                cause,
            })?;
        Ok(origin)
    }
}

impl Origin {
    /// Removes the origin directory and then its lock file, and releases the lock.
    /// Both removals are attempted; the first fault is the one reported, and a
    /// leftover directory is collected by the next run's sweep.
    ///
    /// # Errors
    ///
    /// The directory or the lock file could not be removed for a reason other
    /// than already being gone.
    fn remove(mut self) -> io::Result<()> {
        self.cleaned = true;
        self.cleanup()
    }

    /// Removes the directory and then its lock file; a name already gone has done its job.
    fn cleanup(&self) -> io::Result<()> {
        let directory = absent_is_removed(fs::remove_dir_all(&self.dir));
        let lock = absent_is_removed(fs::remove_file(&self.lock));
        directory?;
        lock
    }
}

impl Drop for Origin {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        self.cleaned = true;
        // A partial origin from a refused write or seed is removed here; anything this
        // cannot remove is unlocked, so the next run's sweep owns it and reports it
        // (larry#343).
        if let Err(cause) = self.cleanup() {
            report_origin_kept(&self.dir, &cause);
        }
    }
}

/// Reports an origin left for the next sweep as one typed JSON line on standard error, the
/// operator's channel (larry#343).
fn report_origin_kept(entry: &std::path::Path, cause: &io::Error) {
    let event = lgwks_std::json::json!({
        "sweep": "origin-kept",
        "entry": format!("{}", entry.display()),
        "refused": cause.to_string(),
    });
    let mut err = io::stderr().lock();
    let written = writeln!(err, "{event}");
    drop(written);
}

/// A removal of something that is already gone has done its job; any other fault stands.
fn absent_is_removed(removal: io::Result<()>) -> io::Result<()> {
    match removal {
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// The tokens a sealed report's model step spent, when every completion reported its
/// prompt and completion counts. The cached and written counts sum the records that
/// reported them, and `cache_reported` says whether every record reported its cached
/// count (larry#230).
fn tokens(report: &Report) -> Option<Spent> {
    let records = report.json()["authored"].as_array()?;
    let mut spent = Spent {
        prompt: 0,
        completion: 0,
        cached: 0,
        written: 0,
        cache_reported: true,
    };
    for record in records {
        spent.prompt = spent
            .prompt
            .saturating_add(record["prompt_tokens"].as_u64()?);
        spent.completion = spent
            .completion
            .saturating_add(record["completion_tokens"].as_u64()?);
        match record["cached_tokens"].as_u64() {
            Some(count) => spent.cached = spent.cached.saturating_add(count),
            None => spent.cache_reported = false,
        }
        if let Some(count) = record["cache_write_tokens"].as_u64() {
            spent.written = spent.written.saturating_add(count);
        }
    }
    Some(spent)
}

/// Runs task `index` of the set and records how it ended, then removes its origin.
/// The run's own failure is reported before a failure to remove the origin.
fn run_one(settings: &Settings, index: usize, task: &EvalTask) -> Result<Outcome, EvalError> {
    let origin = Origin::create(settings, index, task)?;
    let ran = run_in(&origin, settings, task);
    finish_origin(origin, ran)
}

/// Removes the origin, reporting a fault the next sweep owns, and returns the run's own
/// result: a cleanup failure never aborts the arm (larry#343).
fn finish_origin(origin: Origin, ran: Result<Outcome, EvalError>) -> Result<Outcome, EvalError> {
    let entry = origin.dir.clone();
    if let Err(cause) = origin.remove() {
        report_origin_kept(&entry, &cause);
    }
    ran
}

/// An eval session approves every model command so the measured path is the same one Larry ships.
struct EvalObserver;

impl Observer for EvalObserver {
    fn approve(&mut self, _step: usize, _command: &str) -> Approval {
        Approval::Run
    }
}

/// Runs the task's acceptance commands in the origin the session worked in, each as
/// `sh -c` through the executor's command runner, the same way the command path runs
/// them: the session is scored by the set's verifier, never by its own commands
/// (larry#217).
fn session_acceptance_passed(
    origin: &Origin,
    settings: &Settings,
    task: &EvalTask,
) -> Result<bool, EvalError> {
    let ledger = Ledger::open(settings.state_dir()).map_err(|cause| EvalError::Exec {
        id: task.id.clone(),
        cause,
    })?;
    let run = format!("{}-acceptance", task.id);
    for command in &task.acceptance {
        let argv = [String::from("sh"), String::from("-c"), command.clone()];
        let result = run_command(
            &argv,
            Place::Open(&origin.dir),
            settings.path_env(),
            settings.timeout(),
            &ledger,
            &run,
            RunHooks::new(&|| false, None),
        )
        .map_err(|cause| EvalError::Exec {
            id: task.id.clone(),
            cause,
        })?;
        if result.exit_code() != Some(0) || result.timed_out() || result.interrupted() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Runs the task's brief path and classifies its sealed report.
fn command_outcome(
    task: &EvalTask,
    document: &Task,
    settings: &Settings,
) -> Result<(Ending, Option<Spent>), EvalError> {
    match execute_eval(document, settings) {
        Ok(report) => {
            let ending = if report.accepted() {
                Ending::Accepted
            } else {
                Ending::Rejected
            };
            Ok((ending, tokens(&report)))
        }
        Err(
            error @ ExecError::Author {
                fault: AuthorFault::Model(_) | AuthorFault::EmptyReply,
                ..
            },
        ) => Ok((Ending::Refused(error.to_string()), None)),
        Err(cause) => Err(EvalError::Exec {
            id: task.id.clone(),
            cause,
        }),
    }
}

/// Runs the task's terminal session path, then its acceptance commands in the
/// session's origin, and classifies the outcome by the set's verifier.
fn session_outcome(
    origin: &Origin,
    settings: &Settings,
    task: &EvalTask,
) -> Result<(Ending, Option<Spent>), EvalError> {
    let mut observer = EvalObserver;
    match run_session(
        &task.goal,
        &origin.dir,
        settings,
        Isolation::Host,
        &mut observer,
        SessionDoor::Eval,
    ) {
        Ok(ended) => {
            let ending = match ended.stop {
                Stop::EndTurn if session_acceptance_passed(origin, settings, task)? => {
                    Ending::Accepted
                }
                Stop::Refusal(ref cause) => Ending::Refused(cause.clone()),
                Stop::EndTurn | Stop::MaxTurnRequests | Stop::Cancelled => Ending::Rejected,
            };
            Ok((ending, tokens(&ended.report)))
        }
        Err(
            error @ ExecError::Author {
                fault: AuthorFault::Model(_) | AuthorFault::EmptyReply,
                ..
            },
        ) => Ok((Ending::Refused(error.to_string()), None)),
        Err(cause) => Err(EvalError::Exec {
            id: task.id.clone(),
            cause,
        }),
    }
}

/// Runs `task` in its written `origin` and records how it ended.
fn run_in(origin: &Origin, settings: &Settings, task: &EvalTask) -> Result<Outcome, EvalError> {
    let repo = origin
        .dir
        .to_str()
        .ok_or_else(|| EvalError::StatePath(origin.dir.clone()))?;
    let started = Instant::now();
    let (ending, spent) = match task.path {
        EvalPath::Command => {
            let document: Task = task.task(repo).map_err(|cause| EvalError::Exec {
                id: task.id.clone(),
                cause: ExecError::Task(cause),
            })?;
            command_outcome(task, &document, settings)?
        }
        EvalPath::Session => session_outcome(origin, settings, task)?,
    };
    let micros =
        u64::try_from(started.elapsed().as_micros()).map_err(|cause| EvalError::Overlong {
            id: task.id.clone(),
            cause,
        })?;
    Ok(Outcome {
        id: task.id.clone(),
        path: task.path.name(),
        ending,
        micros,
        tokens: spent,
    })
}

/// Runs every task of the set in `bytes` as one arm and scores it. The whole set is
/// checked first; a refused set or an unconfigured model stops with a typed error before
/// anything is spent, and a model refusal is the arm's and is scored. An infrastructure
/// failure ends the arm at its task with [`EvalError::Stopped`], which keeps the card of
/// every outcome before it, since those were paid for (larry#399).
pub fn run(bytes: &[u8], settings: &Settings, prices: Prices) -> Result<Scorecard, EvalError> {
    let set = TaskSet::parse(bytes).map_err(EvalError::Set)?;
    let route = match settings.arm_route() {
        Ok(route) => route,
        Err(ExecError::Route(RouteError::NoRoute)) => return Err(EvalError::ModelUnconfigured),
        Err(cause) => {
            return Err(EvalError::Exec {
                id: String::from("(route)"),
                cause,
            });
        }
    };
    settings.sweep_origins().map_err(|cause| EvalError::Exec {
        id: String::from("(sweep)"),
        cause,
    })?;
    let mut outcomes = Vec::with_capacity(set.tasks.len());
    for (index, task) in set.tasks.iter().enumerate() {
        match run_one(settings, index, task) {
            Ok(outcome) => outcomes.push(outcome),
            Err(cause) => {
                let partial = card(&set.name, route.model(), prices, &outcomes)
                    .with_route(route.id())
                    .stopped(&task.id, &cause.to_string());
                return Err(EvalError::Stopped {
                    card: Box::new(partial),
                    cause: Box::new(cause),
                });
            }
        }
    }
    Ok(card(&set.name, route.model(), prices, &outcomes).with_route(route.id()))
}

#[cfg(test)]
#[path = "eval/sim_eval.rs"]
mod sim;

#[cfg(test)]
#[path = "eval/sim_set.rs"]
mod sim_set;
