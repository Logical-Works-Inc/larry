//! Seeded simulations of a tenant's ceilings and a task's deadline (larry#439, ADR 0111).
//! Source: OWASP ASVS 4.0.3 §1.4.1 (a limit holds at the trusted boundary) and POSIX `sh -c`.
//! One seed draws the tenant's ceilings and the task's values; the refusal is exactly the value
//! past its ceiling, and the deadline is read on a virtual clock, never the wall (SD-4).

use std::time::Duration;

use lgwks_bot::kernel::clock::Clock;
use lgwks_std::seeded::Seeded;

use super::{Ceilings, Deadline, Limits};
use crate::executor::{ExecError, Task, TaskError};

/// Seeds every simulation here sweeps.
const SEEDS: u64 = 256;

/// What every simulation here returns.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A task document naming `base`, with the raw JSON members `members` appended.
fn task(base: &str, members: &str) -> Result<Task, TaskError> {
    let document = format!(r#"{{"repo":"/tmp/r","base":"{base}","acceptance":[["c"]]{members}}}"#);
    Task::parse(document.as_bytes())
}

/// The ceilings an environment of `pairs` names.
fn ceilings(pairs: &[(&str, String)]) -> Result<(Ceilings, Duration), ExecError> {
    let read = |name: &str| {
        pairs
            .iter()
            .find(|pair| pair.0 == name)
            .map(|pair| pair.1.clone())
    };
    Ceilings::from_env(&read, None, 600_000)
}

/// For every seed, a task's timeout and deadline are admitted exactly when they are at or under
/// the tenant's ceiling, and a refusal names the key; a task that names neither runs under the
/// default cut to the ceiling and the deadline ceiling itself.
#[test]
fn sim_a_value_is_refused_exactly_when_it_passes_its_ceiling() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let max_timeout = rng.below(1_000_000)?.saturating_add(1);
        let max_deadline = rng.below(1_000_000)?.saturating_add(1);
        let (tenant, default) = ceilings(&[
            ("LARRY_EXEC_MAX_TIMEOUT_MS", max_timeout.to_string()),
            ("LARRY_EXEC_MAX_DEADLINE_MS", max_deadline.to_string()),
        ])?;
        assert_eq!(
            default,
            Duration::from_millis(600_000_u64.min(max_timeout)),
            "seed {seed}: the default timeout is cut to the ceiling"
        );
        let timeout = rng.below(2_000_000)?.saturating_add(1);
        let deadline = rng.below(2_000_000)?.saturating_add(1);
        let named = task(
            "HEAD",
            &format!(r#","timeout_ms":{timeout},"deadline_ms":{deadline}"#),
        )?;
        match tenant.limits(&named, default) {
            Ok(limits) => {
                assert!(
                    timeout <= max_timeout && deadline <= max_deadline,
                    "seed {seed}"
                );
                assert_eq!(limits.timeout, Duration::from_millis(timeout));
                assert_eq!(limits.deadline_ms, deadline);
            }
            Err(ExecError::PastCeiling { what, .. }) => {
                let expected = if timeout > max_timeout {
                    "timeout_ms"
                } else {
                    "deadline_ms"
                };
                assert!(
                    timeout > max_timeout || deadline > max_deadline,
                    "seed {seed}"
                );
                assert_eq!(
                    what, expected,
                    "seed {seed}: the first value past names itself"
                );
            }
            Err(other) => return Err(other.into()),
        }
        // The boundary itself: a value at its ceiling is admitted, one past it is refused.
        let at = task(
            "HEAD",
            &format!(r#","timeout_ms":{max_timeout},"deadline_ms":{max_deadline}"#),
        )?;
        let admitted = tenant.limits(&at, default)?;
        assert_eq!(
            (admitted.timeout, admitted.deadline_ms),
            (Duration::from_millis(max_timeout), max_deadline),
            "seed {seed}: a value at its ceiling is admitted"
        );
        for (key, value) in [
            ("timeout_ms", max_timeout.saturating_add(1)),
            ("deadline_ms", max_deadline.saturating_add(1)),
        ] {
            let past = task("HEAD", &format!(r#","{key}":{value}"#))?;
            assert!(
                matches!(tenant.limits(&past, default), Err(ExecError::PastCeiling { what, .. }) if what == key),
                "seed {seed}: {key} one past its ceiling is refused by name"
            );
        }
        let unnamed = tenant.limits(&task("HEAD", "")?, default)?;
        assert_eq!(
            unnamed,
            Limits {
                timeout: default,
                deadline_ms: max_deadline
            },
            "seed {seed}: a task naming neither runs under the tenant's own"
        );
    }
    Ok(())
}

/// A shell outside the tenant's list and a base no pattern admits are refused by name; an exact
/// base and a prefix pattern admit; an explicit default past the ceiling and a malformed
/// setting are refused naming the variable.
#[test]
fn shells_bases_and_bad_settings_are_refused_by_name() -> TestResult {
    let (tenant, default) = ceilings(&[
        ("LARRY_EXEC_SHELLS", String::from("sh,bash")),
        ("LARRY_EXEC_BASES", String::from("HEAD,release/*")),
    ])?;
    for (base, members, refused) in [
        ("HEAD", r#","shell":"bash""#, None),
        ("release/2026-11", "", None),
        ("HEAD", r#","shell":"zsh""#, Some("shell")),
        ("main", "", Some("base")),
        ("releasex", "", Some("base")),
    ] {
        match (tenant.limits(&task(base, members)?, default), refused) {
            (Ok(_), None) => {}
            (Err(ExecError::PastCeiling { what, .. }), Some(name)) => {
                assert_eq!(what, name, "{base} {members}");
            }
            (outcome, _) => return Err(format!("{base} {members}: {outcome:?}").into()),
        }
    }
    let read = |_: &str| None;
    assert!(matches!(
        Ceilings::from_env(&read, None, 600_000),
        Ok((_, timeout)) if timeout == Duration::from_millis(600_000)
    ));
    let low = |name: &str| (name == "LARRY_EXEC_MAX_TIMEOUT_MS").then(|| String::from("10"));
    assert!(
        matches!(
            Ceilings::from_env(&low, Some(10), 600_000),
            Ok((_, timeout)) if timeout == Duration::from_millis(10)
        ),
        "an explicit default at the ceiling is admitted"
    );
    assert!(matches!(
        Ceilings::from_env(&low, Some(11), 600_000),
        Err(ExecError::BadCeiling {
            name: "LARRY_EXEC_TIMEOUT_MS",
            ..
        })
    ));
    for (name, value) in [
        ("LARRY_EXEC_MAX_TIMEOUT_MS", "0"),
        ("LARRY_EXEC_MAX_DEADLINE_MS", "86400001"),
        ("LARRY_EXEC_SHELLS", "sh,,bash"),
        ("LARRY_EXEC_SHELLS", "-x"),
        ("LARRY_EXEC_BASES", "*"),
    ] {
        let bad = ceilings(&[(name, String::from(value))]);
        assert!(
            matches!(bad, Err(ExecError::BadCeiling { name: refused, .. }) if refused == name),
            "{name}={value} is refused by name"
        );
    }
    Ok(())
}

/// On a virtual clock, every seed's deadline cuts each command's timeout to what is left, says
/// when it cut, and refuses the next command once the reading reaches the deadline.
#[test]
fn sim_the_deadline_cuts_each_command_on_the_injected_clock() -> TestResult {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed.wrapping_add(1 << 32));
        let deadline_ms = rng.below(10_000)?.saturating_add(1);
        let timeout = Duration::from_millis(rng.below(10_000)?.saturating_add(1));
        let clock = Clock::virtual_at(Duration::from_secs(rng.below(1_000)?));
        let limits = Limits {
            timeout,
            deadline_ms,
        };
        let deadline = Deadline::start(&clock, limits);
        let budget = Duration::from_millis(deadline_ms);
        let mut spent = Duration::ZERO;
        while spent < budget {
            let left = budget.saturating_sub(spent);
            let (cut, capped) = deadline.cap(timeout)?;
            assert_eq!(cut, timeout.min(left), "seed {seed}: cut to what is left");
            assert_eq!(capped, left < timeout, "seed {seed}: the cut is reported");
            let step = Duration::from_millis(rng.below(3_000)?.saturating_add(1));
            clock.advance(step)?;
            spent = spent.saturating_add(step);
        }
        assert!(
            matches!(deadline.cap(timeout), Err(ExecError::DeadlinePassed { deadline_ms: named }) if named == deadline_ms),
            "seed {seed}: a spent deadline refuses the next command by its value"
        );
    }
    Ok(())
}

/// A timeout exactly equal to what is left is not a cut, so the command's own timeout ends it;
/// only a command the deadline cut and that then timed out ends the task typed, and a command
/// that ran out its own timeout, or one the deadline cut that finished in time, lets it go on.
#[test]
fn a_command_ends_the_task_only_when_the_deadline_cut_it() -> TestResult {
    let clock = Clock::virtual_at(Duration::from_secs(7));
    let limits = Limits {
        timeout: Duration::from_millis(1_000),
        deadline_ms: 1_000,
    };
    let deadline = Deadline::start(&clock, limits);
    assert_eq!(
        deadline.cap(Duration::from_millis(1_000))?,
        (Duration::from_millis(1_000), false),
        "a timeout equal to what is left is not cut"
    );
    assert_eq!(
        deadline.cap(Duration::from_millis(1_001))?,
        (Duration::from_millis(1_000), true),
        "a timeout one past what is left is cut"
    );
    for (capped, timed_out, ends) in [
        (true, true, true),
        (true, false, false),
        (false, true, false),
        (false, false, false),
    ] {
        let outcome = deadline.after(capped, timed_out);
        assert_eq!(
            matches!(
                outcome,
                Err(ExecError::DeadlinePassed { deadline_ms: 1_000 })
            ),
            ends,
            "capped {capped}, timed out {timed_out}"
        );
        assert_eq!(
            outcome.is_ok(),
            !ends,
            "capped {capped}, timed out {timed_out}"
        );
    }
    Ok(())
}
