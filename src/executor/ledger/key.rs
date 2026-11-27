//! Key custody: the state directory's one receipt seed and the tenant key
//! digests. The live groups, reports, routes and policies live in the parent
//! module; this one only issues, counts and forgets keys (larry#86, larry#89,
//! larry#397).

use lgwks_deps::storefront::rusqlite::{self, OptionalExtension};
use lgwks_std::sign::SecretKey;

use super::ExecError;
use super::Ledger;

impl Ledger {
    /// The receipt signing key, created from OS entropy on first use.
    ///
    /// Executors racing on a fresh store each draw a seed, and `INSERT OR IGNORE`
    /// into the one-row table keeps the first; every racer then reads that row back,
    /// so one state directory signs under one key.
    pub(crate) fn signing_key(&self) -> Result<SecretKey, ExecError> {
        if let Some(key) = self.stored_key()? {
            return Ok(key);
        }
        let seed = lgwks_std::random::bytes::<32>().map_err(ExecError::Entropy)?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO receipt_key (id, seed) VALUES (1, ?1)",
                rusqlite::params![seed.as_slice()],
            )
            .map_err(ExecError::Sql)?;
        self.stored_key()?.ok_or(ExecError::KeyMissing)
    }

    /// The stored key, if a row exists; a row whose seed is not 32 bytes is damage.
    fn stored_key(&self) -> Result<Option<SecretKey>, ExecError> {
        let row = self
            .conn
            .query_row("SELECT seed FROM receipt_key WHERE id = 1", [], |row| {
                row.get::<_, Vec<u8>>(0)
            })
            .optional()
            .map_err(ExecError::Sql)?;
        let Some(stored) = row else {
            return Ok(None);
        };
        let seed = <[u8; 32]>::try_from(stored.as_slice()).map_err(ExecError::KeySeedLength)?;
        Ok(Some(SecretKey::from_seed(seed)))
    }

    /// Stores `digest` as `tenant`'s one key digest, replacing an earlier one, so
    /// issuing again rotates the key. A tenant with no key yet is stored only while
    /// the store holds fewer than `cap` keys, checked in the same statement, so two
    /// issuers racing at the cap never both land (larry#397). `false` when nothing
    /// was written because the store is full.
    pub(crate) fn set_tenant_key(
        &self,
        tenant: &str,
        digest: &str,
        cap: u32,
    ) -> Result<bool, ExecError> {
        let written = self
            .conn
            .execute(
                "INSERT INTO tenant_keys (tenant, digest) SELECT ?1, ?2 \
                 WHERE EXISTS (SELECT 1 FROM tenant_keys WHERE tenant = ?1) \
                 OR (SELECT COUNT(*) FROM tenant_keys) < ?3 \
                 ON CONFLICT (tenant) DO UPDATE SET digest = excluded.digest",
                rusqlite::params![tenant, digest, i64::from(cap)],
            )
            .map_err(ExecError::Sql)?;
        Ok(written > 0)
    }

    /// Every (tenant, digest hex) pair, at most `limit` of them.
    pub(crate) fn tenant_keys(&self, limit: u32) -> Result<Vec<(String, String)>, ExecError> {
        let bound = i64::from(limit);
        let mut statement = self
            .conn
            .prepare("SELECT tenant, digest FROM tenant_keys ORDER BY tenant LIMIT ?1")
            .map_err(ExecError::Sql)?;
        let mapped = statement
            .query_map([bound], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(ExecError::Sql)?;
        mapped
            .collect::<Result<Vec<(String, String)>, rusqlite::Error>>()
            .map_err(ExecError::Sql)
    }

    /// How many tenant keys the store holds.
    pub(crate) fn tenant_key_count(&self) -> Result<i64, ExecError> {
        self.conn
            .query_row("SELECT COUNT(*) FROM tenant_keys", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(ExecError::Sql)
    }

    /// Deletes `tenant`'s key digest, so its key is refused from the next reload;
    /// `false` when the tenant held no key.
    pub(crate) fn remove_tenant_key(&self, tenant: &str) -> Result<bool, ExecError> {
        let removed = self
            .conn
            .execute(
                "DELETE FROM tenant_keys WHERE tenant = ?1",
                rusqlite::params![tenant],
            )
            .map_err(ExecError::Sql)?;
        Ok(removed > 0)
    }
}
