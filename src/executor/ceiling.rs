//! A tenant's ceilings on the tasks it runs (larry#439, ADR 0111). A tenant is its state
//! directory and the settings its executor reads, so its ceilings live in the one settings home,
//! [`super::Settings`]: the longest command timeout and whole-task deadline a task may name, the
//! shells its commands may run under, and the bases its clone may check out. A task past any of
//! them is refused before anything is cloned, and the deadline it was given caps each command
//! on one clock while it runs.

use std::time::Duration;

use lgwks_bot::kernel::clock::Clock;

use super::task::{Task, is_base, is_shell};
use super::{ExecError, MAX_TIMEOUT_MS};

/// Most shells one tenant may allow.
const MAX_SHELLS: usize = 16;

/// Most base patterns one tenant may allow.
const MAX_BASES: usize = 64;

/// The shells a tenant allows when it names none: the one a planner's task runs under.
const DEFAULT_SHELLS: &str = "sh";

/// The form a millisecond ceiling must take.
const MILLIS: &str = "milliseconds in 1..=86400000";

/// What one tenant lets its tasks name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Ceilings {
    /// The longest per-command timeout a task may name, in milliseconds.
    max_timeout_ms: u64,
    /// The longest whole-task deadline a task may name, in milliseconds; also the deadline of a
    /// task that names none.
    max_deadline_ms: u64,
    /// The shells a task's commands may run under.
    shells: Vec<String>,
    /// The bases a clone may check out, each exact or, ending in `*`, a prefix; `None` admits
    /// every base the task parser admits.
    bases: Option<Vec<String>>,
}

/// What one admitted task runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Limits {
    /// The longest one command may run.
    pub(super) timeout: Duration,
    /// The longest the whole task may run, in milliseconds.
    pub(super) deadline_ms: u64,
}

impl Ceilings {
    /// The ceilings `read` names, and the default command timeout under them: the
    /// `LARRY_EXEC_TIMEOUT_MS` the settings read (`explicit`), which may not pass the timeout
    /// ceiling, or else `fallback` cut to it.
    ///
    /// # Errors
    ///
    /// [`ExecError::BadCeiling`] naming the first variable that is not its form, and the
    /// timeout variable when an explicit default passes the ceiling.
    pub(super) fn from_env(
        read: &impl Fn(&str) -> Option<String>,
        explicit: Option<u64>,
        fallback: u64,
    ) -> Result<(Ceilings, Duration), ExecError> {
        let max_timeout_ms = millis(read, "LARRY_EXEC_MAX_TIMEOUT_MS")?;
        let max_deadline_ms = millis(read, "LARRY_EXEC_MAX_DEADLINE_MS")?;
        let default_ms = match explicit {
            Some(value) if value > max_timeout_ms => {
                return Err(ExecError::BadCeiling {
                    name: "LARRY_EXEC_TIMEOUT_MS",
                    expected: "at most LARRY_EXEC_MAX_TIMEOUT_MS",
                    value: value.to_string(),
                });
            }
            Some(value) => value,
            None => fallback.min(max_timeout_ms),
        };
        let shells = match read("LARRY_EXEC_SHELLS") {
            Some(text) => list(&text, "LARRY_EXEC_SHELLS", MAX_SHELLS, is_shell)?,
            None => vec![String::from(DEFAULT_SHELLS)],
        };
        let bases = match read("LARRY_EXEC_BASES") {
            Some(text) => Some(list(&text, "LARRY_EXEC_BASES", MAX_BASES, is_pattern)?),
            None => None,
        };
        let ceilings = Ceilings {
            max_timeout_ms,
            max_deadline_ms,
            shells,
            bases,
        };
        Ok((ceilings, Duration::from_millis(default_ms)))
    }

    /// What `task` runs under, with `default` its command timeout when it names none.
    ///
    /// # Errors
    ///
    /// [`ExecError::PastCeiling`] naming the first of the shell, base, timeout and deadline
    /// that passes its ceiling.
    pub(super) fn limits(&self, task: &Task, default: Duration) -> Result<Limits, ExecError> {
        if let Some(shell) = task.shell_program()
            && !self.shells.iter().any(|allowed| allowed == shell)
        {
            return Err(past("shell", shell, &self.shells.join(",")));
        }
        if let Some(ref bases) = self.bases
            && !bases.iter().any(|pattern| admits(pattern, task.base()))
        {
            return Err(past("base", task.base(), &bases.join(",")));
        }
        let timeout = match task.timeout_ms() {
            Some(value) if value > self.max_timeout_ms => {
                return Err(past(
                    "timeout_ms",
                    &value.to_string(),
                    &self.max_timeout_ms.to_string(),
                ));
            }
            Some(value) => Duration::from_millis(value),
            None => default,
        };
        let deadline_ms = match task.deadline_ms() {
            Some(value) if value > self.max_deadline_ms => {
                return Err(past(
                    "deadline_ms",
                    &value.to_string(),
                    &self.max_deadline_ms.to_string(),
                ));
            }
            Some(value) => value,
            None => self.max_deadline_ms,
        };
        Ok(Limits {
            timeout,
            deadline_ms,
        })
    }
}

/// Whether `entry` is a base pattern: a base, or a non-empty base stem followed by `*`.
fn is_pattern(entry: &str) -> bool {
    match entry.strip_suffix('*') {
        Some(stem) => is_base(stem),
        None => is_base(entry),
    }
}

/// Whether base `pattern` admits `base`: equal, or a prefix of it when `pattern` ends in `*`.
fn admits(pattern: &str, base: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(stem) => base.starts_with(stem),
        None => pattern == base,
    }
}

/// The refusal of a task that named `value` for `what`, past `ceiling`.
fn past(what: &'static str, value: &str, ceiling: &str) -> ExecError {
    ExecError::PastCeiling {
        what,
        value: value.to_owned(),
        ceiling: ceiling.to_owned(),
    }
}

/// The millisecond ceiling `name` holds: unset is the executor's own bound of one day.
fn millis(read: &impl Fn(&str) -> Option<String>, name: &'static str) -> Result<u64, ExecError> {
    let Some(value) = read(name) else {
        return Ok(MAX_TIMEOUT_MS);
    };
    match value.parse::<u64>() {
        Ok(parsed) if (1..=MAX_TIMEOUT_MS).contains(&parsed) => Ok(parsed),
        _ => Err(ExecError::BadCeiling {
            name,
            expected: MILLIS,
            value,
        }),
    }
}

/// The comma-separated entries of `text`, 1 to `max` of them, each one `valid` admits.
fn list(
    text: &str,
    name: &'static str,
    max: usize,
    valid: impl Fn(&str) -> bool,
) -> Result<Vec<String>, ExecError> {
    let entries: Vec<String> = text.split(',').map(str::to_owned).collect();
    if entries.len() > max || !entries.iter().all(|entry| valid(entry)) {
        return Err(ExecError::BadCeiling {
            name,
            expected: "a comma-separated list of names, no blank entry, no leading -",
            value: text.to_owned(),
        });
    }
    Ok(entries)
}

/// The whole-task deadline of one run, read on one clock: each command's timeout is cut to
/// what is left of it, and a command that cut ends short ends the task typed.
pub(super) struct Deadline<'clock> {
    /// The clock the deadline reads.
    clock: &'clock Clock,
    /// The clock reading at which the task's time is spent.
    until: Duration,
    /// The deadline the task was given, in milliseconds, for its refusal.
    deadline_ms: u64,
}

impl<'clock> Deadline<'clock> {
    /// A deadline `limits.deadline_ms` past `clock`'s reading now.
    pub(super) fn start(clock: &'clock Clock, limits: Limits) -> Deadline<'clock> {
        let budget = Duration::from_millis(limits.deadline_ms);
        let now = clock.now();
        // A reading so late that the deadline cannot be held is a deadline already spent.
        let until = match now.checked_add(budget) {
            Some(until) => until,
            None => now,
        };
        Deadline {
            clock,
            until,
            deadline_ms: limits.deadline_ms,
        }
    }

    /// The timeout the next command runs under: `timeout`, cut to what is left of the task's
    /// time, and whether the cut applied.
    ///
    /// # Errors
    ///
    /// [`ExecError::DeadlinePassed`] when nothing is left.
    pub(super) fn cap(&self, timeout: Duration) -> Result<(Duration, bool), ExecError> {
        let left = self.until.saturating_sub(self.clock.now());
        if left.is_zero() {
            return Err(self.passed());
        }
        Ok((timeout.min(left), left < timeout))
    }

    /// What one command's end means for the task: a command the deadline `capped` that then
    /// `timed_out` was ended by the deadline, so the task ends typed; a command that ran out its
    /// own timeout is that command's result, and the task goes on.
    ///
    /// # Errors
    ///
    /// [`ExecError::DeadlinePassed`] when the deadline ended the command.
    pub(super) const fn after(&self, capped: bool, timed_out: bool) -> Result<(), ExecError> {
        if capped && timed_out {
            return Err(self.passed());
        }
        Ok(())
    }

    /// The refusal of a task whose time is spent.
    const fn passed(&self) -> ExecError {
        ExecError::DeadlinePassed {
            deadline_ms: self.deadline_ms,
        }
    }
}

#[cfg(test)]
#[path = "sim_ceiling.rs"]
mod sim_ceiling;
