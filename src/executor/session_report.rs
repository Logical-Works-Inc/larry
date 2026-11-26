use lgwks_std::json::{Value, json};

use super::island::{Island, Isolation, Outcome};
use super::session::Steps;
use super::session_gate::SessionIdentity;
use super::workspace::report_members;

/// Builds the sealed session body after the typed finish gate has decided the stop.
pub(super) fn body(
    identity: &SessionIdentity<'_>,
    done: Steps,
    route: &Value,
    isolation: Isolation,
    island: Option<&Island>,
    outcome: Outcome,
    diff: Option<super::workspace::Diff>,
) -> Value {
    let mut body = json!({
        "task_id": identity.task_id,
        "task_digest": identity.task_digest,
        "diff": null,
        "results": done.records,
        "denied": [],
        "session": {
            "stop": done.stop.name(),
            "commands": done.records.len(),
            "declined": done.declined,
            "interrupted": done.interrupted,
            "route": route,
            "compaction": done.compaction,
            "isolation": match island {
                Some(isle) => isle.describe(),
                None => json!({ "mode": isolation.name() }),
            },
            "handoff": outcome.name(),
        },
    });
    if let Some(object) = body.as_object_mut()
        && let Some(verification) = done.verification
    {
        object.insert(String::from("verification"), verification);
    }
    if includes_island_members(island)
        && let Some(object) = body.as_object_mut()
    {
        for (name, member) in report_members(diff) {
            object.insert(name.to_owned(), member);
        }
    }
    body
}

/// Island-only report members are present only for an isolated run.
fn includes_island_members(island: Option<&Island>) -> bool {
    island.is_some()
}

#[cfg(test)]
mod tests {
    use super::includes_island_members;

    #[test]
    fn host_reports_do_not_claim_island_members() {
        assert!(!includes_island_members(None));
    }
}
