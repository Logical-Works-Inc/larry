//! Routes (larry#201): where a tenant's model calls go, as data the tenant owns
//! instead of a gateway and a model compiled into the binary. A route names one
//! provider kind, a base URL, the NAME of the variable that holds its key, the
//! model id, an optional temperature, a request byte bound, the country where
//! the provider processes the data, and a short capability description. The
//! key itself is never part of a route.
//!
//! Validation is one function per field and one typed refusal per function, so
//! a hostile value is refused by name before it reaches the store or the wire.
//! The resolver is pure: the named route, else the tenant default, else the
//! operator's environment route, else `NoRoute`. Semantic choice (#202),
//! residency enforcement (#203), budgets (#204) and provider kinds (#205) plug
//! in at [`resolve`], [`Route::residency`], [`Route::request_bytes`] and [`Kind`].

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
    /// The temperature is not a number from 0 to 2.
    BadTemperature,
    /// The residency is not two uppercase ASCII letters (ISO 3166-1 alpha-2).
    BadResidency,
    /// The request byte bound is outside 1024 bytes to 16 MiB.
    BadRequestBytes,
    /// The description is over 1 KiB or holds a control byte other than a newline.
    BadDescribe,
    /// No route has this id.
    Unknown {
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
                out.write_str("route id env is reserved for the LARRY_MODEL_* environment route")
            }
            Self::BadUrl => out.write_str(
                "route url must be a bare http or https base URL: no credentials, query or fragment",
            ),
            Self::BadModel => out
                .write_str("route model must be 1 to 256 bytes with no whitespace or control byte"),
            Self::BadKeyEnv => out.write_str(
                "route key-env must name a variable: A-Z, 0-9 and _, not starting with a digit, at most 64 bytes",
            ),
            Self::BadTemperature => out.write_str("route temperature must be a number from 0 to 2"),
            Self::BadResidency => out.write_str(
                "route residency must be two uppercase letters, an ISO 3166-1 alpha-2 country code",
            ),
            Self::BadRequestBytes => {
                out.write_str("route request bytes must be from 1024 to 16777216")
            }
            Self::BadDescribe => out.write_str(
                "route description must be at most 1024 bytes with no control byte but a newline",
            ),
            Self::Unknown { ref id } => write!(out, "no route has the id {id}"),
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
            | Self::BadTemperature
            | Self::BadResidency
            | Self::BadRequestBytes
            | Self::BadDescribe
            | Self::Unknown { .. }
            | Self::NoRoute
            | Self::TableFull { .. } => None,
        }
    }
}

/// What speaks at the other end of a route. One kind exists today; #205 adds
/// the others, and a new kind is one variant and one wire adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Kind {
    /// An OpenAI-compatible chat-completions endpoint.
    OpenAiChat,
}

impl Kind {
    /// The kind's name in the store and in listings.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai-chat",
        }
    }

    /// The kind a stored name means, `None` for a name no kind has.
    pub(crate) fn parse(name: &str) -> Option<Kind> {
        match name {
            "openai-chat" => Some(Self::OpenAiChat),
            _ => None,
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

impl Route {
    /// A route a tenant stores. Every field is checked, and `env` is refused.
    ///
    /// # Errors
    ///
    /// The [`RouteError`] of the first field that fails its check.
    pub(crate) fn new(draft: &Draft<'_>) -> Result<Route, RouteError> {
        if draft.id == ENV_ROUTE {
            return Err(RouteError::ReservedId);
        }
        Route::build(draft)
    }

    /// The route the operator's environment declares: every check but the
    /// reserved id, which is its own.
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

    /// What a sealed report says about the route: its id, the model, the
    /// gateway's host and port, and the residency (`null` when undeclared).
    /// Never the URL's path, a key or the name of the key's variable.
    pub(crate) fn seal(&self) -> Value {
        json!({
            "id": self.id,
            "model": self.model,
            "host": self.host,
            "residency": self.residency,
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
/// default and sends a prompt somewhere the tenant did not choose.
///
/// # Errors
///
/// [`RouteError::Unknown`] for a name nothing has and [`RouteError::NoRoute`]
/// when no name, default or environment route exists.
pub(crate) fn resolve(
    table: &Table,
    named: Option<&str>,
    environment: Option<Route>,
) -> Result<Route, RouteError> {
    if let Some(route) = table.lookup(named) {
        return Ok(route.clone());
    }
    match (named, environment) {
        (Some(id), Some(route)) if id == ENV_ROUTE => Ok(route),
        (Some(id), _) => Err(RouteError::Unknown { id: id.to_owned() }),
        (None, Some(route)) => Ok(route),
        (None, None) => Err(RouteError::NoRoute),
    }
}

/// Unit tests of the field checks at their boundaries and of the resolver order.
#[cfg(test)]
mod tests {
    use super::{DEFAULT_REQUEST_BYTES, Draft, ENV_ROUTE, Kind, Route, RouteError, Table, resolve};

    /// A draft every check accepts.
    fn accepted() -> Draft<'static> {
        Draft {
            id: "alpha",
            kind: Kind::OpenAiChat,
            base_url: "https://gateway.example:8443/v1/",
            model: "model-1",
            key_env: "ALPHA_KEY",
            temperature: Some(0.5),
            request_bytes: Some(2048),
            residency: Some("CA"),
            describe: "cheap drafts\nsecond line",
        }
    }

    /// The outcome of building the accepted draft after `change`.
    fn built<'text>(change: impl FnOnce(&mut Draft<'text>)) -> Result<Route, RouteError> {
        let mut candidate: Draft<'text> = accepted();
        change(&mut candidate);
        Route::new(&candidate)
    }

    /// Each field is refused at its boundary with its own variant, and accepted one step inside.
    #[test]
    fn every_field_has_its_boundary_and_its_own_refusal() -> Result<(), RouteError> {
        let long_id = "a".repeat(65);
        let ok_id = "a".repeat(64);
        assert_eq!(
            built(|fields| fields.id = &long_id),
            Err(RouteError::BadId),
            "65 bytes"
        );
        built(|fields| fields.id = &ok_id)?;
        for id in ["Up", "", "a b", "a.b"] {
            assert_eq!(
                built(|fields| fields.id = id),
                Err(RouteError::BadId),
                "{id}"
            );
        }
        assert_eq!(
            built(|fields| fields.id = ENV_ROUTE),
            Err(RouteError::ReservedId),
            "env is reserved"
        );
        let far = format!("https://host/{}", "p".repeat(2048));
        for url in [
            "ftp://host/v1",
            "https://user:pass@host/v1",
            "https://host/v1?key=1",
            "https://host/v1#frag",
            "https://",
            "https://ho st/v1",
            "host/v1",
            far.as_str(),
        ] {
            assert_eq!(
                built(|fields| fields.base_url = url),
                Err(RouteError::BadUrl),
                "{url:.40}"
            );
        }
        let long_model = "m".repeat(257);
        for model in ["", "a b", long_model.as_str()] {
            assert_eq!(
                built(|fields| fields.model = model),
                Err(RouteError::BadModel),
                "{model:.20}"
            );
        }
        let long_key = "A".repeat(65);
        for name in ["", "lower", "1ABC", "A-B", long_key.as_str()] {
            assert_eq!(
                built(|fields| fields.key_env = name),
                Err(RouteError::BadKeyEnv),
                "{name:.20}"
            );
        }
        built(|fields| fields.key_env = "_")?;
        built(|fields| fields.key_env = &long_key[1..])?;
        for value in [f64::NAN, f64::INFINITY, -0.5, 2.5] {
            assert_eq!(
                built(|fields| fields.temperature = Some(value)),
                Err(RouteError::BadTemperature),
                "{value}"
            );
        }
        built(|fields| fields.temperature = Some(2.0))?;
        built(|fields| fields.temperature = Some(0.0))?;
        for value in [0, 1023, 16_777_217] {
            assert_eq!(
                built(|fields| fields.request_bytes = Some(value)),
                Err(RouteError::BadRequestBytes),
                "{value}"
            );
        }
        built(|fields| fields.request_bytes = Some(1024))?;
        built(|fields| fields.request_bytes = Some(16_777_216))?;
        for code in ["", "C", "CAN", "ca", "C1", "\u{c9}\u{c9}"] {
            assert_eq!(
                built(|fields| fields.residency = Some(code)),
                Err(RouteError::BadResidency),
                "{code}"
            );
        }
        let long_text = "d".repeat(1025);
        let top_text = "d".repeat(1024);
        assert_eq!(
            built(|fields| fields.describe = &long_text),
            Err(RouteError::BadDescribe),
            "1025"
        );
        built(|fields| fields.describe = &top_text)?;
        assert_eq!(
            built(|fields| fields.describe = "esc\u{1b}[31m"),
            Err(RouteError::BadDescribe),
            "escape"
        );
        Ok(())
    }

    /// A built route keeps its parts: the URL loses its trailing slash, the host
    /// keeps its port, the request bound defaults to 1 MiB, and the seal names no key.
    #[test]
    fn a_built_route_keeps_its_parts_and_seals_no_key() -> Result<(), RouteError> {
        let route = Route::new(&accepted())?;
        assert_eq!(
            route.base_url(),
            "https://gateway.example:8443/v1",
            "slash trimmed"
        );
        assert_eq!(route.host(), "gateway.example:8443", "host keeps its port");
        assert_eq!(route.kind().name(), "openai-chat", "kind name");
        let defaulted = built(|fields| fields.request_bytes = None)?;
        assert_eq!(
            defaulted.request_bytes(),
            DEFAULT_REQUEST_BYTES,
            "1 MiB default"
        );
        let sealed = route.seal().to_string();
        assert!(
            sealed.contains("gateway.example:8443"),
            "host sealed: {sealed}"
        );
        assert!(
            sealed.contains("\"residency\":\"CA\""),
            "residency sealed: {sealed}"
        );
        assert!(
            !sealed.contains("ALPHA_KEY"),
            "the key variable stays out: {sealed}"
        );
        assert!(!sealed.contains("/v1"), "the URL path stays out: {sealed}");
        let undeclared = built(|fields| fields.residency = None)?;
        assert!(
            undeclared.seal().to_string().contains("\"residency\":null"),
            "an undeclared residency seals as null"
        );
        assert_eq!(Kind::parse("openai-chat"), Some(Kind::OpenAiChat), "parse");
        assert_eq!(Kind::parse("other"), None, "an unknown kind");
        Ok(())
    }

    /// The resolver's order: named, else default, else environment, else none;
    /// a name nothing has never falls through.
    #[test]
    fn the_resolver_order_holds() -> Result<(), RouteError> {
        let alpha = Route::new(&accepted())?;
        let beta = built(|fields| fields.id = "beta")?;
        let environment = Route::environment(&Draft {
            id: ENV_ROUTE,
            ..accepted()
        })?;
        let table = Table::new(
            vec![alpha.clone(), beta.clone()],
            Some(String::from("beta")),
        );
        assert_eq!(
            resolve(&table, Some("alpha"), None)?,
            alpha,
            "named beats default"
        );
        assert_eq!(
            resolve(&table, None, Some(environment.clone()))?,
            beta,
            "default beats env"
        );
        assert_eq!(
            resolve(&table, Some("env"), Some(environment.clone()))?,
            environment,
            "env can be named"
        );
        assert_eq!(
            resolve(&table, Some("gamma"), Some(environment.clone())),
            Err(RouteError::Unknown {
                id: String::from("gamma")
            }),
            "an unknown name never falls through"
        );
        assert_eq!(
            resolve(&table, Some("env"), None),
            Err(RouteError::Unknown {
                id: String::from("env")
            }),
            "env is unknown when undeclared"
        );
        let bare = Table::new(Vec::new(), None);
        assert_eq!(
            resolve(&bare, None, Some(environment.clone()))?,
            environment,
            "env alone"
        );
        assert_eq!(
            resolve(&bare, None, None),
            Err(RouteError::NoRoute),
            "nothing"
        );
        Ok(())
    }
}
