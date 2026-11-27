//! The TUI session rows of the executor ledger (larry#256): one row per
//! terminal session in `tui_sessions`, holding its id, name, folder, model,
//! update stamp and two JSON columns (the transcript snapshot and the receipts
//! the snapshot already carries). The ledger owns the SQL and the row shape;
//! `crate::tui::sessions` owns the snapshot shape, the name rule, the Markdown
//! and the recap. Every statement names the exact row or folder it touches, so
//! two folders sharing one store never read, rename or save each other's rows,
//! and creating past the bound writes no row.

use lgwks_deps::storefront::rusqlite;
use lgwks_macros::Error;

use super::ExecError;

/// Schema of the TUI session table, version 10 (larry#256): one row per
/// terminal session. The transcript column holds the snapshot JSON
/// `crate::tui::sessions` writes, bounded there before it is stored.
pub(super) const SESSIONS_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS tui_sessions(id TEXT PRIMARY KEY, \
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 64), folder TEXT NOT NULL, model TEXT NOT NULL, \
    updated INTEGER NOT NULL, transcript TEXT NOT NULL);";

/// Index of the TUI session table, version 11 (larry#256): a folder's listing
/// reads newest first without scanning another folder's rows.
pub(super) const SESSIONS_INDEX_SCHEMA: &str =
    "CREATE INDEX IF NOT EXISTS idx_tui_sessions_folder ON tui_sessions(folder, updated DESC);";

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
    /// The store holds `count` sessions, past its bound of `bound`.
    #[error("the tui session store holds {count} sessions, past its bound of {bound}")]
    Full {
        /// Sessions stored.
        count: usize,
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
    /// The snapshot for `id` is not JSON the store reads.
    #[error("tui session {id} holds bytes that are not a snapshot")]
    Damaged {
        /// The session refused.
        id: String,
        /// The JSON refusal.
        #[source]
        cause: lgwks_std::json::Error,
    },
    /// A snapshot could not be encoded as JSON.
    #[error("a tui session snapshot could not be encoded")]
    Encode(
        /// The JSON refusal.
        #[source]
        lgwks_std::json::Error,
    ),
}

/// One listed session: its id, name and update stamp, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct SessionHead {
    /// The session's id.
    pub(crate) id: String,
    /// The session's name.
    pub(crate) name: String,
    /// Milliseconds since the Unix epoch of its last save, rename or fork.
    pub(crate) updated: i64,
}

/// One stored session: its head, folder, model and raw snapshot JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct SessionRecord {
    /// The session's id, name and update stamp.
    pub(crate) head: SessionHead,
    /// The folder the session runs in.
    pub(crate) folder: String,
    /// The model the session goes to, or why there is none.
    pub(crate) model: String,
    /// The transcript snapshot as JSON.
    pub(crate) transcript: String,
}

/// One session row to create: every column but the reserved receipts column,
/// which is always empty because the snapshot already carries the receipts.
pub(crate) struct NewSession<'columns> {
    /// The session's id, a UUID version 7.
    pub(crate) id: &'columns str,
    /// The person's name for it, 1 to 64 characters, no control character.
    pub(crate) name: &'columns str,
    /// The folder new rows are listed under.
    pub(crate) folder: &'columns str,
    /// The model new rows name.
    pub(crate) model: &'columns str,
    /// Milliseconds since the Unix epoch of the row's write.
    pub(crate) updated: i64,
    /// The transcript snapshot as JSON.
    pub(crate) transcript: &'columns str,
    /// Most rows the store holds; creating past it writes no row.
    pub(crate) bound: u32,
}

impl super::Ledger {
    /// Creates the session row `new`, unless the store already holds
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
                "INSERT INTO tui_sessions (id, name, folder, model, updated, transcript) \
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6 \
                 WHERE (SELECT COUNT(*) FROM tui_sessions) < ?7 \
                 ON CONFLICT (id) DO NOTHING",
                rusqlite::params![
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

    /// How many session rows the store holds.
    pub(crate) fn session_count(&self) -> Result<i64, ExecError> {
        self.conn
            .query_row("SELECT COUNT(*) FROM tui_sessions", [], |row| row.get(0))
            .map_err(ExecError::Sql)
    }

    /// The sessions saved for `folder`, newest first, at most `limit`.
    pub(crate) fn session_list(
        &self,
        folder: &str,
        limit: i64,
    ) -> Result<Vec<SessionHead>, ExecError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, name, updated FROM tui_sessions WHERE folder = ?1 \
                 ORDER BY updated DESC, rowid DESC LIMIT ?2",
            )
            .map_err(ExecError::Sql)?;
        let mapped = statement
            .query_map(rusqlite::params![folder, limit], |row| {
                Ok(SessionHead {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    updated: row.get(2)?,
                })
            })
            .map_err(ExecError::Sql)?;
        mapped
            .collect::<Result<Vec<SessionHead>, rusqlite::Error>>()
            .map_err(ExecError::Sql)
    }

    /// The session row `id` holds, or `None`.
    pub(crate) fn session_load(&self, id: &str) -> Result<Option<SessionRecord>, ExecError> {
        self.conn
            .query_row(
                "SELECT id, name, updated, folder, model, transcript \
                 FROM tui_sessions WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(SessionRecord {
                        head: SessionHead {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            updated: row.get(2)?,
                        },
                        folder: row.get(3)?,
                        model: row.get(4)?,
                        transcript: row.get(5)?,
                    })
                },
            )
            .map(Some)
            .or_else(|cause| match cause {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(ExecError::Sql(cause)),
            })
    }

    /// Names the session `id` holds `name`; `false` when no row holds the id.
    pub(crate) fn session_rename(
        &self,
        id: &str,
        name: &str,
        updated: i64,
    ) -> Result<bool, ExecError> {
        let changed = self
            .conn
            .execute(
                "UPDATE tui_sessions SET name = ?2, updated = ?3 WHERE id = ?1",
                rusqlite::params![id, name, updated],
            )
            .map_err(ExecError::Sql)?;
        Ok(changed > 0)
    }

    /// Holds `transcript` in the session `id`; `false` when no row holds it.
    pub(crate) fn session_save(
        &self,
        id: &str,
        updated: i64,
        transcript: &str,
    ) -> Result<bool, ExecError> {
        let changed = self
            .conn
            .execute(
                "UPDATE tui_sessions SET updated = ?2, transcript = ?3 WHERE id = ?1",
                rusqlite::params![id, updated, transcript],
            )
            .map_err(ExecError::Sql)?;
        Ok(changed > 0)
    }

    /// Copies `source`'s bytes into the new row `id` named `name`; the source
    /// row is untouched, so a fork diverges and the original is unchanged.
    /// Like creation, it writes no row past `bound` and reports `false`.
    pub(crate) fn session_fork(
        &self,
        id: &str,
        name: &str,
        updated: i64,
        source: &SessionRecord,
        bound: u32,
    ) -> Result<bool, ExecError> {
        self.session_create(&NewSession {
            id,
            name,
            folder: &source.folder,
            model: &source.model,
            updated,
            transcript: &source.transcript,
            bound,
        })
    }
}

/// The seeded ledger contract for the TUI session rows (larry#256).
#[cfg(test)]
mod tests {
    use super::{NewSession, SessionStoreError};
    use crate::executor::Ledger;

    /// A fresh state directory: empty when made and removed when its guard
    /// drops, so a process id the system reuses never lands a test in a
    /// stale tree (larry#267).
    fn scratch_directory() -> Result<crate::executor::sim_tree::Scratch, crate::sim_error::SimError>
    {
        crate::executor::sim_tree::scratch("larry-tui-sessions")
    }

    /// Rows are created, listed per folder, loaded whole, renamed, saved
    /// and forked; a second create past the bound writes no row, and a
    /// duplicate id keeps the first record.
    #[test]
    fn sessions_create_list_rename_save_and_fork() -> Result<(), Box<dyn std::error::Error>> {
        let directory = scratch_directory()?;
        let ledger = Ledger::open(&directory)?;
        fn row<'slots>(id: &'slots str, name: &'slots str, updated: i64) -> NewSession<'slots> {
            NewSession {
                id,
                name,
                folder: "/work",
                model: "mock",
                updated,
                transcript: "{}",
                bound: 2,
            }
        }
        assert!(ledger.session_create(&row("a", "one", 1))?);
        assert!(ledger.session_create(&row("b", "two", 2))?);
        assert!(
            !ledger.session_create(&row("c", "three", 3))?,
            "past the bound no row is written"
        );
        assert_eq!(
            ledger.session_count()?,
            2,
            "the refused create wrote nothing"
        );
        assert!(
            !ledger.session_create(&row("a", "uno", 4))?,
            "a duplicate id keeps the first record"
        );
        let listed = ledger.session_list("/work", 10_000)?;
        assert_eq!(
            listed
                .iter()
                .map(|head| head.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"],
            "newest first: {listed:?}"
        );
        assert!(
            ledger.session_list("/elsewhere", 10_000)?.is_empty(),
            "another folder lists nothing"
        );
        assert!(ledger.session_rename("a", "uno", 5)?);
        assert!(!ledger.session_rename("missing", "x", 5)?, "no row, no lie");
        let record = ledger.session_load("a")?.ok_or("the renamed row loads")?;
        assert_eq!((record.head.name.as_str(), record.head.updated), ("uno", 5));
        assert!(ledger.session_save("a", 6, r#"{"kept":true}"#)?);
        assert!(!ledger.session_save("missing", 6, "{}")?, "no row, no lie");
        assert!(
            ledger.session_load("missing")?.is_none(),
            "missing loads as none"
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
        let full = SessionStoreError::Full { count: 3, bound: 2 };
        assert!(full.to_string().contains("past its bound of 2"), "{full}");
    }

    /// Ten thousand sessions list newest-first with a measured p50/p95/p99
    /// and peak RSS: the proof the store stays a scan of a bounded table
    /// (larry#256). The figures are MEASURED on the box that runs them,
    /// printed the way the replay-cost sim prints its own (larry#74): never
    /// a pass condition beyond every sample listing every row.
    #[test]
    fn sessions_many_rows_list_reports_latency_and_rss() -> Result<(), Box<dyn std::error::Error>> {
        use std::fmt::Write as _;

        /// Rows listed: 10,000, the MAX_SESSIONS bound the sessions layer
        /// holds new rows to, so the measurement proves that bound's
        /// listing.
        const ROWS: u32 = 10_000;
        let directory = scratch_directory()?;
        let ledger = Ledger::open(&directory)?;
        for index in 0..i64::from(ROWS) {
            let id = format!("sim-{index:05}");
            let name = format!("session {index}");
            assert!(
                ledger.session_create(&NewSession {
                    id: &id,
                    name: &name,
                    folder: "/work",
                    model: "mock",
                    updated: index,
                    transcript: "{}",
                    bound: ROWS,
                })?,
                "row {index} is stored"
            );
        }
        let mut samples: Vec<std::time::Duration> = Vec::new();
        for _ in 0..64_u32 {
            let started = std::time::Instant::now();
            let rows = ledger.session_list("/work", i64::from(ROWS))?;
            samples.push(started.elapsed());
            assert_eq!(rows.len(), 10_000, "every sample lists every row");
            assert_eq!(
                (
                    rows.first().map(|head| head.name.as_str()),
                    rows.last().map(|head| head.name.as_str())
                ),
                (Some("session 9999"), Some("session 0")),
                "newest first"
            );
        }
        samples.sort();
        let rank = |pct: usize| -> Result<f64, &'static str> {
            let at = samples
                .len()
                .saturating_mul(pct)
                .div_ceil(100)
                .saturating_sub(1);
            let sample = samples.get(at).ok_or("no list samples")?;
            Ok(sample.as_secs_f64() * 1000.0)
        };
        let pid = std::process::id().to_string();
        let rss = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid])
            .output()?;
        let rss_kib: u64 = String::from_utf8(rss.stdout)?.trim().parse()?;
        let mut figures = String::new();
        writeln!(
            figures,
            "{{\"scope\":\"ten thousand tui sessions, one folder\",\"rows\":10000,\"samples\":64,\
              \"list_p50_ms\":{:.3},\"list_p95_ms\":{:.3},\"list_p99_ms\":{:.3},\
              \"sampled_max_rss_kib\":{rss_kib}}}",
            rank(50)?,
            rank(95)?,
            rank(99)?,
        )?;
        std::io::Write::write_all(&mut std::io::stderr().lock(), figures.as_bytes())?;
        Ok(())
    }
}
