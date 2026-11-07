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

use std::fmt;
use std::fs;
use std::io;
use std::num::TryFromIntError;
use std::path::PathBuf;
use std::time::Instant;

use crate::executor::{AuthorFault, ExecError, Report, Settings, Task, execute};
use score::{Ending, Outcome, card};
pub use score::{PriceError, PriceFault, Prices, Scorecard};
use set::{EvalTask, TaskSet};
pub use set::{MAX_SET_BYTES, SetError};

/// Why `larry eval` stopped without a scorecard.
#[derive(Debug)]
#[non_exhaustive]
pub enum EvalError {
    /// The task set was refused before any task ran.
    Set(SetError),
    /// No `LARRY_MODEL_KEY` is set, so no arm is configured.
    ModelUnconfigured,
    /// The state directory is not UTF-8, so no task document can name an origin in it.
    StatePath(PathBuf),
    /// An origin directory or file could not be written or removed.
    Origin {
        /// The task.
        id: String,
        /// The cause.
        cause: io::Error,
    },
    /// The executor failed on something that is not the arm's doing (the ledger, git,
    /// a process group): the run stops rather than score infrastructure as the model.
    Exec {
        /// The task.
        id: String,
        /// The cause.
        cause: ExecError,
    },
    /// A task ran longer than `u64::MAX` microseconds.
    Overlong {
        /// The task.
        id: String,
        /// The conversion that refused the duration.
        cause: TryFromIntError,
    },
}

impl fmt::Display for EvalError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Set(ref cause) => write!(out, "eval set: {cause}"),
            Self::ModelUnconfigured => {
                out.write_str("eval LARRY_MODEL_KEY is unset: an arm needs its model")
            }
            Self::StatePath(ref path) => {
                write!(out, "eval state directory is not UTF-8: {}", path.display())
            }
            Self::Origin { ref id, ref cause } => write!(out, "eval origin of {id}: {cause}"),
            Self::Exec { ref id, ref cause } => write!(out, "eval task {id}: {cause}"),
            Self::Overlong { ref id, ref cause } => {
                write!(out, "eval task {id} ran too long to time: {cause}")
            }
        }
    }
}

impl std::error::Error for EvalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Set(ref cause) => Some(cause),
            Self::Origin { ref cause, .. } => Some(cause),
            Self::Exec { ref cause, .. } => Some(cause),
            Self::Overlong { ref cause, .. } => Some(cause),
            Self::ModelUnconfigured | Self::StatePath(_) => None,
        }
    }
}

/// One task's origin repository; removed when dropped, whatever the run did.
struct Origin {
    /// `<state>/eval/origin-<pid>-<index>`.
    dir: PathBuf,
}

impl Origin {
    /// Writes `task`'s files into a fresh directory and commits them.
    fn create(settings: &Settings, index: usize, task: &EvalTask) -> Result<Origin, EvalError> {
        let failed = |cause: io::Error| EvalError::Origin {
            id: task.id.clone(),
            cause,
        };
        let dir = settings
            .state_dir()
            .join("eval")
            .join(format!("origin-{}-{index}", std::process::id()));
        match fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {}
            Err(cause) => return Err(failed(cause)),
        }
        let origin = Origin { dir };
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

impl Drop for Origin {
    fn drop(&mut self) {
        // A removal that fails leaves a directory the next run's sweep collects.
        drop(fs::remove_dir_all(&self.dir));
    }
}

/// The prompt and completion tokens a sealed report's model step spent, when every
/// completion reported both.
fn tokens(report: &Report) -> Option<(u64, u64)> {
    let records = report.json()["authored"].as_array()?;
    records
        .iter()
        .try_fold((0_u64, 0_u64), |(spent_in, spent_out), record| {
            Some((
                spent_in.saturating_add(record["prompt_tokens"].as_u64()?),
                spent_out.saturating_add(record["completion_tokens"].as_u64()?),
            ))
        })
}

/// Runs task `index` of the set and records how it ended.
fn run_one(settings: &Settings, index: usize, task: &EvalTask) -> Result<Outcome, EvalError> {
    let origin = Origin::create(settings, index, task)?;
    let repo = origin
        .dir
        .to_str()
        .ok_or_else(|| EvalError::StatePath(origin.dir.clone()))?;
    let document: Task = task.task(repo).map_err(|cause| EvalError::Exec {
        id: task.id.clone(),
        cause: ExecError::Task(cause),
    })?;
    let started = Instant::now();
    let finished = execute(&document, settings);
    let micros =
        u64::try_from(started.elapsed().as_micros()).map_err(|cause| EvalError::Overlong {
            id: task.id.clone(),
            cause,
        })?;
    let (ending, spent) = match finished {
        Ok(report) => {
            let ending = if report.accepted() {
                Ending::Accepted
            } else {
                Ending::Rejected
            };
            (ending, tokens(&report))
        }
        // Only the gateway's refusal is the arm's; a symlink or a write failure is the set's
        // or the machine's, and stops the run below.
        Err(
            error @ ExecError::Author {
                fault: AuthorFault::Model(_),
                ..
            },
        ) => (Ending::Refused(error.to_string()), None),
        Err(cause) => {
            return Err(EvalError::Exec {
                id: task.id.clone(),
                cause,
            });
        }
    };
    Ok(Outcome {
        id: task.id.clone(),
        ending,
        micros,
        tokens: spent,
    })
}

/// Runs every task of the set in `bytes` as one arm and scores it. The whole set is
/// checked first; a refused set, an unconfigured model or an infrastructure failure
/// stops with a typed error, while a model refusal is the arm's and is scored.
pub fn run(bytes: &[u8], settings: &Settings, prices: Prices) -> Result<Scorecard, EvalError> {
    let set = TaskSet::parse(bytes).map_err(EvalError::Set)?;
    let model = settings.model_name().ok_or(EvalError::ModelUnconfigured)?;
    settings.sweep_origins().map_err(|cause| EvalError::Exec {
        id: String::from("(sweep)"),
        cause,
    })?;
    let mut outcomes = Vec::with_capacity(set.tasks.len());
    for (index, task) in set.tasks.iter().enumerate() {
        outcomes.push(run_one(settings, index, task)?);
    }
    Ok(card(&set.name, model, prices, &outcomes))
}

#[cfg(test)]
#[path = "sim_eval.rs"]
mod sim;
