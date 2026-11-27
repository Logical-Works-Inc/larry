//! An isolated terminal session (larry#243, ADR 0046): the model's commands run in a
//! confined copy of the person's folder, and the folder changes only when the person
//! reads the diff and says yes.
//!
//! The chain is `run_session` -> [`Island::open`] (snapshot the folder into
//! `<state>/ws/<task_id>`, confine it, prove the confinement holds on this host) ->
//! each command through `run_command` with `Place::Confined` -> [`Island::hand_off`]
//! (the sealed diff, the person's answer, `git apply` of that same diff). A session
//! on a host with no confinement, or whose policy forbids sessions, starts nothing.
//! Source: the reference-monitor rule that the person's yes is the only way a change
//! crosses the boundary, and the isolation lines of ADR 0046.

use std::collections::BTreeSet;
use std::path::Path;

use larry_contracts::authority::Scope;
use lgwks_std::json::{Value, json};

use super::ExecError;
use super::Settings;
use super::confine::{Confine, Platform, supported};
use super::gate;
use super::run::Place;
use super::session::{Handoff, Observer};
use super::workspace::{Diff, Workspace, summary};

/// What the policy refusal tells a person whose commands would run on the host.
const HOST_REFUSED: &str =
    "model commands are off: LARRY_POLICY must name model.command to run them on this host";

/// What the policy refusal tells a person whose commands would run in a workspace copy.
const POLICY_REFUSED: &str = "the policy does not allow a session: LARRY_POLICY must allow file.read, file.write, process.spawn and model.invoke";

/// Where a session's commands run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Isolation {
    /// On the host, in the directory itself: only where an operator isolates them
    /// outside Larry and names `model.command` (`larry acp`, larry#93).
    Host,
    /// In a copy of the folder that the kernel confines each command to (larry#243).
    Workspace(Platform),
}

impl Isolation {
    /// Confinement on the platform this binary was built for.
    pub(crate) const fn workspace() -> Self {
        Self::Workspace(Platform::host())
    }

    /// The scopes the gate requires of a session under this isolation.
    pub(crate) fn required(self) -> BTreeSet<Scope> {
        match self {
            Self::Host => gate::session_scopes(),
            Self::Workspace(_) => gate::isolated_scopes(),
        }
    }

    /// The mode's name in a report.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Workspace(_) => "workspace",
        }
    }

    /// The refusal that would stop a session before it starts, or `None`: the policy
    /// lacks a scope this isolation requires, or the host cannot confine.
    pub(crate) fn blocked(self, settings: &Settings) -> Option<String> {
        if !self.required().is_subset(settings.policy()) {
            return Some(String::from(match self {
                Self::Host => HOST_REFUSED,
                Self::Workspace(_) => POLICY_REFUSED,
            }));
        }
        match self {
            Self::Host => None,
            Self::Workspace(platform) => supported(platform).err().map(|cause| cause.to_string()),
        }
    }

    /// Whether this isolation can start on this host, as the typed refusal.
    pub(crate) fn check(self) -> Result<(), ExecError> {
        match self {
            Self::Host => Ok(()),
            Self::Workspace(platform) => supported(platform).map_err(ExecError::Confine),
        }
    }
}

/// What became of the change a session made, as the report names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Outcome {
    /// There was nothing to offer: the session did not end by its own `done`, or it changed nothing.
    None,
    /// The person said yes and the change is in their folder.
    Applied,
    /// The person said no; their folder is as it was.
    Discarded,
    /// The change could not be offered or applied; their folder is as it was.
    Failed,
}

impl Outcome {
    /// The outcome's name in a report.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Applied => "applied",
            Self::Discarded => "discarded",
            Self::Failed => "failed",
        }
    }
}

/// One session's workspace copy and the confinement its commands run under.
pub(crate) struct Island {
    /// The copy, removed when the island is dropped.
    workspace: Workspace,
    /// The confinement every command runs behind.
    confine: Confine,
    /// The base commit's id, taken before any command ran: the diff is against it.
    base: String,
    /// The platform the confinement is for.
    platform: Platform,
    /// Entries the snapshot copied.
    files: usize,
    /// Content bytes the snapshot copied.
    bytes: u64,
}

impl Island {
    /// Copies the folder `origin` into the workspace named `task_id`, confines it, and
    /// proves on this host that a write beside it is refused, before any model call.
    /// The folder is only read.
    pub(crate) fn open(
        platform: Platform,
        settings: &Settings,
        task_id: &str,
        origin: &Path,
    ) -> Result<Island, ExecError> {
        let (workspace, copied) =
            Workspace::snapshot(settings.state_dir(), origin, task_id, settings.path_env())?;
        let confine = Confine::new(
            platform,
            settings.state_dir(),
            workspace.path(),
            &workspace.scratch(),
        )
        .map_err(ExecError::Confine)?;
        confine
            .probe(settings.path_env())
            .map_err(ExecError::Confine)?;
        let base = workspace.head()?;
        Ok(Island {
            workspace,
            confine,
            base,
            platform,
            files: copied.files,
            bytes: copied.bytes,
        })
    }

    /// Where the session's commands run.
    pub(crate) fn place(&self) -> Place<'_> {
        Place::Confined(&self.confine)
    }

    /// The change the commands made: the sealed diff against the base copy, `None` when
    /// the commands destroyed the work tree.
    pub(crate) fn diff(&self) -> Result<Option<Diff>, ExecError> {
        self.workspace.diff(&self.base)
    }

    /// The report's `session.isolation` member.
    pub(crate) fn describe(&self) -> Value {
        json!({
            "mode": "workspace",
            "platform": self.platform.name(),
            "files": self.files,
            "bytes": self.bytes,
        })
    }

    /// Shows the person `diff` and applies exactly it to `origin` only on their yes.
    /// A change that is empty is not offered; one cut at its bound is not asked about,
    /// since a person cannot read it whole and a part is not the change.
    pub(crate) fn hand_off(
        &self,
        origin: &Path,
        diff: Option<&Diff>,
        observer: &mut dyn Observer,
    ) -> Outcome {
        let Some(diff) = diff else {
            return Outcome::None;
        };
        if diff.truncated {
            observer.handed(
                "the change is over 1 MiB and cannot be applied whole; your folder is unchanged",
                false,
            );
            return Outcome::Failed;
        }
        if diff.text.trim().is_empty() {
            return Outcome::None;
        }
        observer.diff(&diff.text);
        match observer.review(&summary(diff)) {
            Handoff::Discard => {
                observer.handed("discarded; your folder is unchanged", false);
                Outcome::Discarded
            }
            Handoff::Apply => match self.workspace.apply(origin, diff) {
                Ok(()) => {
                    observer.handed("applied to your folder", true);
                    Outcome::Applied
                }
                Err(fault) => {
                    observer.handed(
                        &format!("not applied: {fault}; your folder is unchanged"),
                        false,
                    );
                    Outcome::Failed
                }
            },
        }
    }
}

/// Seeded simulations of an isolated session against a hostile model.
#[cfg(test)]
#[path = "sim_island.rs"]
mod sim;
