//! Which door refuses a stored route that points inside this host's network (larry#627).
//! Source: CWE-918 (server-side request forgery). Requirement proved: only a remote door
//! refuses it, typed and before the key is read; the local door goes on to the key.

use std::error::Error;

use super::sim_tree::{scratch, settings};
use super::{ExecError, Route, Settings};
use crate::model::{Draft, Kind, RouteError};

/// A stored route to `url`, checked as `larry route add` checks it.
fn stored(url: &str) -> Result<Route, RouteError> {
    Route::new(&Draft {
        id: "inward",
        kind: Kind::OpenAiChat,
        base_url: url,
        model: "model-1",
        key_env: "LARRY_KEY_X",
        temperature: None,
        request_bytes: None,
        residency: Some("CA"),
        prompt: None,
        describe: "",
    })
}

#[test]
fn sim_only_a_remote_door_refuses_a_private_destination() -> Result<(), Box<dyn Error>> {
    let state = scratch("destination-door")?;
    let local = settings(&state, None)?;
    let directory = state.to_string_lossy().into_owned();
    let path = std::env::var("PATH")?;
    let remote = Settings::remote_from_env(move |name| match name {
        "LARRY_STATE_DIR" => Some(directory.clone()),
        "PATH" => Some(path.clone()),
        _ => None,
    })?;
    for (url, private) in [
        ("https://10.0.0.1/v1", true),
        ("https://vault.internal/v1", true),
        ("https://gateway.example/v1", false),
    ] {
        let route = stored(url)?;
        let refused = matches!(
            remote.model_config(&route),
            Err(ExecError::Route(RouteError::PrivateDestination { .. }))
        );
        assert_eq!(refused, private, "{url}: remote");
        assert!(
            !matches!(local.model_config(&route), Err(ExecError::Route(_))),
            "{url}: local"
        );
    }
    Ok(())
}
