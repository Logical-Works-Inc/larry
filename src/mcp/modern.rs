//! The stateless era, MCP 2026-07-28.
//!
//! Written from the 2026-07-28 specification (Basic: `_meta`, error codes;
//! Versioning and Compatibility; Server: Discovery, Tools, Caching, Logging;
//! schema/2026-07-28/schema.json). There is no handshake and no state: every
//! request names its protocol version and client capabilities in `_meta`, the
//! server judges that request alone and never reads what an earlier one said.
//! Every result carries `resultType: "complete"` and the server identity in
//! `_meta`; `server/discover` and `tools/list` add `ttlMs` and `cacheScope`.
//! `ping` and `logging/setLevel` no longer exist, so they are method-not-found.
//! No `notifications/message` is ever sent: the spec forbids it without a
//! per-request `logLevel`, and this server emits no logs over the wire at all.

use super::{INSTRUCTIONS, MODERN_VERSION, capabilities, identity, supported_versions, wire};
use lgwks_std::json::{Map, Value, json};

/// `_meta` key: the protocol version of this request (required).
const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
/// `_meta` key: the client capabilities for this request (required).
const CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
/// `_meta` key: the client identity (optional).
const CLIENT_INFO_KEY: &str = "io.modelcontextprotocol/clientInfo";
/// `_meta` key: the minimum log level the client wants for this request (optional).
const LOG_LEVEL_KEY: &str = "io.modelcontextprotocol/logLevel";
/// Result `_meta` key: the server identity.
const SERVER_INFO_KEY: &str = "io.modelcontextprotocol/serverInfo";
/// The syslog levels of RFC 5424 that `LoggingLevel` names in the schema.
const LOG_LEVELS: [&str; 8] = [
    "debug",
    "info",
    "notice",
    "warning",
    "error",
    "critical",
    "alert",
    "emergency",
];
/// Freshness hint on cacheable results: the tool set is compiled into the
/// binary, so it changes only when the process is replaced.
const TTL_MS: u64 = 3_600_000;
/// Methods this era answers; anything else, `ping` included, is method-not-found.
pub(super) const METHODS: [&str; 3] = ["server/discover", "tools/list", "tools/call"];

/// The `_meta` object of a request's params, if it is one.
fn meta_of(params: Option<&Map<String, Value>>) -> Option<&Map<String, Value>> {
    params
        .and_then(|map| map.get("_meta"))
        .and_then(Value::as_object)
}

/// Whether the request opens itself as a 2026-07-28 request: its `_meta`
/// carries the protocol version key, whatever the value.
pub(super) fn is_stateless(params: Option<&Map<String, Value>>) -> bool {
    meta_of(params).is_some_and(|meta| meta.contains_key(VERSION_KEY))
}

/// The reply to a request that needs `_meta` and sent none, with no legacy
/// session to read it from: required fields missing is `-32602`.
pub(super) fn missing_meta(id: &Value) -> Value {
    wire::error(
        id,
        wire::INVALID_PARAMS,
        "params._meta must carry io.modelcontextprotocol/protocolVersion and io.modelcontextprotocol/clientCapabilities; legacy clients send initialize first",
    )
}

/// Answers one stateless request: version, then method, then the rest of `_meta`.
pub(super) fn request(id: &Value, method: &str, params: Option<&Map<String, Value>>) -> Value {
    let Some(meta) = meta_of(params) else {
        return missing_meta(id);
    };
    let Some(requested) = meta.get(VERSION_KEY).and_then(Value::as_str) else {
        return wire::error(
            id,
            wire::INVALID_PARAMS,
            "io.modelcontextprotocol/protocolVersion must be a string",
        );
    };
    if requested != MODERN_VERSION {
        return unsupported_version(id, requested);
    }
    if !METHODS.contains(&method) {
        return wire::error(id, wire::METHOD_NOT_FOUND, "method not found");
    }
    if let Err(message) = check_meta(meta) {
        return wire::error(id, wire::INVALID_PARAMS, message);
    }
    match method {
        "server/discover" => complete(id, discover(), true),
        "tools/list" => super::list_tools(id, params, &|body| finish(body, true)),
        _ => super::call_tool(id, params, &|body| finish(body, false)),
    }
}

/// `UnsupportedProtocolVersionError` (-32022): `data` lists every version this
/// process serves, the legacy ones reached through `initialize` included, and
/// echoes the one asked for (schema.json, UnsupportedProtocolVersionError).
fn unsupported_version(id: &Value, requested: &str) -> Value {
    wire::error_with_data(
        id,
        wire::UNSUPPORTED_PROTOCOL_VERSION,
        "Unsupported protocol version: only 2026-07-28 is served per request; legacy revisions are served after initialize",
        json!({ "supported": supported_versions(), "requested": requested }),
    )
}

/// Checks the rest of `_meta`: capabilities required, identity and log level optional.
fn check_meta(meta: &Map<String, Value>) -> Result<(), &'static str> {
    if !meta.get(CAPABILITIES_KEY).is_some_and(Value::is_object) {
        return Err("io.modelcontextprotocol/clientCapabilities is required and must be an object");
    }
    if let Some(info) = meta.get(CLIENT_INFO_KEY) {
        let named = |key: &str| info.get(key).is_some_and(Value::is_string);
        if !(named("name") && named("version")) {
            return Err("io.modelcontextprotocol/clientInfo needs a string name and version");
        }
    }
    if let Some(level) = meta.get(LOG_LEVEL_KEY)
        && !level
            .as_str()
            .is_some_and(|text| LOG_LEVELS.contains(&text))
    {
        return Err("io.modelcontextprotocol/logLevel is not a recognized log level");
    }
    Ok(())
}

/// The `server/discover` result body: versions, capabilities, identity text.
fn discover() -> Value {
    json!({
        "supportedVersions": supported_versions(),
        "capabilities": capabilities(),
        "instructions": INSTRUCTIONS
    })
}

/// Adds `resultType`, the server identity and, when cacheable, `ttlMs` and
/// `cacheScope` to a result body. The tool set is the same for every caller,
/// so the scope is `public`.
fn finish(mut body: Value, cacheable: bool) -> Value {
    if let Some(map) = body.as_object_mut() {
        map.insert(String::from("resultType"), json!("complete"));
        let mut meta = Map::new();
        meta.insert(String::from(SERVER_INFO_KEY), identity());
        map.insert(String::from("_meta"), Value::Object(meta));
        if cacheable {
            map.insert(String::from("ttlMs"), json!(TTL_MS));
            map.insert(String::from("cacheScope"), json!("public"));
        }
    }
    body
}

/// A finished result as a JSON-RPC reply.
fn complete(id: &Value, body: Value, cacheable: bool) -> Value {
    wire::result(id, finish(body, cacheable))
}

#[cfg(test)]
mod tests {
    use super::{check_meta, is_stateless, request};
    use lgwks_std::json::{Map, Value, json};

    /// A request's params as a map.
    fn params(value: &Value) -> Option<&Map<String, Value>> {
        value.as_object()
    }

    /// Source: MCP 2026-07-28 Basic, per-request protocol fields. A request is
    /// stateless exactly when its `_meta` carries the version key.
    #[test]
    fn only_the_version_key_opens_a_stateless_request() {
        let with = json!({"_meta": {"io.modelcontextprotocol/protocolVersion": 1}});
        let without = json!({"_meta": {"progressToken": 1}});
        assert!(is_stateless(params(&with)), "version key present");
        assert!(!is_stateless(params(&without)), "other keys do not count");
        assert!(!is_stateless(None), "no params");
    }

    /// Source: MCP 2026-07-28 Basic: a request missing a required field is -32602.
    #[test]
    fn each_required_meta_field_is_enforced_on_its_own() {
        let good = json!({"io.modelcontextprotocol/clientCapabilities": {}});
        assert!(
            good.as_object()
                .is_some_and(|meta| check_meta(meta).is_ok()),
            "capabilities alone suffice"
        );
        let bad = [
            json!({}),
            json!({"io.modelcontextprotocol/clientCapabilities": []}),
            json!({"io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": "n"}}),
            json!({"io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/logLevel": "loud"}),
        ];
        for case in &bad {
            assert!(
                case.as_object()
                    .is_some_and(|meta| check_meta(meta).is_err()),
                "{case} must be refused"
            );
        }
    }

    /// Source: MCP 2026-07-28 Versioning: the version is judged before the
    /// method, so `ping` on an older revision is an unsupported version, and
    /// `ping` on this one is not found.
    #[test]
    fn version_is_judged_before_method() {
        let id = json!(1);
        let at = |version: &str| {
            json!({"_meta": {"io.modelcontextprotocol/protocolVersion": version,
                "io.modelcontextprotocol/clientCapabilities": {}}})
        };
        let old = request(&id, "ping", params(&at("2025-11-25")));
        assert_eq!(old["error"]["code"], -32022, "older revision");
        let now = request(&id, "ping", params(&at("2026-07-28")));
        assert_eq!(now["error"]["code"], -32601, "removed method");
    }
}
