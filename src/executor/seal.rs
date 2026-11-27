//! What one finished run seals: its results, the diff as it stands and, when the whole-task
//! deadline spent it, the spent deadline beside the results (larry#439).

use super::workspace::{Workspace, report_members};
use super::{ExecError, Report};

/// What one finished run seals: its identity, every result it kept, the workspace its diff is
/// taken from, the brief's records when it has them, and the spent deadline when one ended it.
pub(super) struct SealedRun<'a> {
    /// The run's identity and digest.
    pub(super) task_id: &'a str,
    /// The digest admission checked, recomputed where the report seals it.
    pub(super) task_digest: &'a str,
    /// Every result the run kept, the cut command's with `timed_out: true` when the deadline
    /// spent it.
    pub(super) results: Vec<lgwks_std::json::Value>,
    /// What the model wrote, when the task carried a brief.
    pub(super) authored: Option<Vec<lgwks_std::json::Value>>,
    /// The workspace the diff is taken from as it stands.
    pub(super) workspace: &'a Workspace,
    /// The base the diff is taken against.
    pub(super) base: &'a str,
    /// The spent whole-task deadline, when one ended the run.
    pub(super) deadline_ms: Option<u64>,
}

/// Seals `seal` under `key`: the results, the diff as it stands and, when the deadline spent the
/// run, the spent deadline beside the results, so a run that already wrote is accounted for
/// (larry#439). `accepted` derives false whenever the deadline member is present, and `larry
/// verify` re-derives it.
pub(super) fn seal_run(
    seal: SealedRun<'_>,
    key: &lgwks_std::sign::SecretKey,
) -> Result<Report, ExecError> {
    let diff = seal.workspace.diff(seal.base)?;
    let mut body = lgwks_std::json::json!({
        "task_id": seal.task_id,
        "task_digest": seal.task_digest,
        "results": seal.results,
        "denied": [],
    });
    if let Some(deadline_ms) = seal.deadline_ms
        && let Some(object) = body.as_object_mut()
    {
        object.insert(
            String::from("deadline_passed"),
            lgwks_std::json::json!({"deadline_ms": deadline_ms}),
        );
    }
    // The diff members name whether the diff is the whole change (larry#206).
    if let Some(object) = body.as_object_mut() {
        for (name, member) in report_members(diff) {
            object.insert(name.to_owned(), member);
        }
    }
    // Only a briefed run names what the model wrote, so a plain run's report keeps its shape.
    if let Some(records) = seal.authored
        && let Some(object) = body.as_object_mut()
    {
        object.insert(
            String::from("authored"),
            lgwks_std::json::Value::Array(records),
        );
    }
    Ok(Report::sealed(body, key))
}
