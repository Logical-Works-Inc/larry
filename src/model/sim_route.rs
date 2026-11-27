//! A built route's parts and the resolver's order (larry#201).

use super::tests::{TENANT, accepted, built};
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
        !sealed.contains("LARRY_KEY_0123456789ABCDEF_ALPHA"),
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

/// A non-loopback plaintext route is refused before the client can send bytes.
#[test]
fn sim_plaintext_non_loopback_is_refused_before_send() {
    for seed in 0..64_u64 {
        let url = format!("http://gateway-{seed}.example:8443/v1");
        let mut draft = accepted();
        draft.base_url = url.as_str();
        let host = format!("gateway-{seed}.example:8443");
        assert_eq!(
            Route::new(&draft),
            Err(RouteError::InsecureTransport { host }),
            "plaintext must not reach a remote host"
        );
    }
}

/// Loopback is the only host class where plaintext is permitted for a local mock.
#[test]
fn sim_plaintext_loopback_is_allowed() -> Result<(), RouteError> {
    for seed in 0..64_u64 {
        let url = format!("http://127.0.0.1:{}/v1", 8000 + seed);
        let mut draft = accepted();
        draft.base_url = url.as_str();
        Route::new(&draft).map(|_| ())?;
    }
    Ok(())
}

#[test]
fn sim_residency_allows_a_matching_country_only() -> Result<(), RouteError> {
    let route = Route::new(&accepted())?;
    let allowed = vec![String::from("CA")];
    assert!(route.residency_refusal(&allowed).is_none());
    let refused = route.residency_refusal(&[String::from("DE")]);
    assert!(matches!(
        refused,
        Some(RouteError::ResidencyNotAllowed { .. })
    ));
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
        resolve(TENANT, &table, Some("alpha"), None)?,
        alpha,
        "named beats default"
    );
    assert_eq!(
        resolve(TENANT, &table, None, Some(environment.clone()))?,
        beta,
        "default beats env"
    );
    assert_eq!(
        resolve(TENANT, &table, Some("env"), Some(environment.clone()))?,
        environment,
        "env can be named"
    );
    assert_eq!(
        resolve(TENANT, &table, Some("gamma"), Some(environment.clone())),
        Err(RouteError::NoSuchId {
            id: String::from("gamma")
        }),
        "an unknown name never falls through"
    );
    assert_eq!(
        resolve(TENANT, &table, Some("env"), None),
        Err(RouteError::NoSuchId {
            id: String::from("env")
        }),
        "env is unknown when undeclared"
    );
    let bare = Table::new(Vec::new(), None);
    assert_eq!(
        resolve(TENANT, &bare, None, Some(environment.clone()))?,
        environment,
        "env alone"
    );
    assert_eq!(
        resolve(TENANT, &bare, None, None),
        Err(RouteError::NoRoute),
        "nothing"
    );
    Ok(())
}
