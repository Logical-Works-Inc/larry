//! Tenant-scoped sealed reports with retention and a byte quota (larry#211).
//!
//! `reports(tenant, task_id)` holds the sealed report a keyed run's retry
//! answers from, so the body never runs twice. A tenant keeps at most
//! [`MAX_REPORT_ROWS`] newest reports and [`RETENTION_SECS`] of age; past
//! [`REPORT_QUOTA_BYTES`] the store refuses the next report typed instead of
//! growing forever. The quota fits one largest legal report, so any single
//! admissible run can always be kept.

use lgwks_deps::storefront::rusqlite::{self, OptionalExtension};

use super::Ledger;
use crate::executor::ExecError;
use crate::executor::receipt::ReceiptError;

/// Newest sealed reports a tenant keeps; older rows are pruned on every keep.
const MAX_REPORT_ROWS: i64 = 512;

/// Age past which a sealed report is pruned on the next keep, in seconds.
const RETENTION_SECS: u64 = 30 * 24 * 3_600;

/// Bytes of sealed reports a tenant may hold; past it the next keep is refused
/// with [`ExecError::ReportQuota`].
const REPORT_QUOTA_BYTES: u64 = 268_435_456;

/// The bounds a keep enforces: the quota, the row window, the age window and
/// the instant the report seals at, in unix seconds. Production passes the live
/// clock's reading; simulations pass seeded values, so retention is
/// deterministic (larry#211, SD-4).
pub(crate) struct ReportBudget {
    /// Bytes of sealed reports the tenant may hold.
    quota_bytes: u64,
    /// Newest reports the tenant keeps.
    retain_rows: i64,
    /// Age past which a report is pruned, in seconds.
    retain_secs: u64,
    /// The instant this report seals at, in unix seconds.
    now: u64,
}

impl ReportBudget {
    /// The production bounds at `now`, in unix seconds.
    pub(crate) fn live(now: u64) -> Self {
        Self {
            quota_bytes: REPORT_QUOTA_BYTES,
            retain_rows: MAX_REPORT_ROWS,
            retain_secs: RETENTION_SECS,
            now,
        }
    }

    /// The test bounds: `quota` bytes, `rows` newest reports, `secs` of age.
    #[cfg(test)]
    pub(crate) fn test(quota: u64, rows: i64, secs: u64, now: u64) -> Self {
        Self {
            quota_bytes: quota,
            retain_rows: rows,
            retain_secs: secs,
            now,
        }
    }

    /// The quota this budget enforces, for the refusal it names.
    pub(crate) fn quota(&self) -> u64 {
        self.quota_bytes
    }

    /// The instant reports seal at under this budget, in unix seconds.
    pub(crate) fn now(&self) -> u64 {
        self.now
    }
}

impl Ledger {
    /// The sealed report `task_id` already recorded for `tenant`, if this
    /// tenant holds one no longer than `limit` bytes. A keyed run id a retry
    /// derives reads its earlier run's report here, so the body is never
    /// dispatched twice (larry#182). A report another tenant recorded under the
    /// same id reads as none: run ids derive from the tenant, and the scope is
    /// the second bound.
    ///
    /// # Errors
    ///
    /// [`ExecError::Sql`] when the statement fails, and [`ExecError::Receipt`]
    /// when the recorded bytes are longer than `limit`, refused rather than
    /// truncated.
    pub(crate) fn report(
        &self,
        tenant: &str,
        task_id: &str,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, ExecError> {
        let row = self
            .conn
            .query_row(
                "SELECT report FROM reports WHERE tenant = ?1 AND task_id = ?2",
                rusqlite::params![tenant, task_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(ExecError::Sql)?;
        match row {
            Some(recorded) if recorded.len() > limit => {
                Err(ExecError::Receipt(ReceiptError::TooLarge {
                    len: recorded.len(),
                }))
            }
            other => Ok(other),
        }
    }

    /// Records `report` as (`tenant`, `task_id`)'s evidence, unless that pair
    /// already holds one: the first run's record is the run, and a later row
    /// under one id would be a second claim about one request (larry#182
    /// defect 3). Prunes the tenant's reports past `budget`'s windows first,
    /// then refuses with [`ExecError::ReportQuota`] when the tenant still holds
    /// its quota: evidence is bounded per tenant, never unbounded per host.
    pub(crate) fn keep_report(
        &self,
        tenant: &str,
        task_id: &str,
        task_digest: &str,
        report: &[u8],
        budget: &ReportBudget,
    ) -> Result<(), ExecError> {
        let written = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO reports (tenant, task_id, task_digest, report, sealed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    tenant,
                    task_id,
                    task_digest,
                    report,
                    i64::try_from(budget.now())?
                ],
            )
            .map_err(ExecError::Sql)?;
        if written == 0 {
            return Ok(());
        }
        self.prune_reports(tenant, budget)?;
        // The store counts bytes in its own integer; the quota compares in a
        // width no sum can overflow, so no conversion can fail or truncate.
        let held: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(report)), 0) FROM reports WHERE tenant = ?1",
                rusqlite::params![tenant],
                |row| row.get::<_, i64>(0),
            )
            .map_err(ExecError::Sql)?;
        if i128::from(held) > i128::from(budget.quota()) {
            self.conn
                .execute(
                    "DELETE FROM reports WHERE tenant = ?1 AND task_id = ?2",
                    rusqlite::params![tenant, task_id],
                )
                .map_err(ExecError::Sql)?;
            return Err(ExecError::ReportQuota {
                limit: budget.quota(),
            });
        }
        Ok(())
    }

    /// Drops this tenant's reports past the age window, then past the row
    /// window, oldest first.
    fn prune_reports(&self, tenant: &str, budget: &ReportBudget) -> Result<(), ExecError> {
        let cutoff = i64::try_from(budget.now().saturating_sub(budget.retain_secs))?;
        self.conn
            .execute(
                "DELETE FROM reports WHERE tenant = ?1 AND sealed_at < ?2",
                rusqlite::params![tenant, cutoff],
            )
            .map_err(ExecError::Sql)?;
        self.conn
            .execute(
                "DELETE FROM reports WHERE tenant = ?1 AND rowid NOT IN \
                 (SELECT rowid FROM reports WHERE tenant = ?1 ORDER BY rowid DESC LIMIT ?2)",
                rusqlite::params![tenant, budget.retain_rows],
            )
            .map_err(ExecError::Sql)?;
        Ok(())
    }

    /// Bytes of sealed reports `tenant` holds, in the store's own integer.
    #[cfg(test)]
    pub(crate) fn report_bytes(&self, tenant: &str) -> Result<i64, ExecError> {
        self.conn
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(report)), 0) FROM reports WHERE tenant = ?1",
                rusqlite::params![tenant],
                |row| row.get::<_, i64>(0),
            )
            .map_err(ExecError::Sql)
    }
}

/// Seeded adversarial simulations of the quota and retention (larry#211).
#[cfg(test)]
#[path = "sim_reports.rs"]
mod sim;
