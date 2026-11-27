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
use crate::executor::identity::{legacy_tenant_of, named_tenant};

/// Step 9 (8 to 9, larry#211 part B): the tenant key tables for per-tenant
/// receipt keys and the stored default tenant. Additive only: the old global
/// `receipt_key` row is carried into `receipt_keys` under the empty placeholder,
/// and [`Ledger::backfill_tenant`] renames it to the resolved tenant right
/// after the migrate commits. Reports and live groups keep their shape here;
/// their tenant columns land in part C. Every statement runs once: the version
/// gate repeats nothing, and a crash rolls the whole step back for the next
/// open to replay.
pub(super) const STEP_TEN: &str = "CREATE TABLE IF NOT EXISTS dir_tenant(id INTEGER PRIMARY KEY CHECK (id = 1), \
    tenant TEXT NOT NULL CHECK (length(tenant) = 32)); \
    CREATE TABLE IF NOT EXISTS receipt_keys(tenant TEXT NOT NULL, \
    key_id INTEGER NOT NULL CHECK (key_id >= 1), seed BLOB NOT NULL CHECK (length(seed) = 32), \
    PRIMARY KEY(tenant, key_id)); \
    INSERT OR IGNORE INTO receipt_keys(tenant, key_id, seed) SELECT '', 1, seed FROM receipt_key;";

/// The empty tenant placeholder step 8 files old rows under before the rename.
const UNCLAIMED: &str = "";

/// Key ids a tenant keeps for verification, newest first.
const MAX_KEY_HISTORY: i64 = 16;

impl Ledger {
    /// The store's default tenant: the stored row, drawn once from OS entropy
    /// by the first opener and never derived from a path, so one directory is
    /// one tenant whatever the spelling and a move keeps its id (larry#211).
    /// Executors racing on a fresh store each draw, and `INSERT OR IGNORE`
    /// keeps the first; every racer then reads that row back. Part B draws
    /// without renaming legacy rows; their rename lands in part C with the
    /// report and group columns. Rows an old binary filed under its
    /// path-derived tenant are renamed onto the stored one, so surviving
    /// config keeps working; keyed run ids minted before the migration map
    /// anew, so one in-flight retry may run again.
    pub(crate) fn dir_tenant(&self, state: &std::path::Path) -> Result<TenantId, ExecError> {
        if self.stored_dir_tenant()?.is_none() {
            let drawn = lgwks_std::random::bytes::<16>().map_err(ExecError::Entropy)?;
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO dir_tenant (id, tenant) VALUES (1, ?1)",
                    rusqlite::params![lgwks_std::hex::encode(
                        TenantId::from_bytes(drawn).as_bytes()
                    )],
                )
                .map_err(ExecError::Sql)?;
        }
        let pinned = self.stored_dir_tenant()?.ok_or(ExecError::KeyMissing)?;
        self.rekey_legacy(state, &lgwks_std::hex::encode(pinned.as_bytes()))?;
        Ok(pinned)
    }

    /// Renames rows an old binary filed under its path-derived tenant onto the
    /// stored one: policies, roots and routes keep working after the move.
    /// Idempotent: only rows under the legacy spelling move, and a replay
    /// moves nothing. Part C extends this to reports and live groups.
    fn rekey_legacy(&self, state: &std::path::Path, pinned: &str) -> Result<(), ExecError> {
        let legacy = lgwks_std::hex::encode(legacy_tenant_of(state).as_bytes());
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
        row.map(|text| {
            let mut bytes = [0_u8; 16];
            lgwks_std::hex::decode_into(&text, &mut bytes).map_err(ExecError::BadTenantId)?;
            Ok(TenantId::from_bytes(bytes))
        })
        .transpose()
    }

    /// Renames step 9's placeholder tenant to the resolved one: the carried key
    /// row joins the tenant that owns it. Runs on every open; only placeholder
    /// rows move, so a replay renames nothing. Reports and live groups keep
    /// their shape in part B; their tenant rename lands in part C.
    pub(crate) fn backfill_tenant(&self, tenant: &str) -> Result<(), ExecError> {
        self.conn
            .execute(
                "UPDATE receipt_keys SET tenant = ?1 WHERE tenant = ?2",
                rusqlite::params![tenant, UNCLAIMED],
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
                let tenant = named_tenant(&name)?;
                Ok((tenant, lgwks_std::hex::encode(tenant.as_bytes())))
            }
            None => {
                let tenant = self.dir_tenant(state)?;
                Ok((tenant, lgwks_std::hex::encode(tenant.as_bytes())))
            }
        }
    }

    /// The receipt signing key this tenant seals under: the active (newest) row,
    /// or a fresh key id 1 minted from OS entropy. Only a run path calls this;
    /// reads use [`Ledger::active_receipt_key`], which never mints.
    pub(crate) fn signing_key_for(&self, tenant: &str) -> Result<(i64, SecretKey), ExecError> {
        if let Some(active) = self.active_row(tenant)? {
            return Ok(active);
        }
        let seed = lgwks_std::random::bytes::<32>().map_err(ExecError::Entropy)?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO receipt_keys (tenant, key_id, seed) VALUES (?1, 1, ?2)",
                rusqlite::params![tenant, seed.as_slice()],
            )
            .map_err(ExecError::Sql)?;
        self.active_row(tenant)?.ok_or(ExecError::KeyMissing)
    }

    /// The active receipt key id and public key, if this tenant holds one; a
    /// tenant with no key yet reads as none, and no seed is drawn (larry#211).
    pub(crate) fn active_receipt_key(
        &self,
        tenant: &str,
    ) -> Result<Option<(i64, PublicKey)>, ExecError> {
        Ok(self
            .active_row(tenant)?
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

    /// The active (newest) receipt key row, if this tenant holds one; a row
    /// whose seed is not 32 bytes is damage.
    fn active_row(&self, tenant: &str) -> Result<Option<(i64, SecretKey)>, ExecError> {
        self.conn
            .query_row(
                "SELECT key_id, seed FROM receipt_keys WHERE tenant = ?1 ORDER BY key_id DESC LIMIT 1",
                rusqlite::params![tenant],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(ExecError::Sql)?
            .map(|(id, stored)| {
                <[u8; 32]>::try_from(stored.as_slice())
                    .map(SecretKey::from_seed)
                    .map(|secret| (id, secret))
                    .map_err(ExecError::KeySeedLength)
            })
            .transpose()
    }
}

/// Seeded adversarial simulations of the tenant store: ids, keys and rotation.
#[cfg(test)]
#[path = "sim_tenant_keys.rs"]
mod sim;
