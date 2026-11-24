//! Seeded simulations of the environment snapshot every setting reads (larry#213).
//!
//! Requirement proved: a variable Larry reads is unset, valid UTF-8 text kept exactly, or bytes
//! that are not text and refused by name, never read as unset or as a default; the refusal names
//! the variable and never its value, which may be a secret; and an unset `PATH` is a typed
//! refusal, not the empty string. Source: POSIX `environ` (names and values are bytes) and Rust's
//! `OsString::into_string`, the one place bytes become text. The reader seam takes the variables
//! as a value, so no case sets the process environment.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use lgwks_std::seeded::Seeded;

use super::{ExecError, Settings, snapshot};
use crate::sim_error::SimError;

/// What every test here returns.
type TestResult = Result<(), SimError>;

/// Seeds each simulation sweeps.
const SEEDS: u64 = 64;

/// The longest value a seed draws, in bytes.
const MAX_BYTES: u64 = 16;

/// A variable Larry reads, a secret among them.
const READ_NAME: &str = "LARRY_RECEIPT_KEY";

/// A variable Larry never reads: its bytes are no concern of the snapshot.
const OTHER_NAME: &str = "EDITOR_BYTES";

/// One drawn value: printable ASCII, or ASCII then a two-byte character, or arbitrary bytes
/// ending in one no UTF-8 text holds.
fn drawn_bytes(stream: &mut Seeded) -> Result<Vec<u8>, SimError> {
    let length = usize::try_from(stream.below(MAX_BYTES)?)?;
    let class = stream.below(3)?;
    let mut bytes = Vec::with_capacity(length.saturating_add(2));
    for _ in 0..length {
        let byte = if class == 2 {
            u8::try_from(stream.below(256)?)?
        } else {
            b'!'.saturating_add(u8::try_from(stream.below(94)?)?)
        };
        bytes.push(byte);
    }
    match class {
        1 => bytes.extend_from_slice("\u{e9}".as_bytes()),
        2 => bytes.push(0xFF),
        _ => {}
    }
    Ok(bytes)
}

/// One seed's snapshot over a drawn value of [`READ_NAME`] and a drawn value of [`OTHER_NAME`],
/// checked against the standard library's own UTF-8 test. Returns what the seed did.
fn snapshot_trace(seed: u64) -> Result<String, SimError> {
    let mut stream = Seeded::from_seed(seed.wrapping_add(3 << 40));
    let value = drawn_bytes(&mut stream)?;
    let other = drawn_bytes(&mut stream)?;
    let vars = vec![
        (OsString::from(READ_NAME), OsString::from_vec(value.clone())),
        (OsString::from(OTHER_NAME), OsString::from_vec(other)),
    ];
    match (std::str::from_utf8(&value), snapshot(vars)) {
        (Ok(text), Ok(table)) => {
            assert_eq!(
                table.get(READ_NAME).map(String::as_str),
                Some(text),
                "seed {seed}: valid text is kept exactly"
            );
            assert!(
                !table.contains_key(OTHER_NAME),
                "seed {seed}: a variable Larry never reads is not in the snapshot"
            );
            Ok(format!("text len={}", text.len()))
        }
        (Err(_), Err(ExecError::NotUnicode { name })) => {
            assert_eq!(
                name, READ_NAME,
                "seed {seed}: the refusal names the variable"
            );
            let shown = ExecError::NotUnicode { name }.to_string();
            assert!(
                shown.contains(READ_NAME) && !shown.contains('\u{FFFD}'),
                "seed {seed}: the line names the variable and carries no value: {shown}"
            );
            Ok(format!("refused len={}", value.len()))
        }
        (Ok(_), Err(_)) | (Err(_), Ok(_)) | (Err(_), Err(_)) => Err(SimError::Missing(
            "the snapshot disagreed with the standard library's UTF-8 check",
        )),
    }
}

/// BLAKE3 of a trace, as hex: one seed must give one trace hash.
fn trace_hash(trace: &str) -> String {
    let mut hasher = lgwks_std::hash::Hasher::new();
    hasher.update(trace.as_bytes());
    hasher.finalize().to_hex()
}

/// An unset variable is absent, valid UTF-8 is kept exactly and bytes that are not text refuse
/// the whole snapshot naming the variable, over 64 seeded values; one seed gives one trace hash.
#[test]
fn sim_env_setting_unset_valid_and_non_unicode_get_their_answers() -> TestResult {
    for seed in 0..SEEDS {
        let first = trace_hash(&snapshot_trace(seed)?);
        let second = trace_hash(&snapshot_trace(seed)?);
        assert_eq!(first, second, "seed {seed}: one seed gives one trace hash");
    }
    let unset = snapshot(Vec::new())?;
    assert!(unset.is_empty(), "an empty environment is an empty table");
    Ok(())
}

/// A name that is not text and begins as one Larry reads is refused too: bytes in a name are no
/// more readable than bytes in a value.
#[test]
fn sim_a_non_unicode_name_is_refused_not_skipped() {
    let mut name = b"LARRY_".to_vec();
    name.push(0xFF);
    let outcome = snapshot(vec![(OsString::from_vec(name), OsString::from("1"))]);
    assert!(
        matches!(outcome, Err(ExecError::NotUnicode { .. })),
        "a variable of bytes under Larry's prefix is refused: {outcome:?}"
    );
}

/// `PATH` has no default: over 64 seeded paths each is kept exactly, and a table without one is
/// a typed refusal that names it, never the empty string.
#[test]
fn sim_an_unset_path_is_refused_and_a_set_one_is_kept() -> TestResult {
    for seed in 0..SEEDS {
        let mut stream = Seeded::from_seed(seed.wrapping_add(5 << 40));
        let length = usize::try_from(stream.below(MAX_BYTES)?)?;
        let mut path = String::new();
        for _ in 0..length {
            path.push(char::from(
                b'!'.saturating_add(u8::try_from(stream.below(94)?)?),
            ));
        }
        let kept = settings_with(Some(path.clone()))?;
        assert_eq!(kept.path, path, "seed {seed}: the PATH is kept exactly");
        let refused = settings_with(None);
        assert!(
            matches!(refused, Err(ExecError::PathUnset)),
            "seed {seed}: an unset PATH is refused typed: {refused:?}"
        );
    }
    let shown = ExecError::PathUnset.to_string();
    assert!(
        shown.contains("PATH"),
        "the refusal names the variable: {shown}"
    );
    Ok(())
}

/// Settings over a state directory and `path` as `PATH`, which `None` leaves unset.
fn settings_with(path: Option<String>) -> Result<Settings, ExecError> {
    Settings::from_env(move |name| match name {
        "LARRY_STATE_DIR" => Some(String::from("state")),
        "PATH" => path.clone(),
        _ => None,
    })
}
