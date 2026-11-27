//! Whether a task's acceptance commands run confined (larry#324, ADR 0088): a brief means the
//! model wrote the tree the command executes, and `larry_submit_task` marks its own task because
//! a remote peer drove the run and no person watches the command. A task with no brief through
//! `larry exec` stays open, where the caller's own command is the only one in the loop.

use super::Task;

impl Task {
    /// Whether acceptance commands run confined: a brief, or the MCP door's flag (larry#324).
    pub(crate) fn confines_acceptance(&self) -> bool {
        self.brief.is_some() || self.confine_acceptance
    }

    /// This task with acceptance confined even without a brief (larry#324).
    pub(crate) fn confining_acceptance(mut self) -> Self {
        self.confine_acceptance = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::super::Knobs;
    use super::Task;

    /// larry#324: a brief, or the MCP door's flag, confines the acceptance commands; a plain
    /// `larry exec` task stays open, and `larry eval`'s briefed task is confined by its brief.
    #[test]
    fn a_brief_or_an_mcp_task_is_confined_and_a_plain_task_is_not()
    -> Result<(), Box<dyn std::error::Error>> {
        let plain = Task::parse(br#"{"repo":"/tmp/repo","base":"HEAD","acceptance":[["true"]]}"#)?;
        assert!(!plain.confines_acceptance(), "a plain exec task stays open");
        let briefed = Task::parse(
            br#"{"repo":"/tmp/repo","base":"HEAD","goal":"g","files":["f"],"acceptance":[["true"]]}"#,
        )?;
        assert!(
            briefed.confines_acceptance(),
            "a brief means the model wrote the tree"
        );
        let mcp = Task::shell_via("/tmp/repo", "g", &["f"], &["true"], None, Knobs::default())?
            .confining_acceptance();
        assert!(
            mcp.confines_acceptance(),
            "larry_submit_task confines its task"
        );
        Ok(())
    }
}
