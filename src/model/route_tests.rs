//! Unit tests of the route field checks at their boundaries and of the resolver
//! order. Split from `route.rs`, which keeps the mechanism: verification beside
//! the mechanism, one file per job, as the seeded simulations live in
//! `sim_route.rs`.

use super::{Draft, ENV_ROUTE, Kind, MAX_PRICE_MICRO, Route, RouteError, tenant_namespace};

/// The tenant text the unit tests build routes under: a 64-hex digest, as
/// `executor::routes` keys its rows by, whose tag is its first 16 characters.
pub(super) const TENANT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A draft every check accepts, its key variable under [`TENANT`]'s tag.
pub(super) fn accepted() -> Draft<'static> {
    Draft {
        id: "alpha",
        kind: Kind::OpenAiChat,
        base_url: "https://gateway.example:8443/v1/",
        model: "model-1",
        key_env: "LARRY_KEY_0123456789ABCDEF_ALPHA",
        temperature: Some(0.5),
        request_bytes: Some(2048),
        residency: Some("CA"),
        prompt: None,
        describe: "cheap drafts\nsecond line",
    }
}

/// The outcome of building the accepted draft after `change`.
pub(super) fn built<'text>(change: impl FnOnce(&mut Draft<'text>)) -> Result<Route, RouteError> {
    let mut candidate: Draft<'text> = accepted();
    change(&mut candidate);
    Route::new(&candidate)
}

/// The id, URL and model are each refused at their boundary with their own variant, and
/// accepted one step inside.
#[test]
fn every_identity_field_has_its_boundary_and_its_own_refusal() -> Result<(), RouteError> {
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
    Ok(())
}

/// The key variable, temperature, request bound, residency and description are each
/// refused at their boundary with their own variant, and accepted one step inside.
#[test]
fn every_other_field_has_its_boundary_and_its_own_refusal() -> Result<(), RouteError> {
    let long_key = format!("LARRY_KEY_{}", "A".repeat(55));
    let top_key = format!("LARRY_KEY_{}", "A".repeat(54));
    for name in ["", "lower", "1ABC", "LARRY_KEY_A-B", long_key.as_str()] {
        assert_eq!(
            built(|fields| fields.key_env = name),
            Err(RouteError::BadKeyEnv),
            "{name:.20}"
        );
    }
    built(|fields| fields.key_env = &top_key)?;
    built(|fields| fields.key_env = "LARRY_KEY_A")?;
    for name in [
        "LARRY_MODEL_KEY",
        "GH_TOKEN",
        "AWS_SECRET_ACCESS_KEY",
        "_",
        "LARRY_KEY_",
        "LARRY_KEY",
        "XLARRY_KEY_A",
        "LARRY_KEYS_A",
    ] {
        assert_eq!(
            built(|fields| fields.key_env = name),
            Err(RouteError::KeyEnvOutsideNamespace),
            "{name}"
        );
    }
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
    Route::environment(&Draft {
        id: ENV_ROUTE,
        key_env: "LARRY_MODEL_KEY",
        ..accepted()
    })?;
    Ok(())
}

/// The price table: ten thousand dollars a million tokens is kept, one
/// micro-dollar past it is refused naming the rate, and the revision rides
/// along untouched by the field checks (larry#204, part of #533).
#[test]
fn a_price_past_the_bound_is_refused() -> Result<(), RouteError> {
    let priced = Route::new(&accepted())?.with_price(2, MAX_PRICE_MICRO, 0)?;
    assert_eq!(priced.price_version(), 2, "the revision rides along");
    assert_eq!(
        priced.price_input(),
        MAX_PRICE_MICRO,
        "the bound itself is kept"
    );
    let both = Route::new(&accepted())?.with_price(2, MAX_PRICE_MICRO, MAX_PRICE_MICRO)?;
    assert_eq!(
        both.price_output(),
        MAX_PRICE_MICRO,
        "the bound itself is kept on the output too"
    );
    assert_eq!(
        Route::new(&accepted())?.with_price(2, MAX_PRICE_MICRO + 1, 0),
        Err(RouteError::BadPrice { field: "input" }),
        "one micro-dollar past ten thousand in"
    );
    assert_eq!(
        Route::new(&accepted())?.with_price(2, 0, MAX_PRICE_MICRO + 1),
        Err(RouteError::BadPrice { field: "output" }),
        "one micro-dollar past ten thousand out"
    );
    Ok(())
}

/// The tenant binding: the tag is the tenant text's first 16 characters in upper case, and another tenant's tag is refused by name (larry#328).
#[test]
fn a_key_variable_is_bound_to_its_tenants_tag() -> Result<(), RouteError> {
    let prefix = tenant_namespace(TENANT);
    assert_eq!(prefix, "LARRY_KEY_0123456789ABCDEF_");
    let own = Route::new(&accepted())?;
    assert_eq!(own.key_env_refusal(TENANT), None, "its own tag");
    let longer = built(|fields| fields.key_env = "LARRY_KEY_0123456789ABCDEF_X")?;
    assert_eq!(longer.key_env_refusal(TENANT), None, "one more byte");
    let theirs = built(|fields| fields.key_env = "LARRY_KEY_0123456789ABCDEE_ALPHA")?;
    let refusal = RouteError::KeyEnvNotThisTenants {
        variable: String::from("LARRY_KEY_0123456789ABCDEE_ALPHA"),
        prefix,
    };
    assert_eq!(theirs.key_env_refusal(TENANT), Some(refusal));
    let bare = built(|fields| fields.key_env = "LARRY_KEY_0123456789ABCDEF_")?;
    let refused = bare.key_env_refusal(TENANT);
    assert!(refused.is_some(), "the bare namespace names nothing");
    Ok(())
}

/// The refusal set is closed and classified once: every [`RouteError`]
/// variant is built here and its [`RouteError::opens_with_route`] answer is
/// pinned, so a resurrected refusal (larry#546, larry#551) fails the build in
/// the classifier until it is listed there, and a reader sees the whole set
/// in one place. `KindNotServed` is deliberately absent: all three named
/// kinds are served, so no constructor can build it (SD-1).
#[test]
fn the_refusal_set_is_closed_with_no_dead_variant() {
    let refusals = [
        (RouteError::BadId, true),
        (RouteError::ReservedId, true),
        (RouteError::BadUrl, true),
        (
            RouteError::InsecureTransport {
                host: String::from("closed.test"),
            },
            false,
        ),
        (RouteError::BadModel, true),
        (RouteError::BadKeyEnv, true),
        (RouteError::KeyEnvOutsideNamespace, true),
        (
            RouteError::KeyEnvNotThisTenants {
                variable: String::from("LARRY_KEY_CLOSED"),
                prefix: String::from("LARRY_KEY_FEDCBA9876543210_"),
            },
            true,
        ),
        (RouteError::BadTemperature, true),
        (RouteError::BadKind, true),
        (RouteError::BadPrompt, true),
        (RouteError::BadResidency, true),
        (RouteError::BadRequestBytes, true),
        (RouteError::BadDescribe, true),
        (RouteError::BadPrice { field: "input" }, true),
        (
            RouteError::NoSuchId {
                id: String::from("closed-9"),
            },
            false,
        ),
        (RouteError::NoRoute, false),
        (RouteError::TableFull { limit: 512 }, false),
        (
            RouteError::Damaged {
                id: String::from("closed-1"),
                cause: None,
            },
            false,
        ),
        (
            RouteError::Damaged {
                id: String::from("closed-2"),
                cause: Some(Box::new(RouteError::BadKind)),
            },
            false,
        ),
    ];
    for (refusal, opens) in refusals {
        assert_eq!(
            refusal.opens_with_route(),
            opens,
            "the classification is pinned"
        );
        assert!(
            !refusal.to_string().is_empty(),
            "every refusal says something"
        );
    }
}

/// Every named kind parses back and seals with its name (larry#205,
/// larry#539). An unknown kind parses to nothing.
#[test]
fn each_kind_parses_and_seals_with_its_name() -> Result<(), RouteError> {
    for name in ["openai-chat", "anthropic-messages", "openai-responses"] {
        let kind = Kind::parse(name).ok_or(RouteError::BadKind)?;
        assert_eq!(kind.name(), name, "{name} round trips");
        let route = built(|fields| fields.kind = kind)?;
        let sealed = route.seal().to_string();
        assert!(
            sealed.contains(&format!("\"kind\":\"{name}\"")),
            "{name} seals: {sealed}"
        );
        assert!(
            sealed.contains("\"prompt_hash\":null"),
            "no template seals as null: {sealed}"
        );
    }
    assert_eq!(Kind::parse("other"), None, "an unknown kind");
    Ok(())
}

/// The prompt template is checked at its boundary and sealed by its hash, never
/// its text: a template past 8 KiB or with a control byte is refused, and the
/// seal names the BLAKE3 hash the stored template re-derives (larry#205).
#[test]
fn a_prompt_template_is_checked_and_sealed_by_its_hash() -> Result<(), RouteError> {
    let long = "p".repeat(8193);
    assert_eq!(
        built(|fields| fields.prompt = Some(&long)),
        Err(RouteError::BadPrompt),
        "8193 bytes"
    );
    let top = "p".repeat(8192);
    assert!(
        built(|fields| fields.prompt = Some(&top)).is_ok(),
        "8192 bytes"
    );
    assert_eq!(
        built(|fields| fields.prompt = Some("bad\u{1}byte")),
        Err(RouteError::BadPrompt),
        "a control byte"
    );
    let templated = built(|fields| fields.prompt = Some("be terse"))?;
    assert_eq!(
        templated.prompt(),
        Some("be terse"),
        "the template reads back"
    );
    let hash = templated.prompt_hash().ok_or(RouteError::BadPrompt)?;
    assert_eq!(hash.len(), 64, "the hash is 64 hex characters");
    assert_eq!(
        templated.prompt_hash(),
        Some(hash.clone()),
        "the hash is stable"
    );
    let sealed = templated.seal().to_string();
    assert!(
        sealed.contains(&format!("\"prompt_hash\":\"{hash}\"")),
        "the hash seals: {sealed}"
    );
    assert!(
        !sealed.contains("be terse"),
        "the template's text never seals: {sealed}"
    );
    Ok(())
}
