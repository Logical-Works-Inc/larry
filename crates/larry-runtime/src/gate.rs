//! The authorization gate: the pure decision and the proof that outlives it.
//!
//! D36 makes authorization a pure function of the declared effect, the policy
//! snapshot, the capability state and the reserved budget snapshot, and E55
//! requires that replaying the recorded log re-derive bit-identical
//! decisions. That is [`authorize`]: it calls the contract reducer and
//! returns the [`ReplayRecord`] that must be persisted with the intent. The
//! record is what makes the decision replayable later — a decision recorded
//! without its inputs is not replayable, only assertable.
//!
//! A decision is taken once, but the effect is reached later. [`Proof`] is the
//! estate's `cap::Auth` issued for the pinned validity lease (larry#77): every
//! mutating reach presents it, and it lapses on the runtime clock at
//! `valid_until`, so a grant that held at admission cannot still be relied on
//! after its lease ends. Passing the gate is not a licence to dispatch later.

use std::time::{Duration, SystemTime};

use larry_contracts::authority::Scope;
use larry_contracts::replay::{Decision, DenialReason, PinnedInputs, ReplayRecord, capability};
use lgwks_bot::BotError;
use lgwks_bot::cap::{Auth, Cap};
use lgwks_bot::clock::Clock;
use lgwks_bot::gate::GrantSet;

use crate::error::RuntimeError;

/// Decides an effect under pinned inputs and returns the replayable record.
///
/// # Errors
///
/// Returns [`RuntimeError::Denied`] carrying the deterministic first-failing
/// [`DenialReason`](larry_contracts::replay::DenialReason). The check order
/// is fixed by the contract reducer (validity, policy, capability, budget),
/// so the recorded reason is itself reproducible.
pub fn authorize(pinned: &PinnedInputs) -> Result<ReplayRecord, RuntimeError> {
    let recorded = pinned.decide();
    match recorded {
        Decision::Authorized => Ok(ReplayRecord::new(pinned.clone(), recorded)),
        Decision::Denied { reason } => Err(RuntimeError::Denied {
            operation: pinned.request.operation,
            reason,
        }),
        other => Err(RuntimeError::UnsupportedDecision(other)),
    }
}

/// The estate authority proof a mutating reach presents (larry#77).
///
/// `auth` is the `cap::Auth` minted for the pinned validity lease, or the
/// typed reason none could be minted: no recorded authorization, or a grant
/// that never covered a required scope. `required` lists each request scope
/// with its capability, so a refusal can name the scope.
#[derive(Debug, Clone)]
pub(crate) struct Proof {
    /// The lease proof, or why the gate could not mint one.
    auth: Result<Auth, DenialReason>,
    /// The request's scopes and the capability each one exercises.
    required: Vec<(Scope, Cap)>,
}

impl Proof {
    /// The proof for `pinned`, leased from `clock`'s reading now to its validity end.
    ///
    /// Only capabilities the policy also allows are granted, so the proof carries
    /// what authorization permitted, never what the worker merely holds. The lease
    /// is measured from the clock's own reading rather than the tick's pinned
    /// instant: on a live clock the tick has already aged, and a lease counted from
    /// its start would outlive `valid_until` by that age. A validity end at or
    /// before now, or a reading `SystemTime` cannot hold, leaves a zero lease,
    /// which lapses at once.
    pub(crate) fn issue(pinned: &PinnedInputs, clock: &Clock) -> Self {
        let required: Vec<(Scope, Cap)> = pinned
            .request
            .required_scopes()
            .iter()
            .map(|&scope| (scope, capability(scope)))
            .collect();
        let ttl = match SystemTime::UNIX_EPOCH
            .checked_add(clock.now())
            .map(|now| pinned.validity.valid_until.duration_since(now))
        {
            Some(Ok(remaining)) => remaining,
            Some(Err(_)) | None => Duration::ZERO,
        };
        let grants = pinned
            .capabilities
            .capabilities()
            .iter()
            .filter(|scope| pinned.policy.allowed_scopes().contains(*scope))
            .fold(GrantSet::empty(), |grants, &scope| {
                grants.grant_expiring(capability(scope), ttl)
            });
        let caps: Vec<Cap> = required.iter().map(|pair| pair.1.clone()).collect();
        let auth = grants
            .issue_at(&caps, clock)
            .map_err(|refusal| denial(&required, refusal));
        Self { auth, required }
    }

    /// The proof of an effect with no recorded authorization: it holds nothing.
    pub(crate) const fn lapsed() -> Self {
        Self {
            auth: Err(DenialReason::AuthorityExpired),
            required: Vec::new(),
        }
    }

    /// Whether the lease still covers every required capability.
    ///
    /// # Errors
    ///
    /// [`DenialReason::AuthorityExpired`] when the lease has ended or was never
    /// issued, and [`DenialReason::CapabilityMissing`] naming the first required
    /// scope the proof does not cover.
    pub(crate) fn check(&self) -> Result<(), DenialReason> {
        let auth = self.auth.as_ref().map_err(|reason| *reason)?;
        let caps: Vec<Cap> = self.required.iter().map(|pair| pair.1.clone()).collect();
        auth.check(&caps)
            .map_err(|refusal| denial(&self.required, refusal))
    }
}

/// The recorded denial an estate refusal stands for: a deficit names the first
/// required scope it lacks, and every other refusal, an expired credential among
/// them, is `AuthorityExpired`, failing closed.
fn denial(required: &[(Scope, Cap)], refusal: BotError) -> DenialReason {
    let BotError::CapabilityDenied { deficit } = refusal else {
        return DenialReason::AuthorityExpired;
    };
    let missing = deficit.first().required();
    match required.iter().find(|pair| pair.1 == *missing) {
        Some(pair) => DenialReason::CapabilityMissing { scope: pair.0 },
        None => DenialReason::AuthorityExpired,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use larry_contracts::authority::{BudgetUnit, Scope};
    use larry_contracts::ids::OperationId;
    use larry_contracts::replay::{
        BudgetSnapshot, CapabilitySnapshot, DenialReason, EffectRequest, PinnedInputs,
        PolicySnapshot, ValiditySnapshot,
    };
    use lgwks_bot::clock::Clock;

    use super::Proof;

    type Outcome = Result<(), Box<dyn std::error::Error>>;

    /// The instant `secs` after the Unix epoch, refused rather than overflowed.
    fn at(secs: u64) -> Result<SystemTime, Box<dyn std::error::Error>> {
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(secs))
            .ok_or_else(|| "fixture time overflow".into())
    }

    /// The pinned inputs for one `FileRead` effect observed at 100 s, leased to
    /// `valid_until`, with the given worker capabilities under an open policy.
    fn pinned(
        capable: &[Scope],
        valid_until: SystemTime,
    ) -> Result<PinnedInputs, Box<dyn std::error::Error>> {
        Ok(PinnedInputs::new(
            EffectRequest::new(
                OperationId::from_bytes([1; 16]),
                [Scope::FileRead].into_iter().collect(),
                1,
                BudgetUnit::Requests,
                at(100)?,
            ),
            PolicySnapshot::new(1, [Scope::FileRead].into_iter().collect()),
            BudgetSnapshot::new(10, BudgetUnit::Requests),
            CapabilitySnapshot::new(capable.iter().copied().collect()),
            ValiditySnapshot::new(valid_until),
        ))
    }

    /// The clock placed at `secs` after the Unix epoch.
    fn clock_at(secs: u64) -> Result<Clock, Box<dyn std::error::Error>> {
        Ok(Clock::virtual_at(
            at(secs)?.duration_since(SystemTime::UNIX_EPOCH)?,
        ))
    }

    /// A lease that ended at the clock's reading is refused as expired (E30), a
    /// proof missing the required scope names it, and an unissued proof holds nothing.
    #[test]
    fn a_boundary_refusal_names_its_typed_reason() -> Outcome {
        assert_eq!(Proof::lapsed().check(), Err(DenialReason::AuthorityExpired));
        let ended = Proof::issue(&pinned(&[Scope::FileRead], at(200)?)?, &clock_at(200)?);
        assert_eq!(ended.check(), Err(DenialReason::AuthorityExpired));
        let past = Proof::issue(&pinned(&[Scope::FileRead], at(200)?)?, &clock_at(500)?);
        assert_eq!(past.check(), Err(DenialReason::AuthorityExpired));
        let uncovered = Proof::issue(&pinned(&[], at(200)?)?, &clock_at(100)?);
        assert_eq!(
            uncovered.check(),
            Err(DenialReason::CapabilityMissing {
                scope: Scope::FileRead
            })
        );
        Ok(())
    }

    /// A proof reissued on restore carries only the lease that remains: issued ten
    /// seconds before `valid_until`, it holds for nine and lapses at the tenth.
    #[test]
    fn a_reissued_proof_carries_only_the_remaining_lease() -> Outcome {
        let clock = clock_at(190)?;
        let reissued = Proof::issue(&pinned(&[Scope::FileRead], at(200)?)?, &clock);
        assert_eq!(reissued.check(), Ok(()));
        clock.advance(Duration::from_secs(9))?;
        assert_eq!(reissued.check(), Ok(()), "one second of lease is left");
        clock.advance(Duration::from_secs(1))?;
        assert_eq!(reissued.check(), Err(DenialReason::AuthorityExpired));
        Ok(())
    }
}
