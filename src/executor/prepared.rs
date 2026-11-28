//! What one run prepares: the opened ledger, the selected tenant and the key
//! the run seals under (larry#211 part B).

use larry_contracts::ids::TenantId;
use lgwks_std::sign::SecretKey;

use super::{ExecError, Ledger, Settings, recovery, sweep};

/// The opened ledger, the selected tenant and the key the run seals under.
pub(crate) struct Prepared {
    /// The state directory's store.
    pub(crate) ledger: Ledger,
    /// The tenant this run executes as.
    pub(crate) tenant: TenantId,
    /// The hex text every ledger row of this tenant carries.
    pub(crate) tenant_text: String,
    /// The active receipt key id this run seals under.
    pub(crate) seal_id: i64,
    /// The secret that seals.
    pub(crate) seal_key: SecretKey,
}

/// What every run does first: opens the ledger and the tenant's receipt key, reaps the process
/// groups a killed executor left, sweeps dead workspaces, and settles every journal a
/// killed executor left before anything new runs (larry#75). A run is live while a
/// process holds its journal's lock, so no sweep and no recovery reaps a concurrent
/// run's evidence, and neither asks for a process id (larry#182 defect 3).
///
/// The key is minted here, on the run path, at most key id 1 on first use: the
/// read-only lookups behind `larry key` and `larry verify` never mint (larry#211).
pub(crate) fn prepare(settings: &Settings) -> Result<Prepared, ExecError> {
    let ledger = Ledger::open(&settings.state_dir)?;
    let tenant = settings.tenant();
    let tenant_text = settings.tenant_text().to_owned();
    let (seal_id, seal_key) = ledger.signing_key_for(&tenant_text)?;
    // The one clock the reap's group waits read, built the way the gate builds its own, so a
    // host whose clock reads before the epoch refuses with nothing reaped (larry#268, larry#308).
    let clock = larry_runtime::live_clock().map_err(ExecError::Gate)?;
    let state = settings.state_dir.as_path();
    ledger.reap(&clock, |task| recovery::run_held(state, task))?;
    let policy = &settings.policy;
    for fault in sweep(&settings.state_dir, &ledger.live_tasks()?, |task| {
        recovery::journal_live(state, policy, task)
    })? {
        fault.report();
    }
    recovery::recover(&settings.state_dir, &settings.policy)?;
    Ok(Prepared {
        ledger,
        tenant,
        tenant_text,
        seal_id,
        seal_key,
    })
}
