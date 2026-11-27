//! Seeded adversarial simulations of the report quota and retention (larry#211):
//! a tenant past its byte quota is refused typed, old rows prune first, and no
//! tenant's rows ever count toward another's. One seed drives every size and
//! stamp; same seed, same trace.

use super::super::Ledger;
use super::ReportBudget;
use crate::executor::sim_tree;
use crate::sim_error::SimError;
use lgwks_std::seeded::Seeded;

/// A report of `len` bytes under `tenant` at `now`.
fn keep(
    ledger: &Ledger,
    tenant: &str,
    id: &str,
    len: usize,
    _now: u64,
    budget: &ReportBudget,
) -> Result<(), crate::executor::ExecError> {
    ledger.keep_report(tenant, id, "digest", &vec![7_u8; len], budget)
}

/// Quota and retention under one seed: refusals are typed, pruning is oldest
/// first, and tenants never share accounting.
fn quota_round(seed: u64) -> Result<(), SimError> {
    let mut rng = Seeded::from_seed(seed);
    let dir = sim_tree::scratch("larry-reports-quota")?;
    let ledger = Ledger::open(&dir)?;
    let gap = usize::try_from(rng.below(40)?.saturating_add(8))?;
    let now = 1_700_000_000_u64;
    // A small quota a test can fill, and a window of four rows.
    let budget = ReportBudget::test(256, 4, 3_600, now);
    for index in 0..4_usize {
        keep(
            &ledger,
            "tenant-a",
            &format!("run-{index}"),
            gap,
            now,
            &budget,
        )?;
    }
    // A fifth row evicts the oldest: the window, not the quota, decides.
    keep(&ledger, "tenant-a", "run-4", gap, now, &budget)?;
    if ledger.report_bytes("tenant-a")? == 0 {
        return Err(SimError::Missing("kept bytes are accounted per tenant"));
    }
    if ledger.report("tenant-a", "run-0", 1_048_576)?.is_some() {
        return Err(SimError::Missing(
            "the retention window prunes oldest first",
        ));
    }
    // Past the quota the next keep is refused typed and keeps nothing: a full
    // quota plus any held byte is past it, whatever the seed drew.
    let over = 256_usize;
    let refused = keep(&ledger, "tenant-a", "run-big", over, now, &budget);
    if !matches!(refused, Err(crate::executor::ExecError::ReportQuota { .. })) {
        return Err(SimError::Missing(
            "a tenant past its quota is refused typed",
        ));
    }
    if ledger.report("tenant-a", "run-big", 1_048_576)?.is_some() {
        return Err(SimError::Missing("a refused keep stores nothing"));
    }
    // Another tenant's accounting starts empty: no tenant pays for another's
    // bytes (larry#208 holds for evidence too).
    keep(&ledger, "tenant-b", "run-0", over, now, &budget)?;
    if ledger.report("tenant-b", "run-0", 1_048_576)?.is_none() {
        return Err(SimError::Missing(
            "one tenant's bytes never count toward another's",
        ));
    }
    // Age past the window prunes on the next keep, before the quota is read.
    let aged = ReportBudget::test(1_048_576, 512, 60, now.saturating_add(3_600));
    keep(
        &ledger,
        "tenant-a",
        "run-fresh",
        gap,
        now.saturating_add(3_600),
        &aged,
    )?;
    if ledger.report("tenant-a", "run-1", 1_048_576)?.is_some() {
        return Err(SimError::Missing(
            "the age window prunes before the quota is read",
        ));
    }
    Ok(())
}

/// Quota, retention and isolation hold for every seed.
#[test]
fn sim_quota_and_retention_hold_for_every_seed() -> Result<(), SimError> {
    for seed in 0..32_u64 {
        quota_round(seed.wrapping_add(0x2111))?;
    }
    Ok(())
}

/// The quota boundary is exact: a tenant holding exactly its quota keeps every
/// byte, and one byte more is refused typed with nothing stored.
#[test]
fn sim_a_quota_met_exactly_keeps_every_byte() -> Result<(), SimError> {
    let dir = sim_tree::scratch("larry-reports-edge")?;
    let ledger = Ledger::open(&dir)?;
    let now = 1_700_000_000_u64;
    let budget = ReportBudget::test(256, 512, 3_600, now);
    keep(&ledger, "tenant-a", "run-a", 200, now, &budget)?;
    keep(&ledger, "tenant-a", "run-b", 56, now, &budget)?;
    if ledger.report_bytes("tenant-a")? != 256 {
        return Err(SimError::Missing("a quota met exactly keeps every byte"));
    }
    if ledger.report("tenant-a", "run-b", 1_048_576)?.is_none() {
        return Err(SimError::Missing("both rows at the bound read back"));
    }
    let refused = keep(&ledger, "tenant-a", "run-c", 1, now, &budget);
    if !matches!(refused, Err(crate::executor::ExecError::ReportQuota { .. })) {
        return Err(SimError::Missing(
            "one byte past the quota is refused typed",
        ));
    }
    if ledger.report("tenant-a", "run-c", 1_048_576)?.is_some() {
        return Err(SimError::Missing("a refused keep stores nothing"));
    }
    Ok(())
}

/// The age boundary is exact: a row stamped exactly at the cutoff survives the
/// next prune, while anything older is dropped.
#[test]
fn sim_a_row_at_the_age_cutoff_survives_its_prune() -> Result<(), SimError> {
    let dir = sim_tree::scratch("larry-reports-age")?;
    let ledger = Ledger::open(&dir)?;
    let then = 1_700_000_000_u64;
    let old = ReportBudget::test(1_048_576, 512, 100, then);
    keep(&ledger, "tenant-a", "run-old", 8, then, &old)?;
    // The cutoff is exactly `then`: `<` keeps the row, `<=` would drop it.
    let now = ReportBudget::test(1_048_576, 512, 100, then.saturating_add(100));
    keep(
        &ledger,
        "tenant-a",
        "run-new",
        8,
        then.saturating_add(100),
        &now,
    )?;
    if ledger.report("tenant-a", "run-old", 1_048_576)?.is_none() {
        return Err(SimError::Missing("a row at the cutoff survives its prune"));
    }
    Ok(())
}
