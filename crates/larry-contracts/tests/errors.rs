//! The contract errors keep the messages and causes the derive produced.
//!
//! Source: `std::error::Error`'s contract (Display is the message a person
//! reads, source is the cause a log walks), and thiserror's rule that a
//! `#[from]` field implies `source`. Requirement: the hand-written impls in
//! `error.rs`, `workspace.rs` and `binding.rs` (larry#339, ADR 0087) say
//! exactly what the replaced derive said, so an edit that empties a message
//! or drops a cause is caught here.

use std::error::Error;
use std::str::FromStr;

use larry_contracts::authority::Scope;
use larry_contracts::binding::BindingError;
use larry_contracts::error::{AuthorityError, OrderingError, SequenceError, TransitionError};
use larry_contracts::ids::TenantId;
use larry_contracts::state::RunState;
use larry_contracts::workspace::WorkspaceError;

/// Failures propagate instead of panicking.
type Check = Result<(), Box<dyn std::error::Error>>;

#[test]
fn every_contract_error_names_what_it_refuses() {
    assert_eq!(
        TransitionError::Terminal {
            state: RunState::Completed,
        }
        .to_string(),
        "run state Completed is terminal and immutable; no transition may leave it",
        "a terminal transition names the state"
    );
    assert_eq!(
        SequenceError::Gap {
            expected: 3,
            got: 5,
        }
        .to_string(),
        "sequence gap: expected 3, got 5",
        "a sequence gap names both numbers"
    );
    assert_eq!(
        OrderingError::CausalParentsExceeded { len: 9, max: 8 }.to_string(),
        "9 causal parents exceed the bound of 8",
        "an oversized parent list names it and its bound"
    );
    assert_eq!(
        OrderingError::Sequence(SequenceError::Exhausted).to_string(),
        "event sequence space exhausted at u64::MAX",
        "a transparent ordering error prints its cause"
    );
    assert_eq!(
        AuthorityError::ScopeNotGranted {
            scope: Scope::FileRead,
        }
        .to_string(),
        "scope FileRead is required but not granted",
        "a missing scope is named"
    );
    assert_eq!(
        WorkspaceError::StaleWork.to_string(),
        "work changed; refresh before continuing",
        "a stale workspace names the refresh"
    );
    assert_eq!(
        BindingError::Expired.to_string(),
        "display authority expired; obtain current authority and display again",
        "an expired display says what to do"
    );
}

#[test]
fn an_identifier_parse_error_keeps_its_cause() -> Check {
    let refused = TenantId::from_str("not-a-uuid");
    let error = refused.err().ok_or("a malformed identifier was accepted")?;
    assert!(
        error.to_string().starts_with("malformed identifier text: "),
        "the identifier error names its kind: {error}"
    );
    let cause = error.source().ok_or("the parse error lost its cause")?;
    assert!(
        !cause.to_string().is_empty(),
        "the cause carries the parser's message"
    );
    Ok(())
}
