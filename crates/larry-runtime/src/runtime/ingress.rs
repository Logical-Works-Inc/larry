//! The external protocol ingress (E47): a command reaches the ready queue only
//! after its client's offer negotiates a version and the capabilities the
//! command needs, so a refusal mutates neither the queue nor the journal.

use larry_contracts::command::Command;

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
    /// for every command so a refusal cannot mutate the queue or journal. A
    /// displayed effect, when present, rides the same command through
    /// [`Self::submit_displayed`], which retains its identity check at the
    /// queue; without one the command enqueues display-less exactly as
    /// [`Self::submit`] does.
    ///
    /// Returns the negotiated protocol version.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Protocol`] for unsupported versions, absent
    /// capabilities, headless questions, tenant mismatch or queue overload,
    /// and the shared enqueue refusals ([`RuntimeError::ReadyWorkFull`],
    /// [`RuntimeError::Binding`]) from the queueing step itself.
    pub fn submit_protocol(
        &mut self,
        offer: &ProtocolOffer,
        command: Command,
        display: Option<DisplayedEffect>,
    ) -> Result<u16, RuntimeError> {
        let negotiated = negotiate(offer)?;
        if negotiated.tenant != command.tenant() {
            return Err(ProtocolError::TenantMismatch {
                negotiated: negotiated.tenant,
                command: command.tenant(),
            }
            .into());
        }
        if self.ready.len() >= MAX_READY_WORK {
            // A full queue needs at least MAX_PROTOCOL_BACKOFF_TICKS ticks to
            // drain (asserted in `protocol`), so the capped delay is exact.
            return Err(ProtocolError::Overloaded {
                capacity: MAX_READY_WORK,
                retry_after_ticks: MAX_PROTOCOL_BACKOFF_TICKS,
            }
            .into());
        }
        let queued = match display {
            Some(shown) => self.submit_displayed(command, shown),
            None => self.submit(command),
        };
        queued.map(|()| negotiated.version)
    }
}
