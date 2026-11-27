//! Route accessors and the sealed route view.

use lgwks_std::json::{Value, json};

use super::{Kind, Route, RouteError, tenant_namespace};

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
    /// What the route is good for.
    pub(crate) fn describe(&self) -> &str {
        &self.describe
    }
    /// What a sealed report says about the route, without a key or key variable.
    pub(crate) fn seal(&self) -> Value {
        json!({
            "id": self.id,
            "model": self.model,
            "host": self.host,
            "residency": self.residency,
        })
    }
}
