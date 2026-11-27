//! Route accessors and the sealed route view.

use lgwks_std::json::{Value, json};

use super::{DEFAULT_PRICE_VERSION, Kind, Route, RouteError, tenant_namespace};

impl Route {
    /// The route id.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    /// The provider kind.
    pub(crate) const fn kind(&self) -> Kind {
        self.kind
    }
    /// The base URL, without a trailing slash.
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }
    /// The URL's host and port.
    pub(crate) fn host(&self) -> &str {
        &self.host
    }
    /// The name of the variable that holds the key.
    pub(crate) fn key_env(&self) -> &str {
        &self.key_env
    }
    /// The binding refusal of this route's key variable under `tenant`.
    pub(crate) fn key_env_refusal(&self, tenant: &str) -> Option<RouteError> {
        let prefix = tenant_namespace(tenant);
        if self
            .key_env
            .strip_prefix(&prefix)
            .is_some_and(|rest| !rest.is_empty())
        {
            return None;
        }
        Some(RouteError::KeyEnvNotThisTenants {
            variable: self.key_env.clone(),
            prefix,
        })
    }
    /// The model id.
    pub(crate) fn model(&self) -> &str {
        &self.model
    }
    /// The pinned sampling temperature, when there is one.
    pub(crate) const fn temperature(&self) -> Option<f64> {
        self.temperature
    }
    /// The request body bound, in bytes.
    pub(crate) const fn request_bytes(&self) -> usize {
        self.request_bytes
    }
    /// The country where the provider processes the data, when declared.
    pub(crate) fn residency(&self) -> Option<&str> {
        self.residency.as_deref()
    }
    /// The prompt template prepended to the system turn, when the route names one.
    pub(crate) fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }
    /// The BLAKE3 digest of the route's prompt template as hex, `None` when the
    /// route names none: what a sealed record carries instead of the text
    /// (larry#205).
    pub(crate) fn prompt_hash(&self) -> Option<String> {
        self.prompt.as_deref().map(|template| {
            let mut hasher = lgwks_std::hash::Hasher::new();
            hasher.update(template.as_bytes());
            hasher.finalize().to_hex()
        })
    }
    /// What the route is good for.
    pub(crate) fn describe(&self) -> &str {
        &self.describe
    }
    /// The route price-table revision.
    pub(crate) const fn price_version(&self) -> u32 {
        self.price_version
    }
    /// Prompt micro-dollars per million tokens from the route table.
    pub(crate) const fn price_input(&self) -> u64 {
        self.price_input
    }
    /// Completion micro-dollars per million tokens from the route table.
    pub(crate) const fn price_output(&self) -> u64 {
        self.price_output
    }
    /// What a sealed report says about the route: its id, kind, the model,
    /// the gateway's host and port, the residency (`null` when undeclared) and
    /// the prompt template's hash (`null` when the route names none). Never the
    /// URL's path, the template, a key or the name of the key's variable. A priced
    /// table rides along: the command bumps the revision from the first, so a
    /// priced route is exactly one whose revision is past 1, even at zero rates.
    pub(crate) fn seal(&self) -> Value {
        let mut sealed = json!({
            "id": self.id,
            "kind": self.kind.name(),
            "model": self.model,
            "host": self.host,
            "residency": self.residency,
            "prompt_hash": self.prompt_hash(),
        });
        if self.price_version != DEFAULT_PRICE_VERSION
            && let Some(object) = sealed.as_object_mut()
        {
            object.insert(String::from("price_version"), json!(self.price_version));
            object.insert(String::from("price_input_micros"), json!(self.price_input));
            object.insert(
                String::from("price_output_micros"),
                json!(self.price_output),
            );
        }
        sealed
    }
}
