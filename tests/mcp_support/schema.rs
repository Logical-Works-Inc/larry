//! Checks the server's replies against the official MCP JSON Schemas.
//!
//! Source: `schema/2026-07-28/schema.json` and `schema/2025-11-25/schema.json`
//! of modelcontextprotocol/modelcontextprotocol at tag 2026-07-28, vendored in
//! `tests/fixtures/mcp/` with their URL in a `$comment`. The validator covers
//! the JSON Schema 2020-12 keywords those files use and refuses any other
//! constraint keyword it meets, so a schema change cannot pass unchecked.

use lgwks_std::json::{self, Value};
use std::path::Path;

/// A check's outcome: the path and reason of the first violation.
type Checked = Result<(), String>;

/// Deepest `$ref` and value nesting followed before the check gives up.
const MAX_DEPTH: usize = 96;

/// Keywords that annotate and never constrain a value.
const ANNOTATIONS: [&str; 9] = [
    "description",
    "$comment",
    "title",
    "default",
    "format",
    "enumNames",
    "examples",
    "$schema",
    "$defs",
];

/// Reply error codes and the schema definition that types each one's body.
const ERROR_BODIES: [(i64, &str); 5] = [
    (-32700, "ParseError"),
    (-32600, "InvalidRequestError"),
    (-32601, "MethodNotFoundError"),
    (-32602, "InvalidParamsError"),
    (-32603, "InternalError"),
];

/// Request methods and the schema definition that types their result.
const RESULT_BODIES: [(&str, &str); 5] = [
    ("initialize", "InitializeResult"),
    ("server/discover", "DiscoverResult"),
    ("tools/list", "ListToolsResult"),
    ("tools/call", "CallToolResult"),
    ("ping", "EmptyResult"),
];

/// One vendored official schema.
pub struct Schema {
    /// The parsed `schema.json`, whose `$defs` every `$ref` resolves into.
    root: Value,
}

impl Schema {
    /// Loads the schema of revision `tag` (`2026-07-28` or `2025-11-25`).
    pub fn vendored(tag: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mcp")
            .join(format!("schema-{tag}.json"));
        Ok(Self {
            root: json::from_slice(&std::fs::read(path)?)?,
        })
    }

    /// Checks one reply to a request of `method`: the JSON-RPC envelope, then
    /// the result or error body the schema defines for it.
    pub fn check_reply(&self, method: &str, reply: &Value) -> Checked {
        if let Some(error) = reply.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .ok_or("error.code is not an integer")?;
            if code == -32022 {
                return self.check_def("UnsupportedProtocolVersionError", reply);
            }
            self.check_def("JSONRPCErrorResponse", reply)?;
            // 2025-11-25 defines no per-code error bodies; its generic `Error`
            // (code integer, message string) was checked above.
            let body = ERROR_BODIES
                .iter()
                .find(|entry| entry.0 == code)
                .map(|entry| entry.1);
            return match body {
                Some(def) if self.defines(def) => self.check_def(def, error),
                Some(_) => Ok(()),
                None => Err(format!("error code {code} is not one this server may emit")),
            };
        }
        self.check_def("JSONRPCResultResponse", reply)?;
        let result = reply.get("result").ok_or("neither result nor error")?;
        let body = RESULT_BODIES
            .iter()
            .find(|entry| entry.0 == method)
            .map(|entry| entry.1);
        match body {
            Some(def) => self.check_def(def, result),
            None => Err(format!("no result schema for method {method}")),
        }
    }

    /// Whether the schema defines `def`.
    fn defines(&self, def: &str) -> bool {
        self.root.pointer(&format!("/$defs/{def}")).is_some()
    }

    /// Checks `value` against the schema definition named `def`.
    fn check_def(&self, def: &str, value: &Value) -> Checked {
        let schema = self
            .root
            .pointer(&format!("/$defs/{def}"))
            .ok_or_else(|| format!("the schema has no definition {def}"))?;
        self.judge(schema, value, def, 0)
    }

    /// Judges `value` against `schema`; `at` names the location in a failure.
    fn judge(&self, schema: &Value, value: &Value, at: &str, depth: usize) -> Checked {
        if depth > MAX_DEPTH {
            return Err(format!("{at}: nesting beyond {MAX_DEPTH}"));
        }
        let Some(keywords) = schema.as_object() else {
            return match schema.as_bool() {
                Some(true) => Ok(()),
                _ => Err(format!("{at}: refused by a false schema")),
            };
        };
        let next = depth.saturating_add(1);
        for (keyword, argument) in keywords {
            let key = keyword.as_str();
            if ANNOTATIONS.contains(&key) {
                continue;
            }
            match key {
                "$ref" => self.judge_ref(argument, value, at, next)?,
                "type" => check_type(argument, value, at)?,
                "const" if argument != value => return Err(format!("{at}: not the constant")),
                "enum" if !argument.as_array().is_some_and(|all| all.contains(value)) => {
                    return Err(format!("{at}: {value} is not an allowed value"));
                }
                "const" | "enum" => {}
                "properties" | "required" | "additionalProperties" => {}
                "items" => self.judge_items(argument, value, at, next)?,
                "allOf" => self.judge_all(argument, value, at, next)?,
                "anyOf" | "oneOf" => self.judge_some(key, argument, value, at, next)?,
                "minimum" | "maximum" | "minLength" | "maxLength" | "minItems" | "maxItems" => {
                    check_bound(key, argument, value, at)?;
                }
                other => return Err(format!("{at}: unsupported keyword {other}")),
            }
        }
        self.judge_object(schema, value, at, next)
    }

    /// `$ref` into this document's `$defs`.
    fn judge_ref(&self, target: &Value, value: &Value, at: &str, depth: usize) -> Checked {
        let name = target
            .as_str()
            .and_then(|text| text.strip_prefix("#/$defs/"))
            .ok_or_else(|| format!("{at}: unsupported $ref {target}"))?;
        let schema = self
            .root
            .pointer(&format!("/$defs/{name}"))
            .ok_or_else(|| format!("{at}: dangling $ref {name}"))?;
        self.judge(schema, value, name, depth)
    }

    /// `properties`, `required` and `additionalProperties` of an object schema.
    fn judge_object(&self, schema: &Value, value: &Value, at: &str, depth: usize) -> Checked {
        let Some(object) = value.as_object() else {
            return Ok(());
        };
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for name in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(name) {
                    return Err(format!("{at}: missing required `{name}`"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        let extra = schema.get("additionalProperties");
        for (name, member) in object {
            let place = format!("{at}.{name}");
            match properties.and_then(|known| known.get(name)) {
                Some(sub) => self.judge(sub, member, &place, depth)?,
                None => {
                    if let Some(sub) = extra {
                        self.judge(sub, member, &place, depth)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// `items`: every array element satisfies the one schema.
    fn judge_items(&self, schema: &Value, value: &Value, at: &str, depth: usize) -> Checked {
        let Some(items) = value.as_array() else {
            return Ok(());
        };
        for (index, item) in items.iter().enumerate() {
            self.judge(schema, item, &format!("{at}[{index}]"), depth)?;
        }
        Ok(())
    }

    /// `allOf`: every subschema holds.
    fn judge_all(&self, schemas: &Value, value: &Value, at: &str, depth: usize) -> Checked {
        for sub in schemas.as_array().ok_or("allOf is not an array")? {
            self.judge(sub, value, at, depth)?;
        }
        Ok(())
    }

    /// `anyOf` (one or more) and `oneOf` (exactly one) subschemas hold.
    fn judge_some(
        &self,
        key: &str,
        schemas: &Value,
        value: &Value,
        at: &str,
        depth: usize,
    ) -> Checked {
        let options = schemas.as_array().ok_or("anyOf/oneOf is not an array")?;
        let held = options
            .iter()
            .filter(|sub| self.judge(sub, value, at, depth).is_ok())
            .count();
        let enough = if key == "oneOf" { held == 1 } else { held >= 1 };
        if enough {
            Ok(())
        } else {
            Err(format!(
                "{at}: {held} of {} {key} branches hold",
                options.len()
            ))
        }
    }
}

/// `type`: a name or a list of names, one of which the value has.
fn check_type(argument: &Value, value: &Value, at: &str) -> Checked {
    let names: Vec<&str> = match argument.as_array() {
        Some(list) => list.iter().filter_map(Value::as_str).collect(),
        None => argument.as_str().into_iter().collect(),
    };
    let has = |name: &&str| match *name {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => false,
    };
    if names.iter().any(has) {
        Ok(())
    } else {
        Err(format!("{at}: {value} is not of type {names:?}"))
    }
}

/// The numeric and size bounds: `minimum`, `maximum`, `minLength`, `maxLength`,
/// `minItems` and `maxItems`.
fn check_bound(key: &str, argument: &Value, value: &Value, at: &str) -> Checked {
    let limit = argument.as_f64().ok_or("a bound is not a number")?;
    let measured = match key {
        "minimum" | "maximum" => value.as_f64(),
        "minLength" | "maxLength" => value
            .as_str()
            .and_then(|text| u32::try_from(text.chars().count()).ok())
            .map(f64::from),
        _ => value
            .as_array()
            .and_then(|items| u32::try_from(items.len()).ok())
            .map(f64::from),
    };
    let Some(size) = measured else {
        return Ok(());
    };
    let holds = if key.starts_with("min") {
        size >= limit
    } else {
        size <= limit
    };
    if holds {
        Ok(())
    } else {
        Err(format!("{at}: {size} breaks {key} {limit}"))
    }
}
