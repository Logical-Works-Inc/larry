//! The one way an ACP journey sets up its operator and session (larry#338): the
//! state directory, the seeded mock gateway, the policy, and one `larry acp`
//! child on those settings. Shared by `tests/acp_harbor_journey.rs`, so the
//! setup has one home.

use std::error::Error;
use std::ffi::OsString;
use std::path::Path;

use crate::acp_agent::Agent;
use crate::mock_model::{Config, Handle};

/// Every scope a session needs, `model.command` among them: what an operator
/// sets inside a Harbor task container.
pub(crate) const SESSION_POLICY: &str =
    "file.read,file.write,process.spawn,model.invoke,model.command,net.egress";

/// The settings an operator exports: the state directory, the mock gateway and,
/// when given, the policy.
pub(crate) fn operator(
    state: &Path,
    mock: &Handle,
    policy: Option<&str>,
) -> Vec<(&'static str, OsString)> {
    let mut env = vec![
        ("LARRY_STATE_DIR", state.as_os_str().to_owned()),
        (
            "LARRY_MODEL_URL",
            OsString::from(format!("http://{}/v1", mock.addr())),
        ),
        ("LARRY_MODEL", OsString::from("mock-model")),
        ("LARRY_MODEL_KEY", OsString::from("acp-key")),
        ("LARRY_MODEL_BACKOFF_MS", OsString::from("0")),
    ];
    if let Some(policy) = policy {
        env.push(("LARRY_POLICY", OsString::from(policy)));
    }
    env
}

/// Starts `larry acp` under `operator`'s settings.
pub(crate) fn agent(
    state: &Path,
    mock: &Handle,
    policy: Option<&str>,
) -> Result<Agent, Box<dyn Error>> {
    start(&operator(state, mock, policy))
}

/// Starts `larry acp` under exactly `env`.
pub(crate) fn start(env: &[(&str, OsString)]) -> Result<Agent, Box<dyn Error>> {
    let pairs: Vec<(&str, &std::ffi::OsStr)> = env
        .iter()
        .map(|pair| (pair.0, pair.1.as_os_str()))
        .collect();
    Agent::start(&pairs)
}

/// A mock whose replies are `contents`, in order.
pub(crate) fn scripted(seed: u64, contents: &[&str]) -> Config {
    Config {
        contents: contents.iter().map(|text| (*text).to_owned()).collect(),
        ..Config::seeded(seed)
    }
}
