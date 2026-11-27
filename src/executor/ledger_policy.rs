//! Tenant-owned repository roots and policy rows in the executor ledger.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use larry_contracts::authority::Scope;
use lgwks_deps::storefront::rusqlite::{self, OptionalExtension};

use super::{ExecError, Ledger};

/// Schema of the durable per-tenant policy, version 6 (larry#207).
pub(super) const POLICY_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS tenant_policies(tenant TEXT PRIMARY KEY, \
    scopes TEXT NOT NULL CHECK (length(scopes) <= 512));";

/// Schema of the durable per-tenant repository roots, version 7 (larry#207).
pub(super) const REPO_ROOT_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS tenant_repo_roots(tenant TEXT NOT NULL, \
    root TEXT NOT NULL CHECK (length(root) <= 4096), PRIMARY KEY(tenant, root));";

/// Maximum durable repository roots one tenant may hold.
const MAX_REPO_ROOTS: usize = 64;
/// SQLite representation of the repository-root bound.
const MAX_REPO_ROOTS_SQL: i64 = 64;

impl Ledger {
    /// Stores one tenant's policy as ordered scope names. An empty set is a durable
    /// deny-all policy, while an absent row is also deny-all for remote doors.
    pub(crate) fn set_tenant_policy(
        &self,
        tenant: &str,
        scopes: &BTreeSet<Scope>,
    ) -> Result<(), ExecError> {
        let encoded = scopes
            .iter()
            .map(|scope| super::super::gate::scope_name(*scope))
            .collect::<Vec<_>>()
            .join(",");
        self.conn
            .execute(
                "INSERT INTO tenant_policies (tenant, scopes) VALUES (?1, ?2) \
                 ON CONFLICT (tenant) DO UPDATE SET scopes = excluded.scopes",
                rusqlite::params![tenant, encoded],
            )
            .map_err(ExecError::Sql)?;
        Ok(())
    }

    /// Reads one tenant's policy from the authoritative state ledger. No row is
    /// the remote door's fail-closed deny-all policy.
    pub(crate) fn tenant_policy(&self, tenant: &str) -> Result<BTreeSet<Scope>, ExecError> {
        let encoded = self
            .conn
            .query_row(
                "SELECT scopes FROM tenant_policies WHERE tenant = ?1",
                rusqlite::params![tenant],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(ExecError::Sql)?;
        super::super::gate::policy_names(encoded.as_deref())
    }

    /// Adds a canonical repository root to a tenant's bounded allowlist.
    pub(crate) fn set_tenant_repo_root(&self, tenant: &str, root: &Path) -> Result<(), ExecError> {
        if !root.is_absolute() {
            return Err(ExecError::RelativeRepoRoot {
                repo: root.display().to_string(),
            });
        }
        let canonical = fs::canonicalize(root).map_err(|cause| ExecError::RepoRoot {
            repo: root.to_path_buf(),
            cause,
        })?;
        let written = self
            .conn
            .execute(
                "INSERT INTO tenant_repo_roots (tenant, root) SELECT ?1, ?2 \
                 WHERE EXISTS (SELECT 1 FROM tenant_repo_roots WHERE tenant = ?1 AND root = ?2) \
                 OR (SELECT COUNT(*) FROM tenant_repo_roots WHERE tenant = ?1) < ?3 \
                 ON CONFLICT (tenant, root) DO UPDATE SET root = excluded.root",
                rusqlite::params![tenant, canonical.to_string_lossy(), MAX_REPO_ROOTS_SQL],
            )
            .map_err(ExecError::Sql)?;
        if written == 0 {
            return Err(ExecError::RepoRootLimit {
                limit: MAX_REPO_ROOTS,
            });
        }
        Ok(())
    }

    /// Canonicalizes and authorizes a task repository before any clone or model
    /// call. State-directory paths are always refused, even if allowlisted. A
    /// local door with no provisioned root row allows any absolute path outside
    /// the state directory; once the tenant holds a root row it binds the local
    /// door too, and a remote door denies when the row is absent.
    pub(crate) fn authorized_repo(
        &self,
        tenant: &str,
        state: &Path,
        repo: &Path,
        local: bool,
    ) -> Result<PathBuf, ExecError> {
        if !repo.is_absolute() {
            return Err(ExecError::RepoNotAllowed {
                repo: repo.to_path_buf(),
            });
        }
        let canonical = fs::canonicalize(repo).map_err(|cause| ExecError::RepoPath {
            repo: repo.to_path_buf(),
            cause,
        })?;
        let canonical_state = fs::canonicalize(state).map_err(ExecError::StateDir)?;
        if canonical.starts_with(&canonical_state) {
            return Err(ExecError::RepoInsideState { repo: canonical });
        }
        let mut statement = self
            .conn
            .prepare("SELECT root FROM tenant_repo_roots WHERE tenant = ?1 ORDER BY root LIMIT ?2")
            .map_err(ExecError::Sql)?;
        let roots = statement
            .query_map(rusqlite::params![tenant, MAX_REPO_ROOTS_SQL], |row| {
                row.get::<_, String>(0)
            })
            .map_err(ExecError::Sql)?
            .collect::<Result<Vec<String>, rusqlite::Error>>()
            .map_err(ExecError::Sql)?;
        let allowed = roots
            .iter()
            .any(|root| canonical.starts_with(Path::new(root)));
        // A first-timer's local door stays open until the tenant's first root
        // row binds it; a remote door denies when its row is absent (larry#207).
        if !allowed && !(local && roots.is_empty()) {
            return Err(ExecError::RepoNotAllowed { repo: canonical });
        }
        Ok(canonical)
    }
}

#[cfg(test)]
mod tests {
    use super::Ledger;
    use crate::executor::sim_tree::scratch;

    /// The one authorization: a local door with no root row allows an absolute
    /// path outside the state directory and binds once its first root exists,
    /// while a remote door denies with no row (larry#207).
    #[test]
    fn a_local_door_stays_open_until_its_first_root() -> Result<(), Box<dyn std::error::Error>> {
        let state = scratch("issue207-policy-state")?;
        let elsewhere = scratch("issue207-policy-elsewhere")?;
        let ledger = Ledger::open(&state)?;
        assert!(
            ledger
                .authorized_repo("nobody", &state, &elsewhere, true)
                .is_ok(),
            "no root row leaves the local door open"
        );
        assert!(matches!(
            ledger.authorized_repo("nobody", &state, &elsewhere, false),
            Err(super::super::ExecError::RepoNotAllowed { .. })
        ));
        let root = scratch("issue207-policy-root")?;
        ledger.set_tenant_repo_root("tenant", &root)?;
        assert!(
            ledger
                .authorized_repo("tenant", &state, &root, true)
                .is_ok(),
            "a provisioned root allows its own tree on the local door"
        );
        assert!(
            ledger
                .authorized_repo("tenant", &state, &elsewhere, true)
                .is_err(),
            "a provisioned root binds the local door too"
        );
        Ok(())
    }
}
