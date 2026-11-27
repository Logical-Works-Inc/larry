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

use super::route_transport::{authority_of, is_loopback};

#[path = "route_table.rs"]
mod table;
pub(crate) use table::Table;
#[path = "route_view.rs"]
mod view;

/// The id of the route the operator's environment declares; no stored route may
/// take it, so a task naming `env` can only mean the environment.
pub(crate) const ENV_ROUTE: &str = "env";
/// Most routes one tenant holds; the next one is refused.
pub(crate) const MAX_ROUTES: u16 = 256;
/// Longest route id, in bytes.
const MAX_ID_BYTES: usize = 64;
/// Longest base URL, in bytes.
pub(super) const MAX_URL_BYTES: usize = 2048;
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
/// The first route price-table revision.
pub(crate) const DEFAULT_PRICE_VERSION: u32 = 1;
/// Highest price a route table may carry, in micro-dollars per million tokens:
/// the same bound the eval price grammar enforces, so a stored rate and a
/// `route price` rate answer to one ceiling.
pub(crate) const MAX_PRICE_MICRO: u64 = 10_000_000_000;

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
    /// The route uses plaintext for a host that is not loopback.
    InsecureTransport {
        /// The host that would receive the request.
        host: String,
    },
    /// The route is outside the tenant's stored residency policy.
    ResidencyNotAllowed {
        /// The route id.
        route: String,
        /// The provider's declared residency, or none.
        residency: Option<String>,
        /// The tenant's allowed country codes.
        allowed: Vec<String>,
    },
    /// The tenant policy exceeds its country-code bound.
    ResidencyPolicyTooLarge {
        /// The maximum number of codes.
        limit: usize,
    },
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
    /// A price is not micro-dollars per million tokens within the bound: a
    /// negative or over-large stored rate, or a `route price` rate the price
    /// grammar refuses.
    BadPrice {
        /// The rate that failed: `input` or `output`.
        field: &'static str,
    },
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

impl RouteError {
    /// Whether this refusal already opens with `route <field>`, so a door
    /// adds no second prefix; every other refusal gets `route: ` by default,
    /// so none reaches the user bare (larry#228, larry#295). Every variant is
    /// listed with no wildcard, so a new refusal fails the build until it is
    /// classified here.
    pub(crate) const fn opens_with_route(&self) -> bool {
        match *self {
            Self::BadId
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
            | Self::ResidencyNotAllowed { .. }
            | Self::ResidencyPolicyTooLarge { .. }
            | Self::BadRequestBytes
            | Self::BadDescribe
            | Self::BadPrice { .. } => true,
            Self::InsecureTransport { .. }
            | Self::NoSuchId { .. }
            | Self::NoRoute
            | Self::TableFull { .. }
            | Self::Damaged { .. } => false,
        }
    }
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
            Self::InsecureTransport { ref host } => {
                write!(out, "route host {host} requires https unless it is loopback")
            }
            Self::ResidencyNotAllowed {
                ref route,
                ref residency,
                ref allowed,
            } => write!(
                out,
                "route {route} residency {residency:?} is outside the tenant residency allowlist {allowed:?}"
            ),
            Self::ResidencyPolicyTooLarge { limit } => {
                write!(out, "route residency allows at most {limit} country codes")
            }
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
            Self::BadPrice { field } => write!(
                out,
                "route {field} price must be dollars per million tokens like 0.30, at most 10000"
            ),
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
        if let Self::Damaged {
            cause: Some(ref inner),
            ..
        } = *self
        {
            Some(inner.as_ref())
        } else {
            None
        }
    }
}

/// What speaks at the other end of a route. One variant per provider kind
/// (larry#205, larry#539); the client's wire adapter for a new kind plugs in
/// at the call site. Every named kind is served (larry#546).
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
    /// The versioned price table revision for this route.
    price_version: u32,
    /// Prompt micro-dollars per million tokens.
    price_input: u64,
    /// Completion micro-dollars per million tokens.
    price_output: u64,
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
        if let Some(refusal) = route.transport_refusal() {
            return Err(refusal);
        }
        let in_namespace = route
            .key_env
            .strip_prefix(KEY_ENV_PREFIX)
            .is_some_and(|rest| !rest.is_empty());
        if !in_namespace {
            return Err(RouteError::KeyEnvOutsideNamespace);
        }
        Ok(route)
    }

    /// A legacy stored route may be listed or removed even when it predates the
    /// HTTPS rule. Resolution calls this check before returning it to a sender.
    pub(crate) fn stored(draft: &Draft<'_>) -> Result<Route, RouteError> {
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
        let route = Route::build(draft)?;
        match route.transport_refusal() {
            Some(refusal) => Err(refusal),
            None => Ok(route),
        }
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
            price_version: DEFAULT_PRICE_VERSION,
            price_input: 0,
            price_output: 0,
        })
    }

    /// The route with its price table set to `version`, `input` and `output`
    /// micro-dollars per million tokens. Rates past the bound are refused, so
    /// a damaged row or a hostile flag never prices a call.
    ///
    /// # Errors
    ///
    /// [`RouteError::BadPrice`] naming the first rate past the bound.
    pub(crate) fn with_price(
        mut self,
        version: u32,
        input: u64,
        output: u64,
    ) -> Result<Route, RouteError> {
        if input > MAX_PRICE_MICRO {
            return Err(RouteError::BadPrice { field: "input" });
        }
        if output > MAX_PRICE_MICRO {
            return Err(RouteError::BadPrice { field: "output" });
        }
        self.price_version = version;
        self.price_input = input;
        self.price_output = output;
        Ok(self)
    }

    /// The transport refusal, when a route would send plaintext off-host.
    pub(crate) fn transport_refusal(&self) -> Option<RouteError> {
        (self.base_url.starts_with("http://") && !is_loopback(&self.host)).then(|| {
            RouteError::InsecureTransport {
                host: self.host.clone(),
            }
        })
    }
    /// The residency refusal for this route under `allowed`, when a policy exists.
    pub(crate) fn residency_refusal(&self, allowed: &[String]) -> Option<RouteError> {
        (!allowed.is_empty()
            && !self
                .residency()
                .is_some_and(|value| allowed.iter().any(|allowed| allowed == value)))
        .then(|| RouteError::ResidencyNotAllowed {
            route: self.id.clone(),
            residency: self.residency.clone(),
            allowed: allowed.to_vec(),
        })
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
    let Some(route) = table.lookup(named) else {
        return match (named, environment) {
            (Some(id), Some(route)) if id == ENV_ROUTE => Ok(route),
            (Some(id), _) => Err(RouteError::NoSuchId { id: id.to_owned() }),
            (None, Some(route)) => Ok(route),
            (None, None) => Err(RouteError::NoRoute),
        };
    };
    if let Some(refusal) = route.transport_refusal() {
        return Err(refusal);
    }
    match route.key_env_refusal(tenant) {
        None => Ok(route.clone()),
        Some(refusal) => Err(refusal),
    }
}
/// Unit tests of the field checks at their boundaries and of the resolver order.
/// They live in `route_tests.rs`: verification beside the mechanism, one file
/// per job, as the seeded simulations live in `sim_route.rs`.
#[cfg(test)]
#[path = "route_tests.rs"]
mod tests;

/// A built route's parts and the resolver's order.
#[cfg(test)]
#[path = "sim_route.rs"]
mod sim_route;
