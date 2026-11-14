//! Seeded adversarial simulations of storing routes (larry#201), driven through
//! `larry route`'s own entry point, the code the shipped binary calls.
//!
//! Source: the route table design of larry#201, SQLite upsert and
//! single-statement atomicity for the 256-route cap, and ISO 3166-1 alpha-2 for
//! the residency shape. Requirement proved: a hostile route field is refused by
//! its own variant and stores nothing, the cap holds under concurrent adds and
//! recovers after a remove, a replayed add is one row, and one seed gives one
//! trace hash. Every actor is a fool: a double-clicker, a fat-finger and a
//! saboteur who writes escape bytes.

#[path = "support/route_cli.rs"]
mod route_cli;

use std::error::Error;
use std::path::Path;
use std::sync::Barrier;

use larry_demo::executor::routes::RouteCliError;
use larry_demo::model::RouteError;
use lgwks_deps::storefront::rusqlite::Connection;
use lgwks_std::hash::Hasher;
use lgwks_std::seeded::Seeded;
use route_cli::{GATEWAY, POOL, Scratch, TestResult, add, joined, listed, parse, route_run};

/// The most routes a tenant holds.
const CAP: usize = 256;

/// One attempt to add a route under concurrency.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The route was stored.
    Saved,
    /// The table refused it as full.
    Full,
    /// Anything else, which no case here expects.
    Other(String),
}

/// Hyperscale, the double-clicker and the neighbour: 8 threads add 320 distinct
/// routes at once to one tenant. Exactly 256 are stored and 64 are refused as
/// full, whatever the interleaving; an update at the cap still passes; a
/// remove frees exactly one place. Bug caught: a count-then-insert race that
/// lets two adders both see 255 and store 257 routes.
#[test]
fn sim_route_cap_refuses_the_257th_and_keeps_256() -> TestResult {
    let state = Scratch::new()?;
    let mut rng = Seeded::from_seed(0x5EED_0201);
    let mut ids = Vec::new();
    for index in 0..320_u32 {
        ids.push(format!("r{index}-{}", rng.below(1_000_000)?));
    }
    let barrier = Barrier::new(8);
    let outcomes: Vec<Outcome> =
        std::thread::scope(|scope| -> Result<Vec<Outcome>, Box<dyn Error>> {
            let mut handles = Vec::new();
            for chunk in ids.chunks(40) {
                let (barrier, path) = (&barrier, state.path());
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    chunk
                        .iter()
                        .map(|id| match add(path, id, &[], &[]) {
                            Ok(_) => Outcome::Saved,
                            Err(RouteCliError::Route(RouteError::TableFull { limit }))
                                if limit == CAP =>
                            {
                                Outcome::Full
                            }
                            Err(other) => Outcome::Other(other.to_string()),
                        })
                        .collect::<Vec<Outcome>>()
                }));
            }
            let mut all = Vec::new();
            for handle in handles {
                all.extend(joined(handle)?);
            }
            Ok(all)
        })?;
    let count = |wanted: Outcome| {
        outcomes
            .iter()
            .filter(|outcome| **outcome == wanted)
            .count()
    };
    assert_eq!(
        (count(Outcome::Saved), count(Outcome::Full), outcomes.len()),
        (CAP, 64, 320),
        "256 stored, 64 refused as full: {outcomes:?}"
    );
    let (stored, _) = listed(state.path())?;
    assert_eq!(stored.len(), CAP, "the table keeps exactly 256");
    let first = stored.first().ok_or("a stored route")?;
    add(state.path(), first, &[("--model", "model-2")], &[])?;
    assert_eq!(
        listed(state.path())?.0.len(),
        CAP,
        "an update at the cap adds no row"
    );
    route_run(state.path(), &[], &["remove", first])?;
    add(state.path(), "late-comer", &[], &[])?;
    assert!(
        matches!(
            add(state.path(), "one-too-many", &[], &[]),
            Err(RouteCliError::Route(RouteError::TableFull { limit: CAP }))
        ),
        "the table is full again after one remove and one add"
    );
    assert_eq!(
        listed(state.path())?.0.len(),
        CAP,
        "recovered to 256, not 257"
    );
    Ok(())
}

/// A garbage value for a fat-finger or a saboteur: letters, digits, spaces,
/// escape and newline bytes and a non-ASCII letter.
fn garbage(rng: &mut Seeded) -> Result<String, Box<dyn Error>> {
    /// The pieces a garbage value is built from.
    const TOKENS: [&str; 12] = [
        "a", "Z", "0", "_", "-", " ", "\u{1b}", "\n", "\u{e9}", "A", "B", ".",
    ];
    let length = rng.below(6)?;
    let mut text = String::new();
    for _ in 0..length {
        let index = usize::try_from(rng.below(12)?)?;
        text.push_str(TOKENS.get(index).ok_or("a token")?);
    }
    Ok(text)
}

/// The reason an independent reading of the written rules refuses `text` as the
/// value of `field`, if any.
fn reference(field: &str, text: &str) -> Option<RouteError> {
    let mut chars = text.chars();
    let (fine, refusal) = match field {
        "id" => (
            !text.is_empty()
                && text.len() <= 64
                && text.chars().all(|ch| {
                    ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-'
                }),
            RouteError::BadId,
        ),
        "--model" => (
            !text.is_empty() && text.len() <= 256 && text.chars().all(|ch| ch.is_ascii_graphic()),
            RouteError::BadModel,
        ),
        "--key-env" => {
            let shaped = chars
                .next()
                .is_some_and(|first| first.is_ascii_uppercase() || first == '_')
                && text.len() <= 64
                && chars.all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_');
            let namespaced = text
                .strip_prefix("LARRY_KEY_")
                .is_some_and(|rest| !rest.is_empty());
            (
                shaped && namespaced,
                if shaped {
                    RouteError::KeyEnvOutsideNamespace
                } else {
                    RouteError::BadKeyEnv
                },
            )
        }
        "--residency" => (
            text.chars().count() == 2 && text.chars().all(|ch| ch.is_ascii_uppercase()),
            RouteError::BadResidency,
        ),
        _ => (
            text.len() <= 1024 && text.chars().all(|ch| ch == '\n' || !ch.is_control()),
            RouteError::BadDescribe,
        ),
    };
    (!fine).then_some(refusal)
}

/// The fat-finger and the saboteur: every field of `route add`, one hostile
/// value at a time, gets its own exact variant and stores nothing; then 256
/// seeded garbage values agree with an independent reading of the rules. Bug
/// caught: a field check that returns another field's variant, or lets an
/// escape byte, a credential or a query into the store.
#[test]
fn sim_hostile_route_fields_are_typed_refusals() -> TestResult {
    let state = Scratch::new()?;
    let long_id = "a".repeat(65);
    let long_url = format!("{GATEWAY}/{}", "p".repeat(2048));
    let long_describe = "d".repeat(1025);
    let cases: Vec<(&str, &str, &str, RouteError)> = vec![
        (long_id.as_str(), "", "", RouteError::BadId),
        ("UPPER", "", "", RouteError::BadId),
        ("a b", "", "", RouteError::BadId),
        ("env", "", "", RouteError::ReservedId),
        ("ok", "--url", "ftp://gateway.test/v1", RouteError::BadUrl),
        (
            "ok",
            "--url",
            "http://u:p@gateway.test/v1",
            RouteError::BadUrl,
        ),
        (
            "ok",
            "--url",
            "http://gateway.test/v1?k=1",
            RouteError::BadUrl,
        ),
        (
            "ok",
            "--url",
            "http://gateway.test/v1#frag",
            RouteError::BadUrl,
        ),
        ("ok", "--url", "gateway.test/v1", RouteError::BadUrl),
        ("ok", "--url", long_url.as_str(), RouteError::BadUrl),
        ("ok", "--model", "", RouteError::BadModel),
        ("ok", "--model", "two words", RouteError::BadModel),
        ("ok", "--key-env", "lower", RouteError::BadKeyEnv),
        ("ok", "--key-env", "1ABC", RouteError::BadKeyEnv),
        ("ok", "--key-env", "A-B", RouteError::BadKeyEnv),
        (
            "ok",
            "--key-env",
            "GH_TOKEN",
            RouteError::KeyEnvOutsideNamespace,
        ),
        (
            "ok",
            "--key-env",
            "LARRY_MODEL_KEY",
            RouteError::KeyEnvOutsideNamespace,
        ),
        (
            "ok",
            "--key-env",
            "LARRY_KEY_",
            RouteError::KeyEnvOutsideNamespace,
        ),
        ("ok", "--temperature", "3", RouteError::BadTemperature),
        ("ok", "--temperature", "-1", RouteError::BadTemperature),
        ("ok", "--temperature", "nan", RouteError::BadTemperature),
        ("ok", "--temperature", "inf", RouteError::BadTemperature),
        (
            "ok",
            "--request-bytes",
            "16777217",
            RouteError::BadRequestBytes,
        ),
        ("ok", "--request-bytes", "0", RouteError::BadRequestBytes),
        ("ok", "--request-bytes", "1023", RouteError::BadRequestBytes),
        ("ok", "--residency", "can", RouteError::BadResidency),
        ("ok", "--residency", "C", RouteError::BadResidency),
        ("ok", "--residency", "ca", RouteError::BadResidency),
        (
            "ok",
            "--describe",
            long_describe.as_str(),
            RouteError::BadDescribe,
        ),
        (
            "ok",
            "--describe",
            "red \u{1b}[31m",
            RouteError::BadDescribe,
        ),
    ];
    for (id, flag, value, expected) in cases {
        let overrides: Vec<(&str, &str)> = if flag.is_empty() {
            Vec::new()
        } else {
            vec![(flag, value)]
        };
        match add(state.path(), id, &overrides, &[]) {
            Err(RouteCliError::Route(ref got)) if *got == expected => {}
            other => {
                return Err(
                    format!("{flag} {value:.30}: wanted {expected:?}, got {other:?}").into(),
                );
            }
        }
    }
    assert_eq!(
        listed(state.path())?.0.len(),
        0,
        "no refusal stored anything"
    );
    let no_residency = [
        "add",
        "ok",
        "--url",
        GATEWAY,
        "--model",
        "m",
        "--key-env",
        "LARRY_KEY_K",
    ];
    assert!(
        matches!(
            route_run(state.path(), &[], &no_residency),
            Err(RouteCliError::Missing {
                flag: "--residency"
            })
        ),
        "a route without a residency is refused naming the flag"
    );
    assert!(
        matches!(
            add(state.path(), "ok", &[("--temperature", "warm")], &[]),
            Err(RouteCliError::BadNumber {
                flag: "--temperature",
                ..
            })
        ),
        "a temperature that is not a number is refused naming the flag"
    );
    for extra in [["--bogus", "1"], ["--url", GATEWAY]] {
        assert!(
            matches!(
                add(state.path(), "ok", &[], &extra),
                Err(RouteCliError::Usage(_))
            ),
            "{extra:?}: an unknown or repeated flag is a usage refusal"
        );
    }
    assert_eq!(
        listed(state.path())?.0.len(),
        0,
        "the refusals stored nothing"
    );

    let mut rng = Seeded::from_seed(0xFA7F1);
    let fields = ["id", "--model", "--key-env", "--residency", "--describe"];
    for case in 0..256_usize {
        let mut value = garbage(&mut rng)?;
        let flag = fields.get(case % fields.len()).ok_or("a field")?;
        if *flag == "--key-env" && rng.below(2)? == 0 {
            value.insert_str(0, "LARRY_KEY_");
        }
        let outcome = if *flag == "id" {
            add(state.path(), &value, &[], &[])
        } else {
            add(state.path(), "fuzzed", &[(flag, value.as_str())], &[])
        };
        match (outcome, reference(flag, &value)) {
            (Ok(_), None) => {}
            (Err(RouteCliError::Route(ref got)), Some(ref want)) if got == want => {}
            (other, want) => {
                return Err(format!(
                    "case {case} {flag} {value:?}: wanted {want:?}, got {other:?}"
                )
                .into());
            }
        }
    }
    Ok(())
}

/// Ephemeral and the double-clicker: the same add replayed 200 times in a row
/// and by 8 threads at once (each reopening the store, as a restart does)
/// leaves exactly one row, and the default flag set by any replay stays. Bug
/// caught: an insert without an upsert that duplicates or errors on the second
/// click, or a replay that clears the default.
#[test]
fn sim_route_add_replayed_is_one_row() -> TestResult {
    let state = Scratch::new()?;
    for _ in 0..200 {
        add(state.path(), "alpha", &[], &["--default"])?;
    }
    assert_eq!(
        listed(state.path())?,
        (vec![String::from("alpha")], Some(String::from("alpha"))),
        "200 sequential replays are one row and one default"
    );
    let models = ["m0", "m1", "m2", "m3", "m4", "m5", "m6", "m7"];
    let barrier = Barrier::new(models.len());
    std::thread::scope(|scope| -> TestResult {
        let mut handles = Vec::new();
        for model in models {
            let (barrier, path) = (&barrier, state.path());
            handles.push(scope.spawn(move || {
                barrier.wait();
                (0..25)
                    .map(|_| {
                        add(path, "alpha", &[("--model", model)], &[])
                            .map(drop)
                            .map_err(|error| error.to_string())
                    })
                    .collect::<Result<Vec<()>, String>>()
            }));
        }
        for handle in handles {
            joined(handle)??;
        }
        Ok(())
    })?;
    let (ids, default) = listed(state.path())?;
    assert_eq!(ids, ["alpha"], "400 replays are one row");
    assert_eq!(
        default.as_deref(),
        Some("alpha"),
        "a replay without --default keeps the default"
    );
    let value = parse(&route_run(state.path(), &[], &["list", "--json"])?)?;
    let model = value["routes"][0]["model"].as_str().ok_or("a model")?;
    assert!(
        models.contains(&model),
        "the row holds one writer's whole value: {model}"
    );
    Ok(())
}

/// One seeded script of route operations against a fresh state directory, and
/// the BLAKE3 digest of every output, every refusal and every listing, in order.
fn trace_of(seed: u64) -> Result<String, Box<dyn Error>> {
    let state = Scratch::new()?;
    let mut rng = Seeded::from_seed(seed);
    let mut hasher = Hasher::new();
    for step in 0..48_u32 {
        let id = POOL.get(usize::try_from(rng.below(6)?)?).ok_or("an id")?;
        let model = format!("m{step}");
        let outcome = match rng.below(5)? {
            0 => add(state.path(), id, &[("--model", model.as_str())], &[]),
            1 => add(
                state.path(),
                id,
                &[("--model", model.as_str())],
                &["--default"],
            ),
            2 => route_run(state.path(), &[], &["remove", id]),
            3 => route_run(state.path(), &[], &["default", id]),
            _ => route_run(state.path(), &[], &["resolve", id]),
        };
        let line = match outcome {
            Ok(text) => text,
            Err(error) => error.to_string(),
        };
        hasher.write_framed(line.as_bytes());
        hasher.write_framed(route_run(state.path(), &[], &["list", "--json"])?.as_bytes());
    }
    Ok(hasher.finalize().to_hex())
}

/// Deterministic: one seed drives the whole script (the ids, the operations,
/// the models) and nothing reads a clock or the machine, so two fresh state
/// directories give one trace hash and another seed gives another.
#[test]
fn sim_same_seed_same_route_trace_hash() -> TestResult {
    for seed in 0..8_u64 {
        let first = trace_of(seed)?;
        assert_eq!(first, trace_of(seed)?, "seed {seed}: one seed, one hash");
        assert_ne!(
            first,
            trace_of(seed.saturating_add(100))?,
            "seed {seed}: another seed, another trace"
        );
    }
    Ok(())
}

/// Giant and empty: a key variable of every length from 0 to 72 bytes, cut from
/// the prefix and then padded with `A`. The written bounds are three ranges: a
/// name shorter than `LARRY_KEY_` plus one byte is empty (not a name) or outside
/// the namespace, 11 to 64 bytes is stored, and 65 or more is not a name. Bug
/// caught: a bound off by one on either side of the prefix or of the 64-byte cap.
#[test]
fn sim_key_variable_length_walk_matches_the_written_bounds() -> TestResult {
    let state = Scratch::new()?;
    let ladder = format!("LARRY_KEY_{}", "A".repeat(72));
    for length in 0..=72_usize {
        let name: String = ladder.chars().take(length).collect();
        let wanted = match length {
            0 | 65.. => Some(RouteError::BadKeyEnv),
            1..=10 => Some(RouteError::KeyEnvOutsideNamespace),
            11..=64 => None,
        };
        match (
            add(state.path(), "walk", &[("--key-env", name.as_str())], &[]),
            wanted,
        ) {
            (Ok(_), None) => {}
            (Err(RouteCliError::Route(ref got)), Some(ref want)) if got == want => {}
            (other, want) => {
                return Err(format!("{length} bytes: wanted {want:?}, got {other:?}").into());
            }
        }
    }
    let (ids, _) = listed(state.path())?;
    assert_eq!(ids, ["walk"], "the accepted lengths replaced one row");
    Ok(())
}

/// The monkey: 512 seeded one-to-three-key edits (delete, overwrite, insert) of
/// a valid `LARRY_KEY_ROUTER` with letters, a digit, a hyphen, a non-ASCII
/// letter and a space. Each result agrees with the reference reading of the
/// rules by exact variant, and the one row holds the last accepted name. Bug
/// caught: a check that reads only the prefix, or only the shape, so a mutant
/// slips through one of them.
#[test]
fn sim_monkey_edits_of_a_valid_key_variable_agree_with_the_reference() -> TestResult {
    /// What the monkey types.
    const KEYS: [char; 8] = ['L', 'Y', '_', 'a', '7', '-', '\u{c9}', ' '];
    let state = Scratch::new()?;
    let mut rng = Seeded::from_seed(0x0DDB_1750);
    let (mut accepted, mut refused) = (0_u32, 0_u32);
    let mut last_good: Option<String> = None;
    for case in 0..512_u32 {
        let mut chars: Vec<char> = "LARRY_KEY_ROUTER".chars().collect();
        for _ in 0..=rng.below(3)? {
            let at = usize::try_from(rng.below(u64::try_from(chars.len().saturating_add(1))?)?)?;
            let key = *KEYS.get(usize::try_from(rng.below(8)?)?).ok_or("a key")?;
            match rng.below(3)? {
                0 => {
                    chars = chars
                        .into_iter()
                        .enumerate()
                        .filter(|entry| entry.0 != at)
                        .map(|entry| entry.1)
                        .collect();
                }
                1 => {
                    if let Some(slot) = chars.get_mut(at) {
                        *slot = key;
                    }
                }
                _ => chars.insert(at, key),
            }
        }
        let name: String = chars.into_iter().collect();
        match (
            add(state.path(), "mutant", &[("--key-env", name.as_str())], &[]),
            reference("--key-env", &name),
        ) {
            (Ok(_), None) => {
                accepted = accepted.saturating_add(1);
                last_good = Some(name);
            }
            (Err(RouteCliError::Route(ref got)), Some(ref want)) if got == want => {
                refused = refused.saturating_add(1);
            }
            (other, want) => {
                return Err(format!("case {case} {name:?}: wanted {want:?}, got {other:?}").into());
            }
        }
    }
    assert!(
        accepted > 0 && refused > 0,
        "the monkey reached both answers: {accepted} accepted, {refused} refused"
    );
    let value = parse(&route_run(state.path(), &[], &["list", "--json"])?)?;
    assert_eq!(
        value["routes"][0]["key_env"].as_str(),
        last_good.as_deref(),
        "the one row holds the last accepted name"
    );
    Ok(())
}

/// Rewrites the key variable of every route row in `state`'s store, the way a
/// saboteur with the file in hand would.
fn set_key_env(state: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    let store = Connection::open(state.join("executor.db"))?;
    store.execute("UPDATE routes SET key_env = ?1", [name])?;
    Ok(())
}

/// The saboteur with the store file, and the neighbour: one tenant's row is
/// rewritten to name a process secret, a cloud key, the bare prefix and a
/// lower-case name. That tenant's list and resolve are refused as `Damaged`
/// naming the row and the exact check it fails, never read; the neighbour's
/// table answers throughout; putting a good name back recovers the victim. Bug
/// caught: a row read back without the checks a new route passes, so an edited
/// store sends the operator's secret to the tenant's host.
#[test]
fn sim_a_row_edited_outside_the_namespace_fails_its_own_tenant_alone() -> TestResult {
    let (victim, bystander) = (Scratch::new()?, Scratch::new()?);
    for state in [&victim, &bystander] {
        add(state.path(), "main", &[], &["--default"])?;
    }
    let edits = [
        ("GH_TOKEN", RouteError::KeyEnvOutsideNamespace),
        ("LARRY_MODEL_KEY", RouteError::KeyEnvOutsideNamespace),
        ("AWS_SECRET_ACCESS_KEY", RouteError::KeyEnvOutsideNamespace),
        ("LARRY_KEY_", RouteError::KeyEnvOutsideNamespace),
        ("lower", RouteError::BadKeyEnv),
    ];
    for (name, cause) in edits {
        set_key_env(victim.path(), name)?;
        for args in [&["list"][..], &["resolve"][..]] {
            match route_run(victim.path(), &[], args) {
                Err(RouteCliError::Route(RouteError::Damaged {
                    ref id,
                    cause: Some(ref inner),
                })) if id == "main" && **inner == cause => {}
                other => {
                    return Err(format!(
                        "{name} {args:?}: wanted Damaged({cause:?}), got {other:?}"
                    )
                    .into());
                }
            }
            route_run(bystander.path(), &[], args)?;
        }
    }
    set_key_env(victim.path(), "LARRY_KEY_BACK")?;
    let value = parse(&route_run(victim.path(), &[], &["list", "--json"])?)?;
    assert_eq!(
        value["routes"][0]["key_env"].as_str(),
        Some("LARRY_KEY_BACK"),
        "a good name back and the victim reads again"
    );
    Ok(())
}

/// Hyperscale and the neighbour: 8 threads at once, each adding 16 routes of its
/// own ids, a seeded half of them naming an honest `LARRY_KEY_T<n>` variable and
/// the rest naming a process secret or the bare prefix. The plan is drawn before
/// any thread starts; every add is answered as the reference says, and the
/// table ends holding exactly the honest ids and no secret's name. Bug caught: a
/// check done after the write, or a shared scratch that lets a hostile add leave
/// a row behind.
#[test]
fn sim_hostile_and_honest_adders_at_once_leave_exactly_the_honest_rows() -> TestResult {
    /// What the saboteur threads try to store.
    const HOSTILE: [&str; 5] = [
        "GH_TOKEN",
        "LARRY_MODEL_KEY",
        "LARRY_KEY_",
        "AWS_SECRET_ACCESS_KEY",
        "HOME",
    ];
    let state = Scratch::new()?;
    let mut rng = Seeded::from_seed(0x5AB0_7E42);
    let mut plans: Vec<Vec<(String, String)>> = Vec::new();
    for thread in 0..8_u32 {
        let mut plan = Vec::new();
        for step in 0..16_u32 {
            let key = if rng.below(2)? == 0 {
                format!("LARRY_KEY_T{thread}")
            } else {
                String::from(
                    *HOSTILE
                        .get(usize::try_from(rng.below(5)?)?)
                        .ok_or("a hostile name")?,
                )
            };
            plan.push((format!("t{thread}-{step}"), key));
        }
        plans.push(plan);
    }
    let barrier = Barrier::new(plans.len());
    std::thread::scope(|scope| -> TestResult {
        let mut handles = Vec::new();
        for plan in &plans {
            let (barrier, path) = (&barrier, state.path());
            handles.push(scope.spawn(move || -> Result<(), String> {
                barrier.wait();
                for entry in plan {
                    let (id, key) = (&entry.0, &entry.1);
                    match (
                        add(path, id, &[("--key-env", key.as_str())], &[]),
                        reference("--key-env", key),
                    ) {
                        (Ok(_), None) => {}
                        (Err(RouteCliError::Route(ref got)), Some(ref want)) if got == want => {}
                        (other, want) => {
                            return Err(format!("{id} {key}: wanted {want:?}, got {other:?}"));
                        }
                    }
                }
                Ok(())
            }));
        }
        for handle in handles {
            joined(handle)??;
        }
        Ok(())
    })?;
    let mut honest: Vec<&str> = plans
        .iter()
        .flatten()
        .filter(|entry| reference("--key-env", &entry.1).is_none())
        .map(|entry| entry.0.as_str())
        .collect();
    honest.sort_unstable();
    let (stored, _) = listed(state.path())?;
    assert_eq!(stored, honest, "exactly the honest ids are stored");
    let value = parse(&route_run(state.path(), &[], &["list", "--json"])?)?;
    for route in value["routes"].as_array().ok_or("a routes array")? {
        let key = route["key_env"].as_str().ok_or("a key variable")?;
        assert!(key.starts_with("LARRY_KEY_T"), "{key} is an honest name");
    }
    Ok(())
}
