use std::io;

use lgwks_macros::Error;

/// Most paths one typed finish request may name.
pub(super) const MAX_FINISH_PATHS: usize = 64;
/// Largest path one typed finish request may name.
pub(super) const MAX_FINISH_PATH_BYTES: usize = 1024;

/// The model's typed completion request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FinishRequest {
    /// The summary shown after the second accepted request.
    pub(super) summary: String,
    /// The exact command whose fresh receipt proves the task.
    pub(super) check: String,
    /// Relative paths that must be present after the check.
    pub(super) artifacts: Vec<String>,
    /// State the operator should expect to remain.
    pub(super) state: Vec<String>,
    /// Servers the operator must keep running.
    pub(super) servers: Vec<String>,
}

/// Why a typed finish request cannot become a receipt.
#[derive(Debug, Error)]
pub(super) enum VerificationFault {
    /// No successful fresh-shell check is the last command.
    #[error("the named check has no fresh passing receipt after the last command")]
    NoFreshCheck,
    /// An artifact name could escape the session tree.
    #[error("artifact path {path} must be relative and normal")]
    BadArtifactPath {
        /// The refused path.
        path: String,
    },
    /// A named artifact is absent or unreadable.
    #[error("artifact path {path} is missing: {source}")]
    MissingArtifact {
        /// The refused path.
        path: String,
        /// The metadata failure.
        source: io::Error,
    },
    /// A named artifact is a symlink.
    #[error("artifact path {path} is a symlink")]
    SymlinkArtifact {
        /// The refused path.
        path: String,
    },
}

/// The two stable identities a session report carries.
pub(super) struct SessionIdentity<'a> {
    /// The run id.
    pub(super) task_id: &'a str,
    /// The input document digest.
    pub(super) task_digest: &'a str,
}

/// The first accepted finish shows the operator the state the model says will remain.
pub(super) fn checklist(request: &FinishRequest) -> String {
    format!(
        "verification checklist: artifacts [{}]; state left behind [{}]; servers to keep up [{}]. Repeat the same finish to end.",
        request.artifacts.join(", "),
        request.state.join(", "),
        request.servers.join(", "),
    )
}

/// A finish may name only the last successful fresh-shell command as its check.
pub(super) fn has_fresh_check(last_command: Option<&str>, last_passed: bool, check: &str) -> bool {
    last_passed && last_command == Some(check)
}

/// Invalidates a checklist after a command reaches the run path.
pub(super) fn invalidate_finish(first_finish: &mut Option<FinishRequest>) {
    *first_finish = None;
}

/// Stops a session after the refusal budget is exhausted.
pub(super) const fn reached_refusal_limit(refused: usize) -> bool {
    refused >= 3
}

/// Keeps a bounded model reply in the compacted conversation.
pub(super) fn kept_reply(reply: &str, max_reply_bytes: usize) -> String {
    if reply.len() > max_reply_bytes {
        format!(
            "(a reply of {} bytes, over the bound, was dropped)",
            reply.len()
        )
    } else {
        reply.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{FinishRequest, invalidate_finish, reached_refusal_limit};

    #[test]
    fn command_invalidates_checklist_handshake() {
        let request = FinishRequest {
            summary: String::from("done"),
            check: String::from("test -f result"),
            artifacts: vec![String::from("result")],
            state: vec![String::from("ready")],
            servers: vec![String::from("none")],
        };
        let mut first = Some(request);
        invalidate_finish(&mut first);
        assert!(first.is_none(), "a later command needs a new checklist");
    }

    #[test]
    fn fresh_check_requires_the_named_passing_command() {
        assert!(!super::has_fresh_check(Some("true"), false, "true"));
        assert!(!super::has_fresh_check(None, true, "true"));
        assert!(!super::has_fresh_check(
            Some("cargo fmt"),
            true,
            "cargo test"
        ));
        assert!(super::has_fresh_check(
            Some("cargo test"),
            true,
            "cargo test"
        ));
    }

    #[test]
    fn kept_reply_is_bounded() {
        assert_eq!(super::kept_reply("ok", 2), "ok");
        assert_eq!(
            super::kept_reply("okay", 2),
            "(a reply of 4 bytes, over the bound, was dropped)"
        );
    }

    #[test]
    fn refusal_budget_stops_at_the_limit() {
        assert!(!reached_refusal_limit(2));
        assert!(reached_refusal_limit(3));
        assert!(reached_refusal_limit(4));
    }
}
