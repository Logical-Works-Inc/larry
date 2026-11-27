//! One receipt key per tenant, and the stored tenant of one state directory
//! (larry#211).
//!
//! The store keeps one `dir_tenant` row: the default tenant, drawn once from OS
//! entropy and never derived from a path, so moving the directory keeps every
//! tenant id. A directory that already filed rows under its legacy path-derived
//! tenant pins that tenant at migration, so in-flight keyed runs keep their
//! ids; a fresh directory draws random. `LARRY_TENANT` selects a named tenant,
//! derived from the name alone, so two tenants share one state directory and
//! each seals under its own key.
//!
//! Receipt keys live in `receipt_keys(tenant, key_id)`: one active row per
//! tenant, older rows kept for verification up to [`MAX_KEY_HISTORY`].
//! `larry key` reads the active public key and never mints; only a run path
//! mints, at most key id 1 on first use, and `larry key rotate` appends the
//! next id. A receipt names its `kid`, so a rotated key verifies old receipts
//! by id and seals new ones under the new id.

use larry_contracts::ids::TenantId;
use lgwks_deps::storefront::rusqlite::{self, OptionalExtension};
use lgwks_std::sign::{PublicKey, SecretKey};

use super::Ledger;
use crate::executor::ExecError;
use crate::executor::identity;

/// Step 8 (7 to 8): the tenant tables for fresh directories, the report table
/// rebuilt around its tenant column, the live-group tenant column, and the old
/// global key row carried into its tenant's key id 1. Old reports and groups
/// land under the empty tenant and the old key under it too; [`Ledger::backfill_tenant`]
/// renames them to the resolved tenant right after the migrate commits.
/// Every statement runs once: the version gate repeats nothing, and a crash
/// rolls the whole step back for the next open to replay.
pub(super) const STEP_EIGHT: &str = "CREATE TABLE IF NOT EXISTS dir_tenant(id INTEGER PRIMARY KEY CHECK (id = 1), \
    tenant TEXT NOT NULL CHECK (length(tenant) = 32)); \
    CREATE TABLE IF NOT EXISTS receipt_keys(tenant TEXT NOT NULL, \
    key_id INTEGER NOT NULL CHECK (key_id >= 1), seed BLOB NOT NULL CHECK (length(seed) = 32), \
    PRIMARY KEY(tenant, key_id)); \
    CREATE TABLE IF NOT EXISTS reports_new(tenant TEXT NOT NULL DEFAULT '', task_id TEXT NOT NULL, \
    task_digest TEXT NOT NULL, report BLOB NOT NULL, sealed_at INTEGER NOT NULL DEFAULT 0, \
    PRIMARY KEY(tenant, task_id)); \
    INSERT OR IGNORE INTO reports_new(tenant, task_id, task_digest, report, sealed_at) \
    SELECT '', task_id, task_digest, report, 0 FROM reports; \
    DROP TABLE reports; \
    ALTER TABLE reports_new RENAME TO reports; \
    ALTER TABLE live_groups ADD COLUMN tenant TEXT NOT NULL DEFAULT ''; \
    INSERT OR IGNORE INTO receipt_keys(tenant, key_id, seed) SELECT '', 1, seed FROM receipt_key; \
    DROP TABLE receipt_key;";

/// The empty tenant placeholder step 8 files old rows under before the rename.
const UNCLAIMED: &str = "";

/// Key ids a tenant keeps for verification, newest first.
const MAX_KEY_HISTORY: i64 = 16;

/// The 32 hex characters of a tenant id, the text every tenant-keyed row carries.
pub(super) fn tenant_hex(tenant: &TenantId) -> String {
    lgwks_std::hex::encode(tenant.as_bytes())
}

/// The tenant id 32 hex characters name; anything else is a damaged row, with
/// the decoder's own cause.
fn tenant_of_hex(text: &str) -> Result<TenantId, ExecError> {
    let mut bytes = [0_u8; 16];
    lgwks_std::hex::decode_into(text, &mut bytes).map_err(ExecError::BadTenantId)?;
    Ok(TenantId::from_bytes(bytes))
}

impl Ledger {
    /// The store's default tenant: the stored row, drawn once from OS entropy
    /// by the first opener and never derived from a path, so one directory is
    /// one tenant whatever the spelling and a move keeps its id (larry#211).
    /// Executors racing on a fresh store each draw, and `INSERT OR IGNORE`
    /// keeps the first; every racer then reads that row back. Rows an old
    /// binary filed under its path-derived tenant are renamed onto the stored
    /// one, so surviving config keeps working; keyed run ids minted before the
    /// migration map anew, so one in-flight retry may run again.
    pub(crate) fn dir_tenant(&self, state: &std::path::Path) -> Result<TenantId, ExecError> {
        if self.stored_dir_tenant()?.is_none() {
            let drawn = lgwks_std::random::bytes::<16>().map_err(ExecError::Entropy)?;
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO dir_tenant (id, tenant) VALUES (1, ?1)",
                    rusqlite::params![tenant_hex(&TenantId::from_bytes(drawn))],
                )
                .map_err(ExecError::Sql)?;
        }
        let pinned = self.stored_dir_tenant()?.ok_or(ExecError::KeyMissing)?;
        self.rekey_legacy(state, &tenant_hex(&pinned))?;
        Ok(pinned)
    }

    /// Renames rows an old binary filed under its path-derived tenant onto the
    /// stored one: policies, roots and routes keep working after the move to an
    /// identity record. Idempotent: only rows under the legacy spelling move,
    /// and a replay moves nothing. A directory the old binary reached through
    /// several spellings keeps the current spelling's rows; older spellings'
    /// rows stay orphaned, never merged.
    fn rekey_legacy(&self, state: &std::path::Path, pinned: &str) -> Result<(), ExecError> {
        let legacy = tenant_hex(&identity::legacy_tenant_of(state));
        if legacy == pinned {
            return Ok(());
        }
        for table in ["tenant_policies", "tenant_repo_roots", "routes"] {
            self.conn
                .execute(
                    &format!("UPDATE {table} SET tenant = ?1 WHERE tenant = ?2"),
                    rusqlite::params![pinned, legacy],
                )
                .map_err(ExecError::Sql)?;
        }
        Ok(())
    }

    /// The stored default tenant, if a row exists; a row whose text is not 32
    /// hex characters of a tenant is damage.
    fn stored_dir_tenant(&self) -> Result<Option<TenantId>, ExecError> {
        let row = self
            .conn
            .query_row("SELECT tenant FROM dir_tenant WHERE id = 1", [], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(ExecError::Sql)?;
        row.map(|text| tenant_of_hex(&text)).transpose()
    }

    /// Renames step 8's placeholder tenant to the resolved one: old reports,
    /// groups and the carried key row join the tenant that owns them. Runs on
    /// every open; only placeholder rows move, so a replay renames nothing.
    /// `now` stamps legacy reports whose age is unknown as fresh, so the
    /// retention window prunes them last, never first (larry#211).
    pub(crate) fn backfill_tenant(&self, tenant: &str, now: u64) -> Result<(), ExecError> {
        for table in ["reports", "live_groups", "receipt_keys"] {
            self.conn
                .execute(
                    &format!("UPDATE {table} SET tenant = ?1 WHERE tenant = ?2"),
                    rusqlite::params![tenant, UNCLAIMED],
                )
                .map_err(ExecError::Sql)?;
        }
        self.conn
            .execute(
                "UPDATE reports SET sealed_at = ?1 WHERE sealed_at = 0",
                rusqlite::params![i64::try_from(now)?],
            )
            .map_err(ExecError::Sql)?;
        Ok(())
    }

    /// The tenant this call runs as: `LARRY_TENANT` when it names one, else the
    /// store's default. An empty variable is unset, like every other `LARRY_*`
    /// one. Returns the id and the hex every row carries.
    pub(crate) fn selected_tenant(
        &self,
        state: &std::path::Path,
        read: &impl Fn(&str) -> Option<String>,
    ) -> Result<(TenantId, String), ExecError> {
        match read("LARRY_TENANT").filter(|value| !value.is_empty()) {
            Some(name) => {
                let tenant = identity::named_tenant(&name)?;
                Ok((tenant, tenant_hex(&tenant)))
            }
            None => {
                let tenant = self.dir_tenant(state)?;
                Ok((tenant, tenant_hex(&tenant)))
            }
        }
    }

    /// The receipt signing key this tenant seals under: the active (newest) row,
    /// or a fresh key id 1 minted from OS entropy. Only a run path calls this;
    /// reads use [`Ledger::active_receipt_key`], which never mints.
    pub(crate) fn signing_key_for(&self, tenant: &str) -> Result<(i64, SecretKey), ExecError> {
        if let Some(active) = self.stored_receipt_key(tenant, None)? {
            return Ok(active);
        }
        let seed = lgwks_std::random::bytes::<32>().map_err(ExecError::Entropy)?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO receipt_keys (tenant, key_id, seed) VALUES (?1, 1, ?2)",
                rusqlite::params![tenant, seed.as_slice()],
            )
            .map_err(ExecError::Sql)?;
        self.stored_receipt_key(tenant, None)?
            .ok_or(ExecError::KeyMissing)
    }

    /// The active receipt key id and public key, if this tenant holds one; a
    /// tenant with no key yet reads as none, and no seed is drawn (larry#211).
    pub(crate) fn active_receipt_key(
        &self,
        tenant: &str,
    ) -> Result<Option<(i64, PublicKey)>, ExecError> {
        Ok(self
            .stored_receipt_key(tenant, None)?
            .map(|(id, secret)| (id, secret.public_key())))
    }

    /// Every receipt public key this tenant holds, newest first, for verifying
    /// a receipt by its key id.
    pub(crate) fn receipt_public_keys(
        &self,
        tenant: &str,
    ) -> Result<Vec<(i64, PublicKey)>, ExecError> {
        let mut statement = self
            .conn
            .prepare("SELECT key_id, seed FROM receipt_keys WHERE tenant = ?1 ORDER BY key_id DESC")
            .map_err(ExecError::Sql)?;
        let mapped = statement
            .query_map(rusqlite::params![tenant], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(ExecError::Sql)?;
        let mut keys = Vec::new();
        for entry in mapped {
            let (id, stored) = entry.map_err(ExecError::Sql)?;
            let seed = <[u8; 32]>::try_from(stored.as_slice()).map_err(ExecError::KeySeedLength)?;
            keys.push((id, SecretKey::from_seed(seed).public_key()));
        }
        Ok(keys)
    }

    /// The stored key: `only` names the key id wanted, `None` the active
    /// (newest) one; a row whose seed is not 32 bytes is damage.
    fn stored_receipt_key(
        &self,
        tenant: &str,
        only: Option<i64>,
    ) -> Result<Option<(i64, SecretKey)>, ExecError> {
        use lgwks_deps::storefront::rusqlite::OptionalExtension as _;
        let row = match only {
            Some(id) => self
                .conn
                .query_row(
                    "SELECT key_id, seed FROM receipt_keys WHERE tenant = ?1 AND key_id = ?2",
                    rusqlite::params![tenant, id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )
                .optional()
                .map_err(ExecError::Sql)?,
            None => self
                .conn
                .query_row(
                    "SELECT key_id, seed FROM receipt_keys WHERE tenant = ?1 ORDER BY key_id DESC LIMIT 1",
                    rusqlite::params![tenant],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )
                .optional()
                .map_err(ExecError::Sql)?,
        };
        row.map(|(id, stored)| {
            <[u8; 32]>::try_from(stored.as_slice())
                .map(SecretKey::from_seed)
                .map(|secret| (id, secret))
                .map_err(ExecError::KeySeedLength)
        })
        .transpose()
    }

    /// The secret key of `key_id` for this tenant, for answering a retry from
    /// the exact row its receipt names; none when the id was pruned or never
    /// minted.
    pub(crate) fn receipt_secret(
        &self,
        tenant: &str,
        key_id: i64,
    ) -> Result<Option<SecretKey>, ExecError> {
        Ok(self
            .stored_receipt_key(tenant, Some(key_id))?
            .map(|(_, secret)| secret))
    }

    /// Appends this tenant's next key id and returns it with its secret, so new
    /// seals move while old receipts still verify by id. History past
    /// [`MAX_KEY_HISTORY`] is dropped, oldest first.
    pub(crate) fn rotate_receipt_key(&self, tenant: &str) -> Result<(i64, SecretKey), ExecError> {
        let next = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(key_id), 0) + 1 FROM receipt_keys WHERE tenant = ?1",
                rusqlite::params![tenant],
                |row| row.get::<_, i64>(0),
            )
            .map_err(ExecError::Sql)?;
        let seed = lgwks_std::random::bytes::<32>().map_err(ExecError::Entropy)?;
        self.conn
            .execute(
                "INSERT INTO receipt_keys (tenant, key_id, seed) VALUES (?1, ?2, ?3)",
                rusqlite::params![tenant, next, seed.as_slice()],
            )
            .map_err(ExecError::Sql)?;
        self.conn
            .execute(
                "DELETE FROM receipt_keys WHERE tenant = ?1 AND key_id NOT IN \
                 (SELECT key_id FROM receipt_keys WHERE tenant = ?1 ORDER BY key_id DESC LIMIT ?2)",
                rusqlite::params![tenant, MAX_KEY_HISTORY],
            )
            .map_err(ExecError::Sql)?;
        Ok((next, SecretKey::from_seed(seed)))
    }
}

/// Seeded adversarial simulations of the tenant store: ids, keys and rotation.
#[cfg(test)]
#[path = "sim_tenant_keys.rs"]
mod sim;
