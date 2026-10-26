//! E65 display and admission on the runtime's real command path.

use std::time::SystemTime;

use lgwks_std::json::serde::Serialize;
use rocco_contracts::authority::{Grant, SecurityEpoch};
use rocco_contracts::binding::{BindingError, BindingField, EffectBinding, MAX_BOUND_FIELD_BYTES};
use rocco_contracts::command::{Command, CommandContent, CommandKind};
use rocco_contracts::ids::{ContentDigest, WorkerId};
use rocco_contracts::replay::{CapabilitySnapshot, PinnedInputs};

use super::Runtime;
use crate::error::RuntimeError;
use crate::owner::EffectOwner;
use crate::payload::WorkProposal;
use crate::projection::ProjectionBackend;

/// A host-created immutable display. Serialization is for rendering only;
/// there is deliberately no deserialization path from model-controlled bytes.
#[derive(Debug, Clone, Serialize)]
#[serde(crate = "lgwks_std::json::serde")]
pub struct DisplayedEffect {
    /// Recomputed complete envelope identity, including the command ID.
    command: ContentDigest,
    /// One independently sufficient grant; never a union of grants.
    grant: Grant,
    /// Runtime security generation at display.
    epoch: SecurityEpoch,
    /// Exact resolved display committed to the intent at admission.
    binding: EffectBinding,
}

impl DisplayedEffect {
    /// What the host must display and retain for the submission.
    #[must_use]
    pub const fn binding(&self) -> &EffectBinding {
        &self.binding
    }

    /// Intersects recorded authorization with this one selected grant.
    pub(super) fn constrain(&self, pinned: &mut PinnedInputs) {
        pinned.capabilities = CapabilitySnapshot::new(
            pinned
                .capabilities
                .capabilities()
                .intersection(self.grant.scopes())
                .copied()
                .collect(),
        );
        pinned.budget.remaining = pinned.budget.remaining.min(self.grant.budget().limit());
        pinned.validity.valid_until = pinned.validity.valid_until.min(self.grant.expiry());
    }
}

/// One bounded queued command with its optional immutable display evidence.
#[derive(Debug)]
pub(super) struct ReadyCommand {
    /// Envelope to admit.
    pub(super) command: Command,
    /// Display evidence, kept outside untrusted proposal bytes.
    pub(super) display: Option<DisplayedEffect>,
}

impl<P, O, C> Runtime<P, O, C>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
    C: crate::clock::Clock,
{
    /// Resolves a command for display under one independently sufficient grant.
    /// The caller is the authenticated host, not a model. This does not submit,
    /// reserve or dispatch. The host displays the returned canonical identity
    /// and supplies the same value to [`Self::submit_displayed`].
    ///
    /// # Errors
    /// Refuses unsupported owners, malformed/oversized commands, non-submit
    /// commands, more than 64 grants, and lack of any single covering grant.
    pub fn display(
        &mut self,
        command: &Command,
        holder: WorkerId,
        grants: &[Grant],
    ) -> Result<DisplayedEffect, RuntimeError> {
        if command.content().kind() != CommandKind::Submit {
            return Err(RuntimeError::UnsupportedCommand(command.content().kind()));
        }
        let digest = command_digest(command)?;
        let proposal: WorkProposal = lgwks_std::json::from_slice(command.content().payload())?;
        self.ensure_run_tenant(proposal.run, command.tenant())?;
        if grants.len() > 64 {
            return Err(BindingError::TooManyGrants.into());
        }
        let at = self.clock.now();
        let grant = grants
            .iter()
            .find(|grant| covers(grant, command, holder, &proposal, at))
            .ok_or(BindingError::NoCoveringGrant)?;
        let intent = proposal.intent(command.tenant(), self.fence.epoch());
        let resolved = self
            .owner
            .resolve(&intent)?
            .ok_or(BindingError::UnsupportedOwner)?;
        let approval = ContentDigest::over(&[
            b"rocco/e65/approval/v1\0",
            digest.as_bytes(),
            &lgwks_std::json::to_vec(grant)?,
            &self.fence.epoch().value().to_be_bytes(),
        ]);
        Ok(DisplayedEffect {
            command: digest,
            grant: grant.clone(),
            epoch: self.fence.epoch(),
            binding: EffectBinding::new(
                resolved,
                approval,
                self.settings.policy.revision,
                grant.expiry().min(self.settings.validity.valid_until),
            ),
        })
    }

    /// Queues the exact display evidence alongside the command, subject to the
    /// existing queue bound. Admission re-resolves rather than trusting it.
    ///
    /// # Errors
    /// Refuses changed command identity and queue saturation, without enqueueing.
    pub fn submit_displayed(
        &mut self,
        command: Command,
        display: DisplayedEffect,
    ) -> Result<(), RuntimeError> {
        if command_digest(&command)? != display.command {
            return Err(BindingError::Changed {
                field: BindingField::Command,
            }
            .into());
        }
        self.enqueue(ReadyCommand {
            command,
            display: Some(display),
        })
    }

    /// Checks target binding before any admission history or budget is changed.
    pub(super) fn validate_display(
        &mut self,
        command: &Command,
        proposal: &WorkProposal,
        display: Option<&DisplayedEffect>,
        at: SystemTime,
    ) -> Result<(), RuntimeError> {
        let intent = proposal.intent(command.tenant(), self.fence.epoch());
        if let Some(shown) = display {
            if command_digest(command)? != shown.command {
                return Err(BindingError::Changed {
                    field: BindingField::Command,
                }
                .into());
            }
            if shown.epoch != self.fence.epoch() {
                return Err(BindingError::Changed {
                    field: BindingField::SecurityEpoch,
                }
                .into());
            }
            shown
                .binding
                .verify_runtime(self.settings.policy.revision, at)?;
            let current = self
                .owner
                .resolve(&intent)?
                .ok_or(BindingError::UnsupportedOwner)?;
            shown.binding.resolved().verify(&current)?;
        } else if self.owner.resolve(&intent)?.is_some() {
            return Err(BindingError::MissingDisplay.into());
        }
        Ok(())
    }

    /// Resolves at every mutating reach, including retries after restoration.
    pub(super) fn validate_bound_reach(
        &mut self,
        intent: &rocco_contracts::effect::EffectIntent,
        at: SystemTime,
    ) -> Result<(), RuntimeError> {
        match intent.binding() {
            Some(binding) => {
                binding.verify_runtime(self.settings.policy.revision, at)?;
                let current = self
                    .owner
                    .resolve(intent)?
                    .ok_or(BindingError::UnsupportedOwner)?;
                binding.resolved().verify(&current)?;
            }
            None => {
                if self.owner.resolve(intent)?.is_some() {
                    return Err(BindingError::MissingDisplay.into());
                }
            }
        }
        Ok(())
    }
}

/// Each candidate stands alone: no scopes, budgets or expiries are stitched.
fn covers(
    grant: &Grant,
    command: &Command,
    holder: WorkerId,
    proposal: &WorkProposal,
    at: SystemTime,
) -> bool {
    grant.tenant() == command.tenant()
        && grant.holder() == holder
        && proposal.required_scopes().is_subset(grant.scopes())
        && grant.budget().unit() == proposal.unit
        && proposal.cost <= grant.budget().limit()
        && at < grant.expiry()
}

/// Recomputes identity from bytes rather than trusting deserialized digest fields.
fn command_digest(command: &Command) -> Result<ContentDigest, RuntimeError> {
    if command.content().payload().len() > MAX_BOUND_FIELD_BYTES {
        return Err(BindingError::TooLarge {
            field: BindingField::Command,
        }
        .into());
    }
    let canonical = Command::new(
        command.id(),
        command.tenant(),
        command.causal_parents().to_vec(),
        CommandContent::new(command.content().kind(), command.content().payload().into()),
    )?;
    Ok(ContentDigest::over(&[
        b"rocco/e65/command/v1\0",
        canonical.id().as_bytes(),
        canonical.envelope_digest().as_bytes(),
    ]))
}
