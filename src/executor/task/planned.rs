//! The task a planner door renders (`larry mcp`, `larry eval`): one task document parsed by the
//! one parser, so every door shares one digest for the run (ADR 0008). The planner may set the
//! shell, base and per-command timeout; the tenant's ceilings bound them later (larry#439).

use super::{Task, TaskError};

/// The shell a planner's commands run under when its task names none (larry#439).
const DEFAULT_SHELL: &str = "sh";

/// The base a planner's clone checks out when its task names none.
const DEFAULT_BASE: &str = "HEAD";

/// What a planner door may set on the task it renders (larry#439).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Knobs<'arg> {
    /// The shell each command runs under, as `<shell> -c <command>`; `sh` when unset.
    pub(crate) shell: Option<&'arg str>,
    /// The base commit-ish the clone checks out; `HEAD` when unset.
    pub(crate) base: Option<&'arg str>,
    /// The longest one command may run, in milliseconds.
    pub(crate) timeout_ms: Option<u64>,
}

impl Task {
    /// The task `larry eval` hands over: `repo` at `HEAD`, the brief, and each acceptance
    /// command run as `sh -c <command>`.
    pub(crate) fn shell(
        repo: &str,
        goal: &str,
        files: &[&str],
        acceptance: &[&str],
    ) -> Result<Task, TaskError> {
        Task::shell_via(repo, goal, files, acceptance, None, Knobs::default())
    }

    /// [`Task::shell`] over the route `route` names, when it names one, and whatever `knobs`
    /// sets: the planner picks them, and the document carries each under the run's digest.
    pub(crate) fn shell_via(
        repo: &str,
        goal: &str,
        files: &[&str],
        acceptance: &[&str],
        route: Option<&str>,
        knobs: Knobs<'_>,
    ) -> Result<Task, TaskError> {
        let commands: Vec<lgwks_std::json::Value> = acceptance
            .iter()
            .map(|command| lgwks_std::json::json!([command]))
            .collect();
        let base = match knobs.base {
            Some(named) => named,
            None => DEFAULT_BASE,
        };
        let shell = match knobs.shell {
            Some(named) => named,
            None => DEFAULT_SHELL,
        };
        let mut document = lgwks_std::json::json!({
            "repo": repo,
            "base": base,
            "shell": shell,
            "goal": goal,
            "files": files,
            "acceptance": commands,
        });
        if let Some(object) = document.as_object_mut() {
            if let Some(named) = route {
                object.insert(String::from("route"), lgwks_std::json::json!(named));
            }
            if let Some(value) = knobs.timeout_ms {
                object.insert(String::from("timeout_ms"), lgwks_std::json::json!(value));
            }
        }
        Task::parse(document.to_string().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::{Knobs, Task};

    /// larry#439: a planner's task runs each command under `sh -c` at `HEAD` by default, and
    /// carries the shell, base and timeout the planner set, each read back exactly.
    #[test]
    fn a_planner_task_carries_its_knobs_or_the_defaults() -> Result<(), Box<dyn std::error::Error>>
    {
        let plain = Task::shell("/tmp/r", "g", &["f"], &["make test"])?;
        assert_eq!(plain.base(), "HEAD", "the default base");
        assert_eq!(plain.shell_program(), Some("sh"), "the default shell");
        let command = vec![String::from("make test")];
        assert_eq!(plain.argv_for(&command), ["sh", "-c", "make test"]);
        assert_eq!(plain.timeout_ms(), None, "no timeout by default");
        let knobs = Knobs {
            shell: Some("bash"),
            base: Some("v1.2"),
            timeout_ms: Some(30_000),
        };
        let set = Task::shell_via("/tmp/r", "g", &["f"], &["make test"], Some("local"), knobs)?;
        assert_eq!(set.base(), "v1.2", "the planner's base");
        assert_eq!(set.argv_for(&command), ["bash", "-c", "make test"]);
        assert_eq!(set.timeout_ms(), Some(30_000), "the planner's timeout");
        assert_eq!(set.route(), Some("local"), "the planner's route");
        Ok(())
    }
}
