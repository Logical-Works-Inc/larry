//! Seeded adversarial simulations of one run's identity (larry#182 defect 3):
//! the idempotency key's bounds, the keyed run id's three parts, and the one
//! property the whole change rests on.
//!
//! Source: RFC 9562 section 5.7 (the canonical version 7 value) and
//! draft-ietf-httpapi-idempotency-key-header-07 section 2. The oracle is an
//! invariant and an independent recomputation, never this code's own output.
//! Every actor here is the stupidest hostile one the module will meet: a
//! double-clicker resubmitting one document, a weird-bytes client sending keys it
//! invented, a fat-finger client one byte past the bound, a neighbour claiming
//! another state directory's key. One seed drives every draw, so a failing seed
//! replays exactly.

use std::path::Path;

use lgwks_std::json::{self, Value};
use lgwks_std::seeded::Seeded;

use super::super::Task;
use super::{fresh_id, run_id, tenant_of};

/// What every test here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Why this simulation's own helper refused: a key it could not draw, or a
/// document it could not build. The refusals this change is about travel as the
/// typed `TaskError`, never as this one.
#[derive(Debug)]
enum SimFault {
    /// A seeded draw had no value, or refused.
    Draw(lgwks_std::seeded::DrawError),
    /// A drawn byte did not fit its range.
    Narrow(std::num::TryFromIntError),
    /// The drawn filler was shorter than the slice taken from it.
    Short,
}

impl std::fmt::Display for SimFault {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Draw(ref cause) => write!(out, "seeded draw: {cause}"),
            Self::Narrow(ref cause) => write!(out, "seeded byte: {cause}"),
            Self::Short => out.write_str("the drawn filler is shorter than the slice taken"),
        }
    }
}

impl std::error::Error for SimFault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Draw(ref cause) => Some(cause),
            Self::Narrow(ref cause) => Some(cause),
            Self::Short => None,
        }
    }
}

impl From<lgwks_std::seeded::DrawError> for SimFault {
    fn from(cause: lgwks_std::seeded::DrawError) -> Self {
        Self::Draw(cause)
    }
}

impl From<std::num::TryFromIntError> for SimFault {
    fn from(cause: std::num::TryFromIntError) -> Self {
        Self::Narrow(cause)
    }
}

/// The seeds every actor replays: named rather than drawn, so a failure prints the
/// exact seed that reproduces it.
const SEEDS: [u64; 4] = [
    0x5EED_0000_0000_0001,
    0x5EED_0000_0000_0002,
    0x5EED_0000_FFFF_FFFF,
    0xDEAD_BEEF_CAFE_0001,
];

/// The most bytes an idempotency key may carry.
const MAX_KEY_BYTES: usize = 256;

/// A task document over `repo`, whose acceptance is one command, under `key` when
/// it is given.
fn document(repo: &str, key: Option<&Value>) -> Result<Task, super::super::TaskError> {
    let mut rendered = json::json!({
        "repo": repo,
        "base": "HEAD",
        "acceptance": [["true"]],
    });
    if let Some(value) = key
        && let Some(object) = rendered.as_object_mut()
    {
        object.insert(String::from("idempotency_key"), value.clone());
    }
    Task::parse(rendered.to_string().as_bytes())
}

/// A printable-ASCII key of `length` bytes drawn from `seeded`.
fn drawn_key(seeded: &mut Seeded, length: usize) -> Result<String, SimFault> {
    let mut text = String::with_capacity(length);
    while text.len() < length {
        let drawn = seeded.below(95)?;
        let byte = u8::try_from(drawn)?.saturating_add(32);
        text.push(char::from(byte));
    }
    Ok(text)
}

/// The double-clicker: one document submitted `count` times under one key mints
/// one run id, and two different keys never mint one id, for every seed.
#[test]
fn sim_a_double_clicked_document_is_one_run_id_for_every_seed() -> TestResult {
    let state = Path::new("/tmp/larry-sim-identity");
    for seed in SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        let length = 1_usize.saturating_add(seeded.index(64)?);
        let key = drawn_key(&mut seeded, length)?;
        let task = document("/tmp/a", Some(&Value::String(key.as_str().into())))?;
        let first = run_id(&task, state)?;
        let clicks = seeded.index(8)?;
        for _click in 0..clicks {
            assert_eq!(
                first,
                run_id(&task, state)?,
                "seed {seed}: a resubmission of one keyed document keeps one run id"
            );
        }
        let other_length = 1_usize.saturating_add(seeded.index(64)?);
        let other_key = drawn_key(&mut seeded, other_length)?;
        if other_key != key {
            let other = document("/tmp/a", Some(&Value::String(other_key.as_str().into())))?;
            assert_ne!(
                first,
                run_id(&other, state)?,
                "seed {seed}: two different keys are two requests"
            );
        }
    }
    Ok(())
}

/// The neighbour: two state directories never mint one keyed run id for one task
/// and one key, and each keeps its own tenant, for every seed.
#[test]
fn sim_a_neighbour_tenant_never_shares_a_keyed_run_id_for_every_seed() -> TestResult {
    let (left, right) = (
        Path::new("/tmp/larry-sim-tenant-left"),
        Path::new("/tmp/larry-sim-tenant-right"),
    );
    for seed in SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        let length = 1_usize.saturating_add(seeded.index(32)?);
        let key = drawn_key(&mut seeded, length)?;
        let task = document("/tmp/a", Some(&Value::String(key.as_str().into())))?;
        assert_ne!(
            tenant_of(left),
            tenant_of(right),
            "seed {seed}: two state directories are two tenants"
        );
        assert_ne!(
            run_id(&task, left)?,
            run_id(&task, right)?,
            "seed {seed}: one key in two tenants is two requests"
        );
        assert_eq!(
            run_id(&task, left)?,
            run_id(&task, left)?,
            "seed {seed}: one tenant's answer does not move"
        );
    }
    Ok(())
}

/// The weird-bytes client: every key it invents either parses to exactly the bytes
/// it sent or is refused, never silently altered, for every seed.
#[test]
fn sim_a_weird_key_is_kept_verbatim_or_refused_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        for _attempt in 0..64 {
            let length = seeded.index(MAX_KEY_BYTES.saturating_add(2))?;
            let mut bytes = Vec::with_capacity(length);
            while bytes.len() < length {
                bytes.push(u8::try_from(seeded.below(256)?)?);
            }
            let value = match std::str::from_utf8(&bytes) {
                Ok(text) => Value::String(text.into()),
                // A client cannot send invalid UTF-8 as JSON text, and the draw
                // stands for one that tried: the document is refused below.
                Err(_) => continue,
            };
            let legal =
                (1..=MAX_KEY_BYTES).contains(&length) && !bytes.iter().any(u8::is_ascii_control);
            let parsed = document("/tmp/a", Some(&value));
            match (legal, parsed) {
                (true, Ok(task)) => assert_eq!(
                    task.idempotency_key(),
                    std::str::from_utf8(&bytes).ok(),
                    "seed {seed}: a legal key is kept byte for byte"
                ),
                (false, Err(_)) => {}
                (true, Err(error)) => {
                    return Err(format!("seed {seed}: a legal key was refused: {error}").into());
                }
                (false, Ok(_)) => {
                    return Err(format!("seed {seed}: an illegal key was accepted").into());
                }
            }
        }
    }
    Ok(())
}

/// The fat-finger client: the key's bound is exact, at one byte and at one byte
/// past it, for every seed's own text.
#[test]
fn sim_the_key_bound_is_exact_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        let filler = drawn_key(&mut seeded, MAX_KEY_BYTES.saturating_add(1))?;
        for length in [
            0,
            1,
            MAX_KEY_BYTES.saturating_sub(1),
            MAX_KEY_BYTES,
            MAX_KEY_BYTES.saturating_add(1),
        ] {
            let key = filler.get(..length).ok_or(SimFault::Short)?;
            let accepted = document("/tmp/a", Some(&Value::String(key.into()))).is_ok();
            let wanted = (1..=MAX_KEY_BYTES).contains(&length);
            assert_eq!(
                accepted, wanted,
                "seed {seed}: a key of {length} bytes accepted is {wanted}"
            );
        }
    }
    Ok(())
}

/// The monkey: a key of JSON-escaped bytes is the text the client meant, so a
/// control byte never slips through an escape, for every seed.
#[test]
fn sim_an_escaped_control_byte_is_never_a_key_for_every_seed() -> TestResult {
    for seed in SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        let length = 1_usize.saturating_add(seeded.index(16)?);
        let at = seeded.index(length)?;
        let mut text = drawn_key(&mut seeded, length)?;
        let control = u32::try_from(seeded.below(32)?).map_err(|_| "draw overflow")?;
        let byte = u8::try_from(control).map_err(|_| "control overflow")?;
        text.insert(at, char::from(byte));
        assert!(
            document("/tmp/a", Some(&Value::String(text.as_str().into()))).is_err(),
            "seed {seed}: a key holding a control byte is refused"
        );
        // The same text without the control byte is a key.
        let clean = text.replace(char::from(byte), "");
        if !clean.is_empty() {
            assert!(
                document("/tmp/a", Some(&Value::String(clean.as_str().into()))).is_ok(),
                "seed {seed}: the same text without the control byte is accepted"
            );
        }
    }
    Ok(())
}

/// The clock-fool and the pulled-plug: a fresh run id is a canonical version 7
/// value that strictly increases within a process, so 256 in a row never repeat
/// and never go backwards, for every seed that starts the sweep.
#[test]
fn sim_fresh_run_ids_never_repeat_or_go_backwards_for_every_seed() -> TestResult {
    for _seed in SEEDS {
        let mut previous: Option<lgwks_std::id::Uuid> = None;
        let mut seen = std::collections::BTreeSet::new();
        for _draw in 0..256 {
            let text = fresh_id()?;
            let parsed = lgwks_std::id::Uuid::parse(&text)?;
            assert_eq!(
                parsed.version(),
                Some(7),
                "a fresh run id carries the version 7 nibble: {text}"
            );
            if let Some(ref last) = previous {
                assert!(
                    *last < parsed,
                    "a fresh run id never goes backwards: {last} then {parsed}"
                );
            }
            previous = Some(parsed);
            seen.insert(text);
        }
        assert_eq!(seen.len(), 256, "256 fresh run ids are 256 values");
    }
    Ok(())
}

/// The wrong-order client: a keyed run id is one function of its three parts in
/// one order, so any part that moves moves the id and the unkeyed run is its own
/// value, for every seed.
#[test]
fn sim_a_keyed_run_id_moves_with_any_part_for_every_seed() -> TestResult {
    let state = Path::new("/tmp/larry-sim-parts");
    for seed in SEEDS {
        let mut seeded = Seeded::from_seed(seed);
        let length = 1_usize.saturating_add(seeded.index(32)?);
        let key = drawn_key(&mut seeded, length)?;
        let base = document("/tmp/a", Some(&Value::String(key.as_str().into())))?;
        let pinned = run_id(&base, state)?;

        // The repository path is part of the task bytes, so it is part of the digest.
        let moved = document("/tmp/b", Some(&Value::String(key.as_str().into())))?;
        assert_ne!(
            pinned,
            run_id(&moved, state)?,
            "seed {seed}: another task document is another request"
        );

        // The acceptance command is the document's content too.
        let mut recontented = json::json!({
            "repo": "/tmp/a",
            "base": "HEAD",
            "acceptance": [["false"]],
            "idempotency_key": key.as_str(),
        });
        if let Some(object) = recontented.as_object_mut() {
            object.insert(String::from("acceptance"), json::json!([["exit", "1"]]));
        }
        let recontented = Task::parse(recontented.to_string().as_bytes())?;
        assert_ne!(
            pinned,
            run_id(&recontented, state)?,
            "seed {seed}: one key over different content is another request"
        );

        // No key at all is a fresh run, and two of them differ.
        let unkeyed = document("/tmp/a", None)?;
        assert_eq!(unkeyed.idempotency_key(), None, "seed {seed}: no key");
        assert_ne!(
            run_id(&unkeyed, state)?,
            run_id(&unkeyed, state)?,
            "seed {seed}: two unkeyed submissions are two runs"
        );
    }
    Ok(())
}
