//! E71 authority ownership: historical inputs cannot authenticate themselves.
//!
//! Implementations are trusted owner adapters, never model-selected plugins.
//! The only in-tree implementation is the deterministic fixture below. This
//! seam does not implement a cryptographic protocol or contact a sibling.

use rocco_contracts::authority::SecurityEpoch;
use rocco_contracts::effect::EffectIntent;
use rocco_contracts::replay::PinnedInputs;

use crate::{FenceRefusal, RevocationFence};

/// The authenticated authority owner for one security-epoch domain.
///
/// A source must authenticate tenant scope and the exact pinned snapshots,
/// retain monotonic epochs and withdrawals, and bound disconnected authority.
/// Returning a fence is evidence from this trusted owner, not from settings,
/// commands or journal replay. All tenants served by a source share its epoch
/// domain and total offline exposure ceiling.
pub trait AuthoritySource {
    /// Starts a new runtime session. Cached offline credit must not survive
    /// this boundary without fresh authentication to the authority owner.
    fn begin_session(&mut self, offline_bound: u32);

    /// Read-only diagnostic state and the generation used for reservation.
    fn fence(&self) -> &RevocationFence;

    /// Authenticates the tenant and exact historical snapshots before lending
    /// the current fence. Runtime checks and spends it before calling an owner.
    /// A failed authentication must never recharge offline exposure.
    ///
    /// # Errors
    /// Returns a typed refusal when current authority cannot authenticate the
    /// supplied evidence. Disconnected operation is permitted only within a
    /// session already authenticated for these exact inputs and tenant.
    fn authenticate_reach(
        &mut self,
        intent: &EffectIntent,
        pinned: &PinnedInputs,
    ) -> Result<&mut RevocationFence, FenceRefusal>;
}

/// Synthetic authority controls for deterministic public-runtime tests only.
pub mod fixture {
    use std::collections::{BTreeMap, BTreeSet};

    use rocco_contracts::ids::{OperationId, TenantId};
    use rocco_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};

    use super::{
        AuthoritySource, EffectIntent, FenceRefusal, PinnedInputs, RevocationFence, SecurityEpoch,
    };
    use crate::FenceError;

    /// Exact authority evidence accepted for one synthetic tenant.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Snapshots {
        /// Source generation under which these exact inputs were authenticated.
        epoch: SecurityEpoch,
        /// Complete policy content, including its revision.
        policy: PolicySnapshot,
        /// Complete capability content.
        capabilities: CapabilitySnapshot,
        /// Complete validity content.
        validity: ValiditySnapshot,
    }

    /// Bounded deterministic authority owner; never real authentication.
    ///
    /// No tenant is trusted until explicitly provisioned. A connection is a
    /// simulation input, not network activity. Epochs belong to this source's
    /// domain; named withdrawals bind both tenant and operation.
    #[derive(Debug)]
    pub struct FixtureAuthority {
        /// Source-owned generation and total offline exposure budget.
        fence: RevocationFence,
        /// Independently provisioned synthetic authority per tenant.
        trusted: BTreeMap<TenantId, Snapshots>,
        /// Sticky scoped withdrawals, bounded by the journal ceiling.
        withdrawn: BTreeSet<(TenantId, OperationId)>,
        /// Simulated availability of the authority owner.
        connected: bool,
        /// Whether this runtime session freshly contacted the source.
        session_authenticated: bool,
    }

    impl FixtureAuthority {
        /// Creates a synthetic source that trusts no tenant or snapshots.
        #[must_use]
        pub const fn new(epoch: SecurityEpoch, offline_bound: u32) -> Self {
            Self {
                fence: RevocationFence::new(epoch, offline_bound),
                trusted: BTreeMap::new(),
                withdrawn: BTreeSet::new(),
                connected: true,
                session_authenticated: false,
            }
        }

        /// Provisions or replaces exact evidence for a synthetic tenant.
        ///
        /// # Errors
        /// Refuses capacity overflow and disconnected provisioning. Updating
        /// evidence never erases withdrawals or refunds offline exposure.
        pub fn trust(
            &mut self,
            tenant: TenantId,
            policy: PolicySnapshot,
            capabilities: CapabilitySnapshot,
            validity: ValiditySnapshot,
        ) -> Result<(), FenceRefusal> {
            if !self.connected {
                return Err(FenceRefusal::AuthorityUnavailable);
            }
            if !self.trusted.contains_key(&tenant) && self.trusted.len() >= crate::MAX_TRACKED_RUNS
            {
                return Err(FenceRefusal::AuthorityCapacity);
            }
            self.trusted.insert(
                tenant,
                Snapshots {
                    epoch: self.fence.epoch(),
                    policy,
                    capabilities,
                    validity,
                },
            );
            Ok(())
        }

        /// Withdraws one tenant's operation without affecting another tenant.
        ///
        /// # Errors
        /// Refuses overflow without evicting any earlier withdrawal.
        pub fn withdraw(
            &mut self,
            tenant: TenantId,
            operation: OperationId,
        ) -> Result<bool, FenceRefusal> {
            let key = (tenant, operation);
            if !self.withdrawn.contains(&key) && self.withdrawn.len() >= crate::MAX_JOURNAL_ENTRIES
            {
                return Err(FenceRefusal::AuthorityCapacity);
            }
            Ok(self.withdrawn.insert(key))
        }

        /// Sets simulated connectivity. Reconnection alone is not authentication.
        pub const fn set_connected(&mut self, connected: bool) {
            self.connected = connected;
        }

        /// Advances this authority domain's generation, withdrawing old intents.
        ///
        /// # Errors
        /// Refuses epoch exhaustion rather than wrapping.
        pub fn advance_epoch(&mut self) -> Result<SecurityEpoch, FenceError> {
            self.fence.advance_epoch()
        }

        /// Simulates an authenticated contact with the authority owner.
        /// Stale reports and disconnected contacts never recharge exposure.
        ///
        /// # Errors
        /// Refuses an unavailable source or a replayed older generation.
        pub fn authenticate(
            &mut self,
            reported: SecurityEpoch,
        ) -> Result<SecurityEpoch, FenceRefusal> {
            if !self.connected {
                return Err(FenceRefusal::AuthorityUnavailable);
            }
            if self.fence.epoch().is_later_than(reported) {
                return Err(FenceRefusal::StaleEpoch {
                    admitted: reported,
                    current: self.fence.epoch(),
                });
            }
            self.session_authenticated = true;
            Ok(self.fence.authenticate(reported))
        }
    }

    impl AuthoritySource for FixtureAuthority {
        fn begin_session(&mut self, offline_bound: u32) {
            self.fence.restrict_offline_bound(offline_bound);
            // A new session cannot inherit the old process's offline lease.
            let current = self.fence.epoch();
            self.session_authenticated =
                self.connected && self.fence.authenticate(current) == current;
        }

        fn fence(&self) -> &RevocationFence {
            &self.fence
        }

        fn authenticate_reach(
            &mut self,
            intent: &EffectIntent,
            pinned: &PinnedInputs,
        ) -> Result<&mut RevocationFence, FenceRefusal> {
            if !self.session_authenticated {
                return Err(FenceRefusal::AuthorityUnavailable);
            }
            let Some(trusted) = self.trusted.get(&intent.tenant) else {
                return Err(FenceRefusal::AuthorityTenant {
                    tenant: intent.tenant,
                });
            };
            if self.withdrawn.contains(&(intent.tenant, intent.operation)) {
                return Err(FenceRefusal::Revoked {
                    operation: intent.operation,
                });
            }
            self.fence.check_epoch(intent.admission_epoch)?;
            if trusted.epoch != self.fence.epoch()
                || trusted.policy != pinned.policy
                || trusted.capabilities != pinned.capabilities
                || trusted.validity != pinned.validity
            {
                return Err(FenceRefusal::StaleSnapshots);
            }
            Ok(&mut self.fence)
        }
    }
}
