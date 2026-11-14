//! The external protocol ingress (E47): a command reaches the ready queue only
//! after its client's offer negotiates a version and the capabilities the
//! command needs, so a refusal mutates neither the queue nor the journal.

use larry_contracts::command::{Command, CommandAcceptance, CommandKind};

use super::DisplayedEffect;
use super::Runtime;
use crate::MAX_READY_WORK;
use crate::error::RuntimeError;
use crate::owner::EffectOwner;
use crate::projection::ProjectionBackend;
use crate::protocol::{MAX_PROTOCOL_BACKOFF_TICKS, ProtocolError, ProtocolOffer, negotiate};

impl<P, O> Runtime<P, O>
where
    P: ProjectionBackend + Default,
    O: EffectOwner,
{
    /// Negotiates protocol version and capabilities before queueing a command.
    ///
    /// This is the external protocol ingress. Local in-process callers may
    /// continue using [`Self::submit`]; protocol callers must present an offer
    /// for every command so a refusal cannot mutate the pending set or journal.
    /// The command, with its displayed effect when present, is durably
    /// admitted before this returns (E28, ADR 0025).
    ///
    /// Returns the negotiated protocol version and how the command was accepted:
    /// a retry under one identity and one content is
    /// [`CommandAcceptance::DuplicateReplay`], which the caller can see instead of
    /// an acknowledgement that hides it (larry#182 defect 3).
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Protocol`] for unsupported versions, absent
    /// capabilities, headless questions, tenant mismatch or overload, and the
    /// admission refusals ([`RuntimeError::Command`],
    /// [`RuntimeError::RunTenantMismatch`], [`RuntimeError::Binding`], the
    /// journal's) from the durable write itself.
    pub fn submit_protocol(
        &mut self,
        offer: &ProtocolOffer,
        command: Command,
        display: Option<DisplayedEffect>,
    ) -> Result<(CommandAcceptance, u16), RuntimeError> {
        let negotiated = negotiate(offer)?;
        if matches!(
            command.content().kind(),
            CommandKind::Ask | CommandKind::Answer
        ) {
            // A question or an answer needs a human at the other end; a
            // headless client is refused, never a silent wait (E30).
            offer.serves_questions()?;
        }
        if negotiated.tenant != command.tenant() {
            return Err(ProtocolError::TenantMismatch {
                negotiated: negotiated.tenant,
                command: command.tenant(),
            }
            .into());
        }
        if self.admitted.len() >= MAX_READY_WORK {
            // A full set needs at least MAX_PROTOCOL_BACKOFF_TICKS ticks to
            // drain (asserted in `protocol`), so the capped delay is exact.
            return Err(ProtocolError::Overloaded {
                capacity: MAX_READY_WORK,
                retry_after_ticks: MAX_PROTOCOL_BACKOFF_TICKS,
            }
            .into());
        }
        // The command is acknowledged only once it is durable (E28): the version
        // returned is the acknowledgement, and the acceptance says whether this
        // request was already recorded (larry#182 defect 3).
        let acceptance = self.admit(command, display)?;
        Ok((acceptance, negotiated.version))
    }
}
