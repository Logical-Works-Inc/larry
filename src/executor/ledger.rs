//! The ledger of process groups a run has started and not yet settled, in
//! `<state>/executor.db` (larry#85).

use std::path::Path;

use lgwks_deps::storefront::rusqlite;
use lgwks_std::process::{ProcessIdentity, reap_orphaned_group};

use super::ExecError;

/// Schema of the live-group table, version 1. `IF NOT EXISTS` because two executors
/// starting at once on one state directory both read `user_version` 0 and both run the
/// step: `lgwks_deps::sqlite::migrate` reads the version and applies the step outside one
/// transaction (logicalworks-crates#872).
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS live_groups(task_id TEXT NOT NULL, pid INTEGER NOT NULL, \
    started TEXT NOT NULL, PRIMARY KEY(pid, started));";

/// The sqlite table of live process groups, keyed by pid and start time.
#[non_exhaustive]
pub(crate) struct Ledger {
    /// The open connection to `executor.db`.
    conn: rusqlite::Connection,
}

impl Ledger {
    /// Opens, creating if needed, the ledger under the state directory.
    pub(crate) fn open(state_dir: &Path) -> Result<Ledger, ExecError> {
        let conn =
            lgwks_deps::sqlite::open(&state_dir.join("executor.db")).map_err(ExecError::Ledger)?;
        lgwks_deps::sqlite::migrate(&conn, 1, &[SCHEMA]).map_err(ExecError::Ledger)?;
        Ok(Ledger { conn })
    }

    /// Records a group that a task has started.
    pub(crate) fn insert(
        &self,
        task_id: &str,
        identity: &ProcessIdentity,
    ) -> Result<(), ExecError> {
        self.conn
            .execute(
                "INSERT INTO live_groups (task_id, pid, started) VALUES (?1, ?2, ?3)",
                rusqlite::params![task_id, identity.pid(), identity.started()],
            )
            .map_err(ExecError::Sql)?;
        Ok(())
    }

    /// Removes a group that has settled.
    pub(crate) fn remove(&self, identity: &ProcessIdentity) -> Result<(), ExecError> {
        self.forget(i64::from(identity.pid()), identity.started())
    }

    /// Settles every recorded group: a damaged row is dropped, a live group is reaped and
    /// its row dropped. Returns how many rows were dropped.
    pub(crate) fn reap(&self) -> Result<usize, ExecError> {
        let rows = self.rows()?;
        let mut settled = 0_usize;
        for (pid, started) in rows {
            // A row no identity can be rebuilt from was damaged or written by something
            // else: reaping against it would be a guess, so it is dropped unsignalled.
            let identity = match i32::try_from(pid) {
                Ok(narrow_pid) => match ProcessIdentity::new(narrow_pid, &started) {
                    Ok(identity) => identity,
                    Err(_damaged) => {
                        self.forget(pid, &started)?;
                        settled = settled.saturating_add(1);
                        continue;
                    }
                },
                Err(_out_of_range) => {
                    self.forget(pid, &started)?;
                    settled = settled.saturating_add(1);
                    continue;
                }
            };
            let _outcome = reap_orphaned_group(&identity).map_err(ExecError::Reap)?;
            self.forget(pid, &started)?;
            settled = settled.saturating_add(1);
        }
        Ok(settled)
    }

    /// The task ids that still hold a recorded group.
    pub(crate) fn live_tasks(&self) -> Result<Vec<String>, ExecError> {
        let mut statement = self
            .conn
            .prepare("SELECT DISTINCT task_id FROM live_groups ORDER BY task_id")
            .map_err(ExecError::Sql)?;
        let mapped = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(ExecError::Sql)?;
        mapped
            .collect::<Result<Vec<String>, rusqlite::Error>>()
            .map_err(ExecError::Sql)
    }

    /// Every recorded (pid, start time) pair.
    fn rows(&self) -> Result<Vec<(i64, String)>, ExecError> {
        let mut statement = self
            .conn
            .prepare("SELECT pid, started FROM live_groups")
            .map_err(ExecError::Sql)?;
        let mapped = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(ExecError::Sql)?;
        mapped
            .collect::<Result<Vec<(i64, String)>, rusqlite::Error>>()
            .map_err(ExecError::Sql)
    }

    /// Deletes the row for one (pid, start time) pair.
    fn forget(&self, pid: i64, started: &str) -> Result<(), ExecError> {
        self.conn
            .execute(
                "DELETE FROM live_groups WHERE pid = ?1 AND started = ?2",
                rusqlite::params![pid, started],
            )
            .map_err(ExecError::Sql)?;
        Ok(())
    }
}

/// Ledger tests: a dead leader and a damaged row are both settled.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use lgwks_deps::storefront::rusqlite;
    use lgwks_std::process::ProcessIdentity;

    use super::Ledger;

    /// A pid no live process holds: the largest positive `i32` less one.
    const DEAD_PID: i32 = 2_147_483_646;

    /// Per-process counter that gives each test its own state directory.
    static DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A fresh scratch directory under the system temp dir.
    fn scratch_directory() -> PathBuf {
        let index = DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("larry-ledger-{}-{index}", std::process::id()))
    }

    /// The start-time string this platform's process identity reads: the kinfo form on macOS
    /// (the proc form is Linux-only and is refused elsewhere as unsupported).
    const PLATFORM_START: &str = if cfg!(target_os = "linux") {
        "proc:x:1"
    } else {
        "kinfo:1.000000"
    };

    /// A dead leader's row and a damaged row are settled by reap, leaving the ledger empty.
    #[test]
    fn sim_ledger_reaps_a_dead_leader_and_a_damaged_row() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = scratch_directory();
        let ledger = Ledger::open(&directory)?;
        let dead = ProcessIdentity::new(DEAD_PID, PLATFORM_START)?;
        ledger.insert("task-a", &dead)?;
        assert_eq!(
            ledger.live_tasks()?,
            vec!["task-a".to_owned()],
            "one live task is listed after insert"
        );
        ledger.remove(&dead)?;
        assert!(
            ledger.live_tasks()?.is_empty(),
            "removal empties the ledger"
        );
        ledger.insert("task-b", &dead)?;
        ledger.conn.execute(
            "INSERT INTO live_groups (task_id, pid, started) VALUES (?1, ?2, ?3)",
            rusqlite::params!["task-c", DEAD_PID, "zzz"],
        )?;
        assert_eq!(
            ledger.reap()?,
            2,
            "the dead-leader row and the damaged row are both settled"
        );
        assert!(ledger.live_tasks()?.is_empty(), "reap leaves no row behind");
        drop(std::fs::remove_dir_all(&directory));
        Ok(())
    }
}
