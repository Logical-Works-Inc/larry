//! The initialize lifecycle of MCP 2025-11-25, 2025-06-18, 2025-03-26 and 2024-11-05.
//!
//! Written from those revisions' Lifecycle pages and from 2026-07-28
//! Versioning, "Backward Compatibility with Initialization-Based Versions": a
//! dual-era server answers `initialize` with legacy semantics for the stdio
//! process, echoes the requested revision when it serves it and otherwise
//! offers the newest legacy one. Results keep the legacy shape (no `resultType`,
//! no cache hints); `ping` is still a legacy method.

use super::{INSTRUCTIONS, LATEST_LEGACY, LEGACY_VERSIONS, capabilities, identity, wire};
use lgwks_std::json::{Map, Value, json};

/// The legacy session of one stdio process: the whole lifecycle is one flag.
#[derive(Default)]
pub(super) struct Lifecycle {
    /// Set when an initialize request has been answered.
    initialized: bool,
}

impl Lifecycle {
    /// Whether an initialize has been answered, so legacy requests are served.
    pub(super) fn is_open(&self) -> bool {
        self.initialized
    }

    /// The handshake: negotiate a legacy revision and open the session.
    pub(super) fn initialize(&mut self, id: &Value, params: Option<&Map<String, Value>>) -> Value {
        if self.initialized {
            return wire::error(id, wire::INVALID_REQUEST, "already initialized");
        }
        let Some(requested) = params
            .and_then(|map| map.get("protocolVersion"))
            .and_then(Value::as_str)
        else {
            return wire::error(
                id,
                wire::INVALID_PARAMS,
                "initialize requires params.protocolVersion as a string",
            );
        };
        let negotiated = match LEGACY_VERSIONS
            .iter()
            .copied()
            .find(|version| *version == requested)
        {
            Some(version) => version,
            None => LATEST_LEGACY,
        };
        self.initialized = true;
        wire::result(
            id,
            json!({
                "protocolVersion": negotiated,
                "capabilities": capabilities(),
                "serverInfo": identity(),
                "instructions": INSTRUCTIONS
            }),
        )
    }
}

/// Answers one request of an open legacy session (and a pre-initialize `ping`).
pub(super) fn request(id: &Value, method: &str, params: Option<&Map<String, Value>>) -> Value {
    match method {
        "ping" => wire::result(id, json!({})),
        "tools/list" => super::list_tools(id, params, &|body| body),
        "tools/call" => super::call_tool(id, params, &|body| body),
        _ => wire::error(id, wire::METHOD_NOT_FOUND, "method not found"),
    }
}
