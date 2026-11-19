//! A built route's parts and the resolver's order (larry#201).

use super::tests::{accepted, built};
use super::{DEFAULT_REQUEST_BYTES, Draft, ENV_ROUTE, Kind, Route, RouteError, Table, resolve};

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
        !sealed.contains("LARRY_KEY_ALPHA"),
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
        Err(RouteError::NoSuchId {
            id: String::from("gamma")
        }),
        "an unknown name never falls through"
    );
    assert_eq!(
        resolve(&table, Some("env"), None),
        Err(RouteError::NoSuchId {
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
