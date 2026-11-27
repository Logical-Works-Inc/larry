//! Seeded coverage sims for the display boundary, the catch-up consumer and
//! the owner trait defaults.
//!
//! A display binds the exact command a human approved and nothing else; the
//! consumer pages every fact within its byte budget; the trait defaults
//! refuse without performing. Sources: E65 (display binding), E35/D25
//! (bounded catch-up), D21/D43 (the owner boundary), larry#68 (test doubles
//! never ship) and CWE-20, CWE-639 (forged binding).

mod support;

#[path = "support/atomic.rs"]
mod atomic;

use std::time::{Duration, SystemTime};

use atomic::AtomicOwner;
use larry_contracts::authority::{Budget, BudgetUnit, Grant, Scope, SecurityEpoch};
use larry_contracts::binding::{
    BindingError, BindingField, EffectTarget, MAX_BOUND_FIELD_BYTES, ResolvedEffect,
};
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectIntent, EffectOutcome, ReconciliationStrategy};
use larry_contracts::event::EventSequence;
use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId, WorkerId};
use larry_runtime::consumer::{
    ConsumerCursor, ConsumerLimits, ConsumerPage, ConsumerRecord, CursorError,
};
use larry_runtime::error::RuntimeError;
use larry_runtime::journal::JournalEntry;
use larry_runtime::owner::EffectOwner;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::projection::FoldProjection;
use larry_runtime::runtime::Runtime;
use lgwks_bot::kernel::cap::Cap;
use support::{
    clock, command_id, epoch, operation, proposal, run, runtime, settings, submit, tenant,
};

/// Fallible test outcome; setup failures propagate instead of panicking.
type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Calls a tiny `const` constructor or getter through an opaque function
/// pointer so its body executes out of line in its own crate.
fn outlined<F>(func: F) -> F {
    std::hint::black_box(func)
}

/// One independently covering host grant for the display tests.
fn display_grant(
    tenant: TenantId,
    holder: WorkerId,
    expiry: SystemTime,
) -> Result<Grant, Box<dyn std::error::Error>> {
    Ok(Grant::new(
        tenant,
        holder,
        [Scope::FileRead].into_iter().collect(),
        Budget::new(100, BudgetUnit::Requests),
        expiry,
        0,
    )?)
}

/// A submitted `FileRead` command for the display tests.
fn display_command(number: u8, tenant: TenantId) -> Result<Command, Box<dyn std::error::Error>> {
    let first = tenant.as_bytes()[0].wrapping_add(number);
    let bytes = [first; 16];
    let body = larry_runtime::payload::WorkProposal::new(
        RunId::from_bytes(bytes),
        tenant,
        OperationId::from_bytes(bytes),
        [Scope::FileRead].into_iter().collect(),
        3,
        BudgetUnit::Requests,
        ReconciliationStrategy::Queryable,
    );
    Ok(Command::new(
        CommandId::from_bytes(bytes),
        tenant,
        Vec::new(),
        CommandContent::new(
            CommandKind::Submit,
            lgwks_std::json::to_vec(&body)?.into_boxed_slice(),
        ),
    )?)
}

// Source: D25 (bounded catch-up) and CWE-20 (a hostile cursor). The page and
// cursor getters read back exactly, causal parents bind into the digest, an
// entry past the byte budget is refused without advancing, and every
// out-of-range bound is refused.
#[test]
fn sim_consumer_page_getters_and_causal_parents() -> Outcome {
    use larry_contracts::event::CausalRef;
    let tenant = tenant(3);
    let mut runtime = runtime()?;
    let parent = CausalRef::new(run(3), EventSequence::FIRST);
    for index in [30_u8, 31] {
        let parents = if index == 31 {
            vec![parent]
        } else {
            Vec::new()
        };
        let body = proposal(
            run(3),
            tenant,
            operation(index),
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        );
        let payload = lgwks_std::json::to_vec(&body)?.into_boxed_slice();
        runtime.submit(Command::new(
            CommandId::from_bytes([index; 16]),
            tenant,
            parents,
            CommandContent::new(CommandKind::Steer, payload),
        )?)?;
    }
    runtime.tick()?;
    let mut consumer = runtime.subscribe(
        tenant,
        None,
        ConsumerLimits::new(100, 2_097_152, 256, 8_388_608)?,
    )?;
    {
        let page = runtime.consumer_page(tenant, &mut consumer)?;
        assert_eq!(page.records().len(), 4, "every tenant fact is paged");
        let first = page.records().first().ok_or("the page holds the head")?;
        let sequence_of = outlined(ConsumerRecord::sequence);
        assert_eq!(
            sequence_of(first),
            1,
            "sequences are one-based journal positions"
        );
        let entry_of = outlined(ConsumerRecord::entry);
        assert!(matches!(entry_of(first), JournalEntry::Command(_)));
        let bytes_of = outlined(ConsumerPage::decoded_bytes);
        assert!(bytes_of(&page) > 0);
        let caught_up = outlined(ConsumerPage::caught_up);
        assert!(caught_up(&page), "four facts end at the head");
        let cursor_of: fn(&ConsumerCursor) -> usize = outlined(ConsumerCursor::sequence);
        assert_eq!(cursor_of(&consumer.cursor()), 4);
    }

    // A hostile byte budget smaller than one entry refuses that entry.
    let mut tight =
        runtime.subscribe(tenant, None, ConsumerLimits::new(100, 8, 256, 8_388_608)?)?;
    let refused = runtime.consumer_page(tenant, &mut tight);
    assert!(
        matches!(refused, Err(CursorError::EntryTooLarge { .. })),
        "an indivisible entry is refused: {refused:?}"
    );

    // Zero and over-ceiling bounds are refused before any read.
    for limits in [
        ConsumerLimits::new(0, 2_097_152, 256, 8_388_608),
        ConsumerLimits::new(101, 2_097_152, 256, 8_388_608),
        ConsumerLimits::new(100, 0, 256, 8_388_608),
        ConsumerLimits::new(100, 2_097_153, 256, 8_388_608),
        ConsumerLimits::new(100, 2_097_152, 0, 8_388_608),
        ConsumerLimits::new(100, 2_097_152, 257, 8_388_608),
        ConsumerLimits::new(100, 2_097_152, 256, 0),
        ConsumerLimits::new(100, 2_097_152, 256, 8_388_609),
        ConsumerLimits::new(100, 2_097_152, 256, 7),
    ] {
        assert!(
            matches!(limits, Err(CursorError::InvalidBounds)),
            "an unusable bound is refused: {limits:?}"
        );
    }
    Ok(())
}

// Source: E65 (the display binds the exact command a human approved) and
// CWE-639 (a display approved for one identity must not authorize another).
// A non-submit display, an oversized command and an unserved grant kind are
// each refused with the exact typed error, and the owner is never reached
// for a refused display.
#[test]
fn sim_display_boundary_refusals_name_their_cause() -> Outcome {
    let tenant = tenant(4);
    let holder = WorkerId::from_bytes([0xE5; 16]);
    let expiry = epoch()?
        .checked_add(Duration::from_secs(3_600))
        .ok_or("time")?;

    // A display is only ever made for a submission (E65).
    let mut runtime = runtime()?;
    let cancel_body = proposal(
        run(4),
        tenant,
        operation(4),
        &[Scope::FileRead],
        0,
        ReconciliationStrategy::Queryable,
    );
    let cancel = Command::new(
        command_id(40),
        tenant,
        Vec::new(),
        CommandContent::new(
            CommandKind::Cancel,
            lgwks_std::json::to_vec(&cancel_body)?.into_boxed_slice(),
        ),
    )?;
    assert!(
        matches!(
            runtime.display(&cancel, holder, &[]),
            Err(RuntimeError::UnsupportedCommand(CommandKind::Cancel))
        ),
        "a non-submit display is refused"
    );

    // A command past the bound field ceiling is refused before any decode.
    let huge = vec![b'x'; MAX_BOUND_FIELD_BYTES + 1];
    let oversized = Command::new(
        command_id(41),
        tenant,
        Vec::new(),
        CommandContent::new(CommandKind::Submit, huge.into_boxed_slice()),
    )?;
    assert!(
        matches!(
            runtime.display(&oversized, holder, &[]),
            Err(RuntimeError::Binding(BindingError::TooLarge { .. }))
        ),
        "an oversized command is refused by name"
    );

    // A grant of an unserved kind covers nothing on its own.
    let submit_cmd = display_command(1, tenant)?;
    let kinded = Grant::new(
        tenant,
        holder,
        [Scope::ModelInvoke].into_iter().collect(),
        Budget::new(100, BudgetUnit::Requests),
        expiry,
        0,
    )?;
    assert!(
        matches!(
            runtime.display(&submit_cmd, holder, std::slice::from_ref(&kinded)),
            Err(RuntimeError::Binding(BindingError::NoCoveringGrant))
        ),
        "an unserved grant kind covers nothing"
    );
    Ok(())
}

// Source: E65 (the submitted command must equal the displayed one) and
// CWE-639. A single appended byte is a changed command, never the approved
// one: the submission is refused naming the command field.
#[test]
fn sim_tampered_command_is_not_the_displayed_one() -> Outcome {
    let tenant = tenant(4);
    let expiry = epoch()?
        .checked_add(Duration::from_secs(3_600))
        .ok_or("time")?;
    let submit_cmd = display_command(1, tenant)?;
    let mut resolving: Runtime<FoldProjection, AtomicOwner> = Runtime::new(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        AtomicOwner::new(7, 0),
        clock()?,
    )?;
    let grant = display_grant(tenant, atomic::holder(), expiry)?;
    let shown = resolving.display(&submit_cmd, atomic::holder(), &[grant])?;
    let tampered = Command::new(
        submit_cmd.id(),
        submit_cmd.tenant(),
        submit_cmd.causal_parents().to_vec(),
        CommandContent::new(
            submit_cmd.content().kind(),
            [submit_cmd.content().payload(), &b"x"[..]]
                .concat()
                .into_boxed_slice(),
        ),
    )?;
    assert!(
        matches!(
            resolving.submit_displayed(tampered, shown),
            Err(RuntimeError::Binding(BindingError::Changed {
                field: BindingField::Command
            }))
        ),
        "a tampered command is not the displayed one"
    );
    Ok(())
}

// Source: E65 (no display, no authority). A reach with no display against a
// resolving owner, and a target that appears only at the reach, are both
// refused as missing display: the host never approved either effect.
#[test]
fn sim_unbound_reaches_are_refused_missing_display() -> Outcome {
    let tenant = tenant(4);
    // An unbound reach against a resolving owner is refused: a display the
    // host never made cannot become authority at the boundary.
    let mut unbound: Runtime<FoldProjection, AtomicOwner> = Runtime::new(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        AtomicOwner::new(9, 0),
        clock()?,
    )?;
    let body = proposal(
        run(4),
        tenant,
        operation(5),
        &[Scope::FileRead],
        1,
        ReconciliationStrategy::Queryable,
    );
    unbound.submit(submit(command_id(43), &body)?)?;
    let refused = unbound.tick()?;
    assert!(
        refused
            .refused()
            .iter()
            .any(|entry| matches!(entry.1, RuntimeError::Binding(BindingError::MissingDisplay))),
        "an unbound reach is refused: {:?}",
        refused.refused()
    );

    // A target that appears only at the reach is refused too: admission saw
    // no target, so the reach that found one has no display to check it
    // against, and the effect never runs.
    struct FlipOwner {
        calls: usize,
        target: EffectTarget,
    }
    impl EffectOwner for FlipOwner {
        fn resolve(
            &mut self,
            _intent: &EffectIntent,
        ) -> Result<Option<ResolvedEffect>, BindingError> {
            self.calls += 1;
            if self.calls == 1 {
                Ok(None)
            } else {
                Ok(Some(ResolvedEffect::new(
                    self.target,
                    1,
                    SecurityEpoch::INITIAL,
                )))
            }
        }
        fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
            EffectOutcome::Unknown {
                operation: intent.operation,
            }
        }
    }
    let mut flip: Runtime<FoldProjection, FlipOwner> = Runtime::new(
        settings(&[Scope::FileRead], &[Scope::FileRead], 100)?,
        FlipOwner {
            calls: 0,
            target: EffectTarget::new(b"a", b"b", b"c", b"d", b"e", b"f")?,
        },
        clock()?,
    )?;
    flip.submit(submit(command_id(44), &body)?)?;
    let late = flip.tick()?;
    assert!(
        late.refused()
            .iter()
            .any(|entry| matches!(entry.1, RuntimeError::Binding(BindingError::MissingDisplay))),
        "a target that appears only at the reach is refused: {:?}",
        late.refused()
    );
    Ok(())
}

// Source: D21/D43 (the owner boundary) and larry#68 (test doubles never
// ship). The trait's defaults refuse without performing: an unbound reach
// and a bare resolve are `UnsupportedOwner`, never a fabricated success,
// and the scripted double's default answers unknown, never success.
#[test]
fn sim_owner_defaults_refuse_and_report_nothing() -> Outcome {
    struct BareOwner;
    impl EffectOwner for BareOwner {
        fn reconcile(&mut self, intent: &EffectIntent, _at: SystemTime) -> EffectOutcome {
            EffectOutcome::Unknown {
                operation: intent.operation,
            }
        }
    }
    let intent = EffectIntent::new(
        larry_contracts::ids::EffectId::from_bytes([1; 16]),
        operation(6),
        run(6),
        tenant(6),
        ReconciliationStrategy::Queryable,
        SecurityEpoch::INITIAL,
    );
    let mut bare = BareOwner;
    assert_eq!(
        bare.resolve(&intent),
        Err(larry_contracts::binding::BindingError::UnsupportedOwner),
        "a bare resolve performs nothing"
    );
    let proof = lgwks_bot::kernel::gate::GrantSet::empty().issue(&[])?;
    assert_eq!(
        bare.dispatch(&intent, &proof, &[], SystemTime::UNIX_EPOCH),
        Err(larry_contracts::binding::BindingError::UnsupportedOwner),
        "a bare dispatch performs nothing"
    );
    assert_eq!(
        bare.deduplication_window(&intent),
        None,
        "a bare owner promises no deduplication"
    );
    // The scripted double with no script answers unknown, never success, and
    // records the reach it took to answer it.
    let mut scripted = ScriptedOwner::default();
    let unknown = scripted.dispatch(&intent, &proof, &[], SystemTime::UNIX_EPOCH)?;
    assert!(
        matches!(unknown, EffectOutcome::Unknown { .. }),
        "an untold fixture invents no success"
    );
    assert_eq!(
        scripted.dispatched(),
        &[operation(6)],
        "the reach is recorded even when the answer is unknown"
    );
    // A lapsed proof is refused before any send, naming expiry.
    let lapsed = lgwks_bot::kernel::gate::GrantSet::empty()
        .grant_expiring(Cap::new("larry.test"), Duration::ZERO)
        .issue_at(
            &[Cap::new("larry.test")],
            &lgwks_bot::kernel::clock::Clock::virtual_at(Duration::ZERO),
        )?;
    assert_eq!(
        scripted.dispatch(&intent, &lapsed, &[], SystemTime::UNIX_EPOCH),
        Err(larry_contracts::binding::BindingError::Expired),
        "a lapsed proof is refused before the send"
    );
    Ok(())
}
