//! The bounded tenant-route budget one executor process shares (larry#204):
//! at most 256 keys, each the authenticated tenant and the resolved route.
//! Split from the completion client so the client stays under the size law;
//! the client is the limiter's one caller beside these tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::ModelError;

/// The bounded state shared by model clients in one executor process.
///
/// Each key holds what its tenant-route settled and what its calls in flight
/// still hold: `before` reserves the call's request size as pending tokens
/// (and the priced micros) under the same lock that checks the budgets, and
/// `record` settles that reservation to the usage the call reports, releasing
/// it whole on a failed call. A check without the hold would admit every call
/// in flight before any of them records, spending N times past the budget.
pub(crate) struct Limiter {
    /// Tenant and route usage, bounded before a new key is inserted.
    entries: Mutex<BTreeMap<(String, String), Budget>>,
}

/// What one tenant-route key settled and still holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Budget {
    /// Reported tokens settled by answered calls.
    spent_tokens: u64,
    /// Reported micro-dollars settled by answered calls.
    spent_micros: u64,
    /// Request sizes held by calls the gateway has not answered yet.
    pending_tokens: u64,
    /// Their priced micro-dollar hold.
    pending_micros: u64,
}

impl Limiter {
    /// Maximum distinct tenant-route keys held by one process.
    const MAX_KEYS: usize = 256;

    /// A fresh process-local limiter.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(BTreeMap::new()),
        })
    }

    /// Reserve a call already over either configured budget, before any HTTP
    /// work: the call's `reserve_tokens` (its request size) and
    /// `reserve_micros` (priced from the route's table) join the key's pending
    /// hold under the same lock that checks spent plus pending against both
    /// budgets, so calls in flight never admit past what the budgets allow.
    pub(crate) fn before(
        &self,
        tenant: &str,
        route: &str,
        token_limit: u64,
        micro_limit: u64,
        reserve_tokens: u64,
        reserve_micros: u64,
    ) -> Result<(), ModelError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|cause| ModelError::BudgetUnavailable {
                reason: cause.to_string(),
            })?;
        let key = (tenant.to_owned(), route.to_owned());
        let usage = match entries.get(&key) {
            Some(usage) => *usage,
            None if entries.len() < Self::MAX_KEYS => Budget::default(),
            None => {
                return Err(ModelError::LimiterFull {
                    limit: Self::MAX_KEYS,
                });
            }
        };
        if usage.spent_tokens.saturating_add(usage.pending_tokens) >= token_limit {
            return Err(ModelError::BudgetExceeded {
                tokens: usage.spent_tokens,
                limit: token_limit,
            });
        }
        if usage.spent_micros.saturating_add(usage.pending_micros) >= micro_limit {
            return Err(ModelError::MoneyBudgetExceeded {
                micros: usage.spent_micros,
                limit: micro_limit,
            });
        }
        let held = entries.entry(key).or_insert(usage);
        held.pending_tokens = held.pending_tokens.saturating_add(reserve_tokens);
        held.pending_micros = held.pending_micros.saturating_add(reserve_micros);
        Ok(())
    }

    /// Settle a call `before` admitted: release its `reserve_tokens` and
    /// `reserve_micros` hold and add the usage its answer reported. A failed
    /// call settles zero usage, so its hold is only released; a reported
    /// usage settles even past the budget, because the gateway already spent
    /// it. A key the limiter never admitted is a new key, so past the bound
    /// it is refused rather than grown: the table never holds more than its
    /// key budget.
    pub(crate) fn record(
        &self,
        tenant: &str,
        route: &str,
        reserve_tokens: u64,
        reserve_micros: u64,
        tokens: u64,
        micros: u64,
    ) -> Result<(), ModelError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|cause| ModelError::BudgetUnavailable {
                reason: cause.to_string(),
            })?;
        let key = (tenant.to_owned(), route.to_owned());
        if !entries.contains_key(&key) && entries.len() >= Self::MAX_KEYS {
            return Err(ModelError::LimiterFull {
                limit: Self::MAX_KEYS,
            });
        }
        let usage = entries.entry(key).or_insert_with(Budget::default);
        usage.pending_tokens = usage.pending_tokens.saturating_sub(reserve_tokens);
        usage.pending_micros = usage.pending_micros.saturating_sub(reserve_micros);
        usage.spent_tokens = usage.spent_tokens.saturating_add(tokens);
        usage.spent_micros = usage.spent_micros.saturating_add(micros);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::{Limiter, ModelError};

    /// Two tenants sharing one limiter keep their own budgets: one tenant's
    /// burst to its token budget refuses only its own next call, and the other
    /// tenant still admits and records under the same route.
    #[test]
    fn sim_two_tenants_sharing_one_limiter_keep_their_own_budgets() -> Result<(), ModelError> {
        for seed in 0..16_u64 {
            let limiter = Limiter::new();
            let spent = format!("tenant-{seed}-spent");
            let fresh = format!("tenant-{seed}-fresh");
            limiter.record(&spent, "route", 0, 0, 1_000, 50)?;
            assert!(
                matches!(
                    limiter.before(&spent, "route", 1_000, u64::MAX, 0, 0),
                    Err(ModelError::BudgetExceeded {
                        tokens: 1_000,
                        limit: 1_000
                    })
                ),
                "seed {seed}: the spent tenant is refused at its budget"
            );
            limiter.before(&fresh, "route", 1_000, u64::MAX, 0, 0)?;
            limiter.record(&fresh, "route", 0, 0, 1, 0)?;
            limiter.before(&fresh, "route", 1_000, u64::MAX, 0, 0)?;
        }
        Ok(())
    }

    /// A full limiter refuses a new tenant-route key on both paths: `before`
    /// admits nothing past 256 keys, and `record` refuses rather than growing
    /// past the bound, while a key already held still admits.
    #[test]
    fn sim_a_full_limiter_refuses_a_new_key_on_record() -> Result<(), ModelError> {
        let limiter = Limiter::new();
        for index in 0..256_u32 {
            limiter.record(&format!("tenant-{index}"), "route", 0, 0, 0, 0)?;
        }
        assert!(
            matches!(
                limiter.record("tenant-new", "route", 0, 0, 0, 0),
                Err(ModelError::LimiterFull { limit: 256 })
            ),
            "a record past 256 keys is refused, never grown"
        );
        assert!(
            matches!(
                limiter.before("tenant-new", "route", u64::MAX, u64::MAX, 0, 0),
                Err(ModelError::LimiterFull { limit: 256 })
            ),
            "a call past 256 keys is refused before any gateway byte"
        );
        limiter.before("tenant-0", "route", u64::MAX, u64::MAX, 0, 0)?;
        Ok(())
    }

    /// Calls in flight share one budget: `before` holds each call's reserve
    /// as pending tokens under the same lock that checks the budgets, and
    /// `record` settles it to the reported usage. 100 and 1,000 threads on
    /// one tenant-route with a 900-token budget, each reserving and settling
    /// 10 tokens, admit exactly 90 calls each: no schedule spends past the
    /// budget, and the other tenant is untouched.
    #[test]
    fn sim_concurrent_reservations_hold_one_budget() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        const BUDGET: u64 = 900;
        const RESERVE: u64 = 10;
        // BUDGET / RESERVE: the budget admits exactly this many calls.
        const ADMITTED: usize = 90;
        for seed in 0..4_u64 {
            for threads in [100_usize, 1000] {
                let limiter = Limiter::new();
                let tenant = format!("tenant-{seed}");
                let entered = Arc::new(Barrier::new(threads));
                let settled = Arc::new(Barrier::new(threads));
                let admitted = Arc::new(AtomicUsize::new(0));
                let refused = Arc::new(AtomicUsize::new(0));
                std::thread::scope(|scope| {
                    for _ in 0..threads {
                        scope.spawn(|| {
                            entered.wait();
                            let held = limiter
                                .before(&tenant, "route", BUDGET, u64::MAX, RESERVE, 0)
                                .is_ok();
                            settled.wait();
                            if held {
                                admitted.fetch_add(1, Ordering::Relaxed);
                            } else {
                                refused.fetch_add(1, Ordering::Relaxed);
                            }
                            if held {
                                assert!(
                                    limiter
                                        .record(&tenant, "route", RESERVE, 0, RESERVE, 0)
                                        .is_ok(),
                                    "seed {seed}: a settle after its admit never refuses"
                                );
                            }
                        });
                    }
                });
                let admitted = admitted.load(Ordering::Relaxed);
                let refused = refused.load(Ordering::Relaxed);
                assert_eq!(
                    admitted, ADMITTED,
                    "seed {seed} with {threads} threads: the budget admits exactly its share"
                );
                assert_eq!(
                    refused,
                    threads.saturating_sub(ADMITTED),
                    "seed {seed} with {threads} threads: the rest are refused, never spent"
                );
                assert!(
                    matches!(
                        limiter.before(&tenant, "route", BUDGET, u64::MAX, RESERVE, 0),
                        Err(ModelError::BudgetExceeded {
                            tokens: BUDGET,
                            limit: BUDGET
                        })
                    ),
                    "seed {seed}: the settled budget refuses the next call"
                );
                assert!(
                    limiter
                        .before(
                            &format!("tenant-{seed}-other"),
                            "route",
                            BUDGET,
                            u64::MAX,
                            RESERVE,
                            0
                        )
                        .is_ok(),
                    "seed {seed}: the burst never touches the other tenant"
                );
            }
        }
    }
}
