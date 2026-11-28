//! The TUI session rows of the executor ledger (larry#256): one row per
//! terminal session in `tui_sessions`, holding its tenant, id, name, folder,
//! model, update stamp and the transcript snapshot as JSON. The ledger owns the
//! SQL and the row shape; `crate::tui::sessions` owns the snapshot shape and the
//! name rule. Every write names the exact tenant and row it touches, and
//! creating past a tenant's bound writes no row.

use lgwks_deps::storefront::rusqlite;
use lgwks_macros::Error;

use super::ExecError;

/// Schema of the TUI session table, migration step 12 (larry#256): one row per
/// terminal session, keyed by tenant and id so no tenant reads or writes
/// another's. The transcript column holds the snapshot JSON
/// `crate::tui::sessions` writes, bounded there before it is stored.
pub(super) const SESSIONS_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS tui_sessions(tenant TEXT NOT NULL, \
    id TEXT NOT NULL, name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 64), folder TEXT NOT NULL, \
    model TEXT NOT NULL, updated INTEGER NOT NULL, transcript TEXT NOT NULL, PRIMARY KEY (tenant, id));";

/// Why a TUI session row was refused.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SessionStoreError {
    /// No session holds `id`.
    #[error("unknown tui session {id}")]
    NoSuchSession {
        /// The id that names no row.
        id: String,
    },
    /// `name` is blank, past 64 characters or holds a control character.
    #[error("bad tui session name {name}")]
    BadName {
        /// The refused text.
        name: String,
    },
    /// The tenant already holds its `bound` of sessions.
    #[error("the tui session store is full at {bound} sessions")]
    Full {
        /// Sessions allowed.
        bound: u32,
    },
    /// The snapshot for `id` is past its byte bound.
    #[error("tui session {id} is past its byte bound with {bytes} bytes")]
    TooLarge {
        /// The session refused.
        id: String,
        /// The snapshot's bytes.
        bytes: usize,
    },
    /// A snapshot could not be encoded as JSON.
    #[error("a tui session snapshot could not be encoded")]
    Encode(
        /// The JSON refusal.
        #[source]
        lgwks_std::json::Error,
    ),
}

/// One session row to create.
pub(crate) struct NewSession<'columns> {
    /// The tenant the session belongs to, as every ledger row spells it.
    pub(crate) tenant: &'columns str,
    /// The session's id, a UUID version 7.
    pub(crate) id: &'columns str,
    /// The person's name for it, 1 to 64 characters, no control character.
    pub(crate) name: &'columns str,
    /// The folder the session runs in.
    pub(crate) folder: &'columns str,
    /// The model the session goes to.
    pub(crate) model: &'columns str,
    /// Milliseconds since the Unix epoch of the row's write.
    pub(crate) updated: i64,
    /// The transcript snapshot as JSON.
    pub(crate) transcript: &'columns str,
    /// Most rows the tenant holds; creating past it writes no row.
    pub(crate) bound: u32,
}

impl super::Ledger {
    /// Creates the session row `new`, unless its tenant already holds
    /// `new.bound` rows: that writes no row and reports `false`, in one
    /// statement, so two creators racing at the bound never both land
    /// (larry#397's shape). A duplicate id writes no row either: the first
    /// record under one id is the session, so a retried create never replaces
    /// it.
    pub(crate) fn session_create(&self, new: &NewSession<'_>) -> Result<bool, ExecError> {
        let allowed = i64::from(new.bound);
        let written = self
            .conn
            .execute(
                "INSERT INTO tui_sessions (tenant, id, name, folder, model, updated, transcript) \
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 \
                 WHERE (SELECT COUNT(*) FROM tui_sessions WHERE tenant = ?1) < ?8 \
                 ON CONFLICT (tenant, id) DO NOTHING",
                rusqlite::params![
                    new.tenant,
                    new.id,
                    new.name,
                    new.folder,
                    new.model,
                    new.updated,
                    new.transcript,
                    allowed
                ],
            )
            .map_err(ExecError::Sql)?;
        Ok(written > 0)
    }

    /// Holds `transcript` in `tenant`'s session `id`; `false` when it holds none.
    pub(crate) fn session_save(
        &self,
        tenant: &str,
        id: &str,
        updated: i64,
        transcript: &str,
    ) -> Result<bool, ExecError> {
        let changed = self
            .conn
            .execute(
                "UPDATE tui_sessions SET updated = ?3, transcript = ?4 \
                 WHERE tenant = ?1 AND id = ?2",
                rusqlite::params![tenant, id, updated, transcript],
            )
            .map_err(ExecError::Sql)?;
        Ok(changed > 0)
    }
}

/// The ledger contract for the TUI session rows (larry#256).
#[cfg(test)]
mod tests {
    use super::{NewSession, SessionStoreError};
    use crate::executor::Ledger;
    use crate::sim_error::SimError;

    /// A row of `tenant` to create, bounded at two rows a tenant.
    fn row<'slots>(tenant: &'slots str, id: &'slots str, updated: i64) -> NewSession<'slots> {
        NewSession {
            tenant,
            id,
            name: "one",
            folder: "/work",
            model: "mock",
            updated,
            transcript: "{}",
            bound: 2,
        }
    }

    /// The name, update stamp and snapshot of `tenant`'s row `id`.
    fn stored(ledger: &Ledger, tenant: &str, id: &str) -> Result<(String, i64, String), SimError> {
        Ok(ledger.conn.query_row(
            "SELECT name, updated, transcript FROM tui_sessions WHERE tenant = ?1 AND id = ?2",
            [tenant, id],
            |found| Ok((found.get(0)?, found.get(1)?, found.get(2)?)),
        )?)
    }

    /// Rows are created and saved; a create past a tenant's bound writes no row
    /// while another tenant still creates, a duplicate id keeps the first
    /// record, and a save of no row, or of another tenant's row, says so. The
    /// directory is fresh (larry#267).
    #[test]
    fn sessions_create_save_and_stop_at_the_bound() -> Result<(), SimError> {
        let directory = crate::executor::sim_tree::scratch("larry-tui-sessions")?;
        let ledger = Ledger::open(&directory)?;
        assert!(ledger.session_create(&row("t", "a", 1))?);
        assert!(
            !ledger.session_create(&row("t", "a", 4))?,
            "a duplicate id keeps the first record"
        );
        assert!(ledger.session_create(&row("t", "b", 2))?);
        assert!(
            !ledger.session_create(&row("t", "c", 3))?,
            "past the bound no row is written"
        );
        assert!(
            ledger.session_create(&row("u", "a", 5))?,
            "one tenant's bound is not another's"
        );
        let held: i64 =
            ledger
                .conn
                .query_row("SELECT COUNT(*) FROM tui_sessions", [], |found| {
                    found.get(0)
                })?;
        assert_eq!(held, 3, "the refused creates wrote nothing");
        let first = (String::from("one"), 1, String::from("{}"));
        assert_eq!(stored(&ledger, "t", "a")?, first);
        assert!(
            !ledger.session_save("u", "b", 6, "{}")?,
            "another tenant's row is not there to write"
        );
        assert!(ledger.session_save("t", "a", 6, r#"{"kept":true}"#)?);
        assert_eq!(
            stored(&ledger, "t", "a")?,
            (String::from("one"), 6, String::from(r#"{"kept":true}"#))
        );
        assert_eq!(
            (stored(&ledger, "t", "b")?.2, stored(&ledger, "u", "a")?.1),
            (String::from("{}"), 5),
            "a save touches its row alone"
        );
        Ok(())
    }

    /// The error for a refused row names what was refused.
    #[test]
    fn session_store_errors_name_what_they_refuse() {
        let unknown = SessionStoreError::NoSuchSession {
            id: String::from("x"),
        };
        assert_eq!(unknown.to_string(), "unknown tui session x");
        let full = SessionStoreError::Full { bound: 2 };
        assert_eq!(
            full.to_string(),
            "the tui session store is full at 2 sessions"
        );
    }
}
