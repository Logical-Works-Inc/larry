//! Version and capability negotiation at the runtime ingress.

use std::collections::BTreeSet;
use std::fmt;

use larry_contracts::ids::TenantId;

/// Minimum protocol version accepted by this runtime.
pub const MIN_PROTOCOL_VERSION: u16 = 1;
/// Maximum protocol version accepted by this runtime.
pub const MAX_PROTOCOL_VERSION: u16 = 1;
/// Maximum deterministic overload delay, measured in logical ticks.
pub(crate) const MAX_PROTOCOL_BACKOFF_TICKS: u32 = 4;

/// A full ready queue drains `MAX_RUNS_PER_TICK` commands a tick, so it needs at
/// least `MAX_PROTOCOL_BACKOFF_TICKS` ticks: the overload delay is always the cap.
const _: () = assert!(
    crate::MAX_READY_WORK >= crate::MAX_RUNS_PER_TICK.saturating_mul(4)
        && MAX_PROTOCOL_BACKOFF_TICKS == 4,
    "a full ready queue takes at least the capped backoff to drain"
);

/// Capabilities a client may offer to the runtime protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ProtocolCapability {
    /// Command submission.
    SubmitCommand,
    /// Human response to a question.
    HumanQuestion,
}

/// Inclusive protocol-version interval offered by a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolVersionRange(u16, u16);

impl ProtocolVersionRange {
    /// Creates an inclusive version interval; validity is checked at negotiation.
    #[must_use]
    pub const fn new(minimum: u16, maximum: u16) -> Self {
        Self(minimum, maximum)
    }
}

/// Tenant-scoped protocol range and request requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolOffer(
    TenantId,
    ProtocolVersionRange,
    BTreeSet<ProtocolCapability>,
    bool,
    bool,
);

impl ProtocolOffer {
    /// Creates a protocol offer for one tenant.
    #[must_use]
    pub fn new(
        tenant_id: TenantId,
        version_range: ProtocolVersionRange,
        offered_capabilities: BTreeSet<ProtocolCapability>,
        is_headless: bool,
        needs_human_answer: bool,
    ) -> Self {
        Self(
            tenant_id,
            version_range,
            offered_capabilities,
            is_headless,
            needs_human_answer,
        )
    }
}

impl ProtocolOffer {
    /// Whether the offering client can put a question before a human and
    /// carry the answer back: it offers `HumanQuestion` and is not headless
    /// (E30, E47).
    ///
    /// # Errors
    ///
    /// [`ProtocolError::CapabilityMissing`] without `HumanQuestion`, then
    /// [`ProtocolError::HeadlessQuestion`] for a headless client.
    pub(crate) fn serves_questions(&self) -> Result<(), ProtocolError> {
        let &Self(_, _, ref capabilities, headless, _) = self;
        if !capabilities.contains(&ProtocolCapability::HumanQuestion) {
            return Err(ProtocolError::CapabilityMissing {
                capability: ProtocolCapability::HumanQuestion,
            });
        }
        if headless {
            return Err(ProtocolError::HeadlessQuestion);
        }
        Ok(())
    }
}

/// Negotiated version and tenant identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NegotiatedProtocol {
    /// Tenant bound to this negotiation.
    pub(crate) tenant: TenantId,
    /// Highest mutually supported protocol version.
    pub(crate) version: u16,
}

/// Typed refusals returned by negotiation and protocol submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolError {
    /// The offered range is reversed.
    InvalidVersionRange {
        /// Inclusive lower bound supplied by the client.
        min: u16,
        /// Inclusive upper bound supplied by the client.
        max: u16,
    },
    /// No offered version is supported by this runtime.
    UnsupportedVersion {
        /// Lowest version offered by the client.
        offered_min: u16,
        /// Highest version offered by the client.
        offered_max: u16,
        /// Lowest version accepted by the runtime.
        supported_min: u16,
        /// Highest version accepted by the runtime.
        supported_max: u16,
    },
    /// A required capability is absent.
    CapabilityMissing {
        /// Required capability not present in the offer.
        capability: ProtocolCapability,
    },
    /// A human question cannot be handled by a headless client.
    HeadlessQuestion,
    /// The offer and command belong to different tenants.
    TenantMismatch {
        /// Tenant named by the offer.
        negotiated: TenantId,
        /// Tenant named by the command.
        command: TenantId,
    },
    /// The bounded ready queue is full.
    Overloaded {
        /// Maximum queued command count.
        capacity: usize,
        /// Deterministic retry delay in logical ticks.
        retry_after_ticks: u32,
    },
}

/// Formats an inclusive protocol version interval.
fn display_range(formatter: &mut fmt::Formatter<'_>, min: u16, max: u16) -> fmt::Result {
    write!(formatter, "{min}..={max}")
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidVersionRange { min, max } => {
                formatter.write_str("invalid protocol version range ")?;
                display_range(formatter, min, max)?;
                formatter.write_str("; set the minimum no greater than the maximum")
            }
            Self::UnsupportedVersion {
                offered_min,
                offered_max,
                supported_min,
                supported_max,
            } => {
                formatter.write_str("unsupported protocol version range ")?;
                display_range(formatter, offered_min, offered_max)?;
                formatter.write_str("; supported range is ")?;
                display_range(formatter, supported_min, supported_max)
            }
            Self::CapabilityMissing { capability } => write!(
                formatter,
                "required protocol capability {capability:?} is absent; enable it or remove the dependent request"
            ),
            Self::HeadlessQuestion => formatter.write_str(
                "human question cannot be served headlessly; attach an authorized response surface or remove the question requirement",
            ),
            Self::TenantMismatch {
                negotiated,
                command,
            } => write!(
                formatter,
                "protocol offer is scoped to tenant {negotiated}, but command belongs to tenant {command}"
            ),
            Self::Overloaded {
                capacity,
                retry_after_ticks,
            } => write!(
                formatter,
                "command queue is full at {capacity}; retry after {retry_after_ticks} logical ticks"
            ),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// Selects the highest overlapping protocol version without mutating state.
///
/// Crate-internal: external callers negotiate through
/// [`Runtime::submit_protocol`](crate::runtime::Runtime::submit_protocol),
/// which also applies the tenant and overload checks a bare negotiation
/// cannot.
///
/// # Errors
/// Returns typed errors for unsupported bounds, missing capabilities, and a
/// human question that cannot be answered headlessly.
pub(crate) fn negotiate(offer: &ProtocolOffer) -> Result<NegotiatedProtocol, ProtocolError> {
    let &ProtocolOffer(tenant_id, ref range, ref capabilities, headless, needs_answer) = offer;
    let &ProtocolVersionRange(offered_min, offered_max) = range;
    let lower = offered_min.max(MIN_PROTOCOL_VERSION);
    let upper = offered_max.min(MAX_PROTOCOL_VERSION);
    if offered_min > offered_max {
        return Err(ProtocolError::InvalidVersionRange {
            min: offered_min,
            max: offered_max,
        });
    }
    if lower > upper {
        return Err(ProtocolError::UnsupportedVersion {
            offered_min,
            offered_max,
            supported_min: MIN_PROTOCOL_VERSION,
            supported_max: MAX_PROTOCOL_VERSION,
        });
    }
    if !capabilities.contains(&ProtocolCapability::SubmitCommand) {
        return Err(ProtocolError::CapabilityMissing {
            capability: ProtocolCapability::SubmitCommand,
        });
    }
    if needs_answer && !capabilities.contains(&ProtocolCapability::HumanQuestion) {
        return Err(ProtocolError::CapabilityMissing {
            capability: ProtocolCapability::HumanQuestion,
        });
    }
    if needs_answer && headless {
        return Err(ProtocolError::HeadlessQuestion);
    }
    Ok(NegotiatedProtocol {
        tenant: tenant_id,
        version: upper,
    })
}
