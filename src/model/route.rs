//! Routes (larry#201): where a tenant's model calls go, as data the tenant owns
//! instead of a gateway compiled into the binary. A route names one provider
//! kind, a base URL, the NAME of the variable that holds its key, the model id,
//! an optional temperature, a request byte bound, the country where the
//! provider processes the data, and a short capability description. The key
//! itself is never part of a route, and only that tenant may name its variable
//! (`LARRY_KEY_<TAG>_*`, larry#328).
//!
//! Every field is checked by one function with one typed refusal. The resolver
//! is pure: the named route, else the tenant default, else the operator's
//! environment route, else `NoRoute`. Semantic choice (#202), residency
//! enforcement (#203), budgets (#204) and provider kinds (#205) plug in at
//! [`resolve`], [`Route::residency`], [`Route::request_bytes`] and [`Kind`].

use std::fmt;

use lgwks_std::json::{Value, json};

/// The id of the route the operator's environment declares; no stored route may
/// take it, so a task naming `env` can only mean the environment.
pub(crate) const ENV_ROUTE: &str = "env";
/// Most routes one tenant holds; the next one is refused.
pub(crate) const MAX_ROUTES: u16 = 256;
/// Longest route id, in bytes.
const MAX_ID_BYTES: usize = 64;
/// Longest base URL, in bytes.
const MAX_URL_BYTES: usize = 2048;
/// Longest model id, in bytes.
const MAX_MODEL_BYTES: usize = 256;
/// Longest key variable name, in bytes.
const MAX_KEY_ENV_BYTES: usize = 64;
/// The namespace a stored route's key variable lives in. A route is tenant
/// data, so it may only name a variable the operator set aside for routes; the
/// process's other secrets (`GH_TOKEN`, cloud keys, `LARRY_MODEL_KEY`) are out.
const KEY_ENV_PREFIX: &str = "LARRY_KEY_";
/// Characters of the tenant text that make the tenant's own tag: 64 bits of the
/// tenant digest, so two tenants on one host share a namespace with probability
/// below 2^-33 at the 65,536-tenant bound (larry#328).
const TENANT_TAG_CHARS: usize = 16;
/// Longest capability description, in bytes.
const MAX_DESCRIBE_BYTES: usize = 1024;
/// The request body bound a route gets when it names none: 1 MiB.
pub(crate) const DEFAULT_REQUEST_BYTES: usize = 1_048_576;
/// Smallest request body bound a route may name.
const MIN_REQUEST_BYTES: usize = 1024;
/// Largest request body bound a route may name: 16 MiB.
const MAX_REQUEST_BYTES: usize = 16_777_216;
/// Highest sampling temperature a route may name.
const MAX_TEMPERATURE: f64 = 2.0;
/// Longest prompt template, in bytes.
const MAX_PROMPT_BYTES: usize = 8192;

/// Why a route, a route name or a resolution was refused. Every variant is one
/// failure; none carries a key, a URL or a variable's value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RouteError {
    /// The id is not 1 to 64 bytes of `a-z`, `0-9`, `_` and `-`.
    BadId,
    /// The id is `env`, which only the operator's environment may be.
    ReservedId,
    /// The base URL is not a bare http or https URL: no credentials, query or
    /// fragment, a host made of letters, digits, dots, hyphens and a port.
    BadUrl,
    /// The model id is empty, over 256 bytes, or holds whitespace or a control byte.
    BadModel,
    /// The key variable name is not `[A-Z_][A-Z0-9_]{0,63}`.
    BadKeyEnv,
    /// The key variable is not `LARRY_KEY_` followed by at least one more byte, so
    /// it could name a secret the operator never set aside for routes.
    KeyEnvOutsideNamespace,
    /// The key variable is `LARRY_KEY_*` but not in this tenant's namespace, so
    /// its value could be another tenant's key sent to this route's host.
    KeyEnvNotThisTenants {
        /// The variable the route names.
        variable: String,
        /// The prefix this tenant's routes may name.
        prefix: String,
    },
    /// The temperature is not a number from 0 to 2.
    BadTemperature,
    /// The kind is not one the client speaks.
    BadKind,
    /// The prompt template is over 8 KiB or holds a control byte but a newline.
    BadPrompt,
    /// The residency is not two uppercase ASCII letters (ISO 3166-1 alpha-2).
    BadResidency,
    /// The request byte bound is outside 1024 bytes to 16 MiB.
    BadRequestBytes,
    /// The description is over 1 KiB or holds a control byte other than a newline.
    BadDescribe,
    /// No route has this id.
    NoSuchId {
        /// The id that was asked for.
        id: String,
    },
    /// The task names no route, the tenant has no default and the environment
    /// declares none: there is nowhere to send a model call.
    NoRoute,
    /// The tenant already holds the most routes it may.
    TableFull {
        /// The bound.
        limit: usize,
    },
    /// A stored route fails validation: the store was edited or damaged.
    Damaged {
        /// The id of the stored row.
        id: String,
        /// The field check the row fails, when it is one of the route checks.
        cause: Option<Box<RouteError>>,
    },
}

impl fmt::Display for RouteError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::BadId => out.write_str("route id must be 1 to 64 bytes of a-z, 0-9, _ and -"),
            Self::ReservedId => {
                // One `route` word, as every field refusal carries (larry#228, larry#295).
                out.write_str("route id env is reserved for the LARRY_MODEL_* environment variables")
            }
            Self::BadUrl => out.write_str(
                "route url must be a bare http or https base URL: no credentials, query or fragment",
            ),
            Self::BadModel => out
                .write_str("route model must be 1 to 256 bytes with no whitespace or control byte"),
            Self::BadKeyEnv => out.write_str(
                "route key-env must name a variable: A-Z, 0-9 and _, not starting with a digit, at most 64 bytes",
            ),
            Self::KeyEnvOutsideNamespace => write!(
                out,
                "route key-env must start with {KEY_ENV_PREFIX} and name at least one more byte"
            ),
            Self::KeyEnvNotThisTenants {
                ref variable,
                ref prefix,
            } => write!(
                out,
                "route key-env {variable} is not this tenant's: it must start with {prefix}"
            ),
            Self::BadTemperature => out.write_str("route temperature must be a number from 0 to 2"),
            Self::BadKind => out.write_str(
                "route kind must be one of openai-chat, anthropic-messages, openai-responses",
            ),
            Self::BadPrompt => out.write_str(
                "route prompt must be at most 8192 bytes with no control byte but a newline",
            ),
            Self::BadResidency => out.write_str(
                "route residency must be two uppercase letters, an ISO 3166-1 alpha-2 country code",
            ),
            Self::BadRequestBytes => {
                out.write_str("route request bytes must be from 1024 to 16777216")
            }
            Self::BadDescribe => out.write_str(
                "route description must be at most 1024 bytes with no control byte but a newline",
            ),
            Self::NoSuchId { ref id } => write!(out, "no route has the id {id}"),
            Self::NoRoute => out.write_str(
                "no model route: add one with `larry route add`, or set LARRY_MODEL_URL, LARRY_MODEL and LARRY_MODEL_KEY",
            ),
            Self::TableFull { limit } => write!(
                out,
                "the tenant already holds the most routes it may ({limit})"
            ),
            Self::Damaged { ref id, .. } => write!(out, "the stored route {id} fails validation"),
        }
    }
}

impl std::error::Error for RouteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Damaged {
                cause: Some(ref inner),
                ..
            } => Some(inner.as_ref()),
            Self::Damaged { cause: None, .. }
            | Self::BadId
            | Self::ReservedId
            | Self::BadUrl
            | Self::BadModel
            | Self::BadKeyEnv
            | Self::KeyEnvOutsideNamespace
            | Self::KeyEnvNotThisTenants { .. }
            | Self::BadTemperature
            | Self::BadKind
            | Self::BadPrompt
            | Self::BadResidency
            | Self::BadRequestBytes
            | Self::BadDescribe
            | Self::NoSuchId { .. }
            | Self::NoRoute
            | Self::TableFull { .. } => None,
        }
    }
}

/// What speaks at the other end of a route. One variant per provider kind
/// (larry#205); a new kind is one variant and one wire adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Kind {
    /// An OpenAI-compatible chat-completions endpoint.
    OpenAiChat,
    /// The Anthropic Messages API (`x-api-key`, `anthropic-version`).
    AnthropicMessages,
    /// The OpenAI Responses API.
    OpenAiResponses,
}

impl Kind {
    /// The kind's name in the store and in listings.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai-chat",
            Self::AnthropicMessages => "anthropic-messages",
            Self::OpenAiResponses => "openai-responses",
        }
    }

    /// The kind a stored name means, `None` for a name no kind has.
    pub(crate) fn parse(name: &str) -> Option<Kind> {
        match name {
            "openai-chat" => Some(Self::OpenAiChat),
            "anthropic-messages" => Some(Self::AnthropicMessages),
            "openai-responses" => Some(Self::OpenAiResponses),
            _ => None,
        }
    }

    /// Whether the kind's file replies are structured JSON (`{"content": ...}`)
    /// rather than bare text: the Responses and Messages APIs answer in their
    /// own envelopes, so the brief asks for the file inside one object.
    pub(crate) const fn structured(self) -> bool {
        match self {
            Self::OpenAiChat => false,
            Self::AnthropicMessages | Self::OpenAiResponses => true,
        }
    }

    /// The request path the kind speaks, appended to the route's base URL.
    pub(crate) const fn path(self) -> &'static str {
        match self {
            Self::OpenAiChat => "/chat/completions",
            Self::AnthropicMessages => "/messages",
            Self::OpenAiResponses => "/responses",
        }
    }
}

/// The fields of a route before they are checked.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Draft<'draft> {
    /// The route id.
    pub(crate) id: &'draft str,
    /// The provider kind.
    pub(crate) kind: Kind,
    /// The base URL.
    pub(crate) base_url: &'draft str,
    /// The model id.
    pub(crate) model: &'draft str,
    /// The name of the variable holding the key.
    pub(crate) key_env: &'draft str,
    /// The sampling temperature, when the route pins one.
    pub(crate) temperature: Option<f64>,
    /// The request body bound, when the route names one.
    pub(crate) request_bytes: Option<usize>,
    /// The country where the provider processes the data, when declared.
    pub(crate) residency: Option<&'draft str>,
    /// The prompt template prepended to the system turn, when the route names one.
    pub(crate) prompt: Option<&'draft str>,
    /// What the route is good for, in the tenant's words.
    pub(crate) describe: &'draft str,
}

/// One checked route.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Route {
    /// The route id.
    id: String,
    /// The provider kind.
    kind: Kind,
    /// The base URL, without a trailing slash.
    base_url: String,
    /// The URL's host and port: all a sealed report says about the gateway.
    host: String,
    /// The name of the variable holding the key; never the key.
    key_env: String,
    /// The model id sent on every request.
    model: String,
    /// The sampling temperature, when pinned.
    temperature: Option<f64>,
    /// The request body bound, in bytes.
    request_bytes: usize,
    /// The country where the provider processes the data, when declared.
    residency: Option<String>,
    /// The prompt template prepended to the system turn, when the route names one.
    prompt: Option<String>,
    /// What the route is good for.
    describe: String,
}

/// The host and port of a bare http or https base URL, `None` when `url` is not one.
fn authority_of(url: &str) -> Option<&str> {
    if url.len() > MAX_URL_BYTES
        || url.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '?' | '#' | '\\' | '@')
        })
    {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split('/').next()?;
    let well_formed = !authority.is_empty()
        && authority.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':' | b'[' | b']')
        });
    well_formed.then_some(authority)
}

/// Whether `text` is a route id: 1 to 64 bytes of `a-z`, `0-9`, `_` and `-`.
pub(crate) fn is_route_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_ID_BYTES
        && text
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

/// The namespace one tenant's route key variables live in: `LARRY_KEY_<TAG>_`,
/// where `<TAG>` is the first 16 characters of the tenant text in upper case.
/// The tag is a namespace, never a secret: `larry route add` prints it.
pub(crate) fn tenant_namespace(tenant: &str) -> String {
    let tag: String = tenant.chars().take(TENANT_TAG_CHARS).collect();
    format!("{KEY_ENV_PREFIX}{}_", tag.to_ascii_uppercase())
}

/// Whether `text` names an environment variable: `[A-Z_][A-Z0-9_]{0,63}`.
fn is_key_env(text: &str) -> bool {
    let mut bytes = text.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    text.len() <= MAX_KEY_ENV_BYTES
        && (first.is_ascii_uppercase() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Whether `text` is a model id: 1 to 256 bytes of printable ASCII, no space.
fn is_model_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_MODEL_BYTES
        && text.bytes().all(|byte| byte.is_ascii_graphic())
}

/// Whether `text` is an ISO 3166-1 alpha-2 shaped code: two uppercase ASCII letters.
fn is_residency(text: &str) -> bool {
    text.len() == 2 && text.bytes().all(|byte| byte.is_ascii_uppercase())
}

/// Whether `text` is a description: at most 1 KiB, no control byte but a newline.
fn is_describe(text: &str) -> bool {
    text.len() <= MAX_DESCRIBE_BYTES
        && text
            .chars()
            .all(|character| character == '\n' || !character.is_control())
}

/// Whether `text` is a prompt template: at most 8 KiB, no control byte but a newline.
fn is_prompt(text: &str) -> bool {
    text.len() <= MAX_PROMPT_BYTES
        && text
            .chars()
            .all(|character| character == '\n' || !character.is_control())
}

impl Route {
    /// A route a tenant stores. Every field is checked, `env` is refused, and
    /// the key variable must be `LARRY_KEY_` and at least one more byte, so a
    /// stored route never names a secret the operator did not set aside for it.
    /// Which tenant may name it is checked at `larry route add` and again in
    /// [`resolve`], never as a field check (larry#328).
    ///
    /// # Errors
    ///
    /// The [`RouteError`] of the first field that fails its check; the key
    /// namespace is checked after the fields.
    pub(crate) fn new(draft: &Draft<'_>) -> Result<Route, RouteError> {
        if draft.id == ENV_ROUTE {
            return Err(RouteError::ReservedId);
        }
        let route = Route::build(draft)?;
        let in_namespace = route
            .key_env
            .strip_prefix(KEY_ENV_PREFIX)
            .is_some_and(|rest| !rest.is_empty());
        if in_namespace {
            Ok(route)
        } else {
            Err(RouteError::KeyEnvOutsideNamespace)
        }
    }

    /// The route the operator's environment declares: every check but the
    /// reserved id, which is its own, and the key namespace, since the operator
    /// names `LARRY_MODEL_KEY` themself.
    ///
    /// # Errors
    ///
    /// The [`RouteError`] of the first field that fails its check.
    pub(crate) fn environment(draft: &Draft<'_>) -> Result<Route, RouteError> {
        Route::build(draft)
    }

    /// Checks every field of `draft` and keeps the result.
    fn build(draft: &Draft<'_>) -> Result<Route, RouteError> {
        if !is_route_id(draft.id) {
            return Err(RouteError::BadId);
        }
        let host = authority_of(draft.base_url).ok_or(RouteError::BadUrl)?;
        if !is_model_id(draft.model) {
            return Err(RouteError::BadModel);
        }
        if !is_key_env(draft.key_env) {
            return Err(RouteError::BadKeyEnv);
        }
        if let Some(temperature) = draft.temperature
            && !(0.0..=MAX_TEMPERATURE).contains(&temperature)
        {
            return Err(RouteError::BadTemperature);
        }
        let request_bytes = match draft.request_bytes {
            Some(bytes) => bytes,
            None => DEFAULT_REQUEST_BYTES,
        };
        if !(MIN_REQUEST_BYTES..=MAX_REQUEST_BYTES).contains(&request_bytes) {
            return Err(RouteError::BadRequestBytes);
        }
        if let Some(residency) = draft.residency
            && !is_residency(residency)
        {
            return Err(RouteError::BadResidency);
        }
        if !is_describe(draft.describe) {
            return Err(RouteError::BadDescribe);
        }
        if let Some(prompt) = draft.prompt
            && !is_prompt(prompt)
        {
            return Err(RouteError::BadPrompt);
        }
        Ok(Route {
            id: draft.id.to_owned(),
            kind: draft.kind,
            base_url: draft.base_url.trim_end_matches('/').to_owned(),
            host: host.to_owned(),
            key_env: draft.key_env.to_owned(),
            model: draft.model.to_owned(),
            temperature: draft.temperature,
            request_bytes,
            residency: draft.residency.map(str::to_owned),
            prompt: draft.prompt.map(str::to_owned),
            describe: draft.describe.to_owned(),
        })
    }

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

    /// The binding refusal of this route's key variable under `tenant`, `None`
    /// when the variable is one only that tenant may name: `LARRY_KEY_<TAG>_`
    /// for this tenant's tag, then at least one byte. A row stored before the
    /// binding, or edited in the store, fails here, so its value is never read
    /// (larry#328, CWE-200).
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
    /// route names none: what a sealed record carries instead of the text.
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

    /// What a sealed report says about the route: its id, kind, the model, the
    /// gateway's host and port, the residency (`null` when undeclared) and the
    /// prompt template's hash (`null` when the route names none). Never the
    /// URL's path, the template, a key or the name of the key's variable.
    pub(crate) fn seal(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind.name(),
            "model": self.model,
            "host": self.host,
            "residency": self.residency,
            "prompt_hash": self.prompt_hash(),
        })
    }
}

/// A tenant's stored routes and which one is the default.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Table {
    /// The routes, ordered by id.
    routes: Vec<Route>,
    /// The id of the default route, when there is one.
    default: Option<String>,
}

impl Table {
    /// A table of `routes` whose default is `default`.
    pub(crate) fn new(routes: Vec<Route>, default: Option<String>) -> Table {
        Table { routes, default }
    }

    /// The routes, ordered by id.
    pub(crate) fn routes(&self) -> &[Route] {
        &self.routes
    }

    /// The id of the default route.
    pub(crate) fn default_id(&self) -> Option<&str> {
        self.default.as_deref()
    }

    /// The stored route `named` asks for, or the default when it names none.
    pub(crate) fn lookup(&self, named: Option<&str>) -> Option<&Route> {
        let wanted = named.or(self.default.as_deref())?;
        self.routes.iter().find(|route| route.id == wanted)
    }
}

/// The route a task uses: the one it names, else the tenant default, else the
/// operator's environment route, else a refusal. A name no stored route has is
/// refused before anything else is tried, so a typo never falls through to a
/// default and sends a prompt somewhere the tenant did not choose. A stored
/// route whose key variable is not `tenant`'s own is refused before its value
/// is ever read, and never falls through to the environment route either: the
/// tenant asked for a route it may not use (larry#328).
///
/// # Errors
///
/// [`RouteError::KeyEnvNotThisTenants`] for a stored route outside `tenant`'s
/// namespace, [`RouteError::NoSuchId`] for a name nothing has and
/// [`RouteError::NoRoute`] when no name, default or environment route exists.
pub(crate) fn resolve(
    tenant: &str,
    table: &Table,
    named: Option<&str>,
    environment: Option<Route>,
) -> Result<Route, RouteError> {
    if let Some(route) = table.lookup(named) {
        return match route.key_env_refusal(tenant) {
            None => Ok(route.clone()),
            Some(refusal) => Err(refusal),
        };
    }
    match (named, environment) {
        (Some(id), Some(route)) if id == ENV_ROUTE => Ok(route),
        (Some(id), _) => Err(RouteError::NoSuchId { id: id.to_owned() }),
        (None, Some(route)) => Ok(route),
        (None, None) => Err(RouteError::NoRoute),
    }
}
/// Unit tests of the field checks at their boundaries and of the resolver order.
#[cfg(test)]
#[path = "route_tests.rs"]
mod tests;

/// A built route's parts and the resolver's order.
#[cfg(test)]
#[path = "sim_route.rs"]
mod sim_route;
