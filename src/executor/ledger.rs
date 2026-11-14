//! The ledger of process groups a run has started and not yet settled, the
//! receipt signing key and the tenant's model routes, in `<state>/executor.db`
//! (larry#85, larry#86, larry#201).

use std::fs::{File, TryLockError};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

use lgwks_deps::storefront::rusqlite::{self, OptionalExtension};
use lgwks_std::process::{ProcessIdentity, reap_orphaned_group};
use lgwks_std::sign::SecretKey;

use super::ExecError;
use super::run::await_group_gone;

/// The longest `open` waits for another executor's lock on the state directory (larry#214).
/// Past it the open is refused with [`ExecError::LedgerLockTimeout`], never a wait forever.
const LEDGER_LOCK_BOUND: Duration = Duration::from_secs(60);

/// Schema of the live-group table, version 1. `IF NOT EXISTS` because two executors
/// starting at once on one state directory both read `user_version` 0 and both run the
/// step: `lgwks_deps::sqlite::migrate` reads the version and applies the step outside one
/// transaction (logicalworks-crates#872).
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS live_groups(task_id TEXT NOT NULL, pid INTEGER NOT NULL, \
    started TEXT NOT NULL, PRIMARY KEY(pid, started));";

/// Schema of the receipt key table, version 2: at most one row, a 32-byte ed25519 seed.
const KEY_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS receipt_key(id INTEGER PRIMARY KEY CHECK (id = 1), \
    seed BLOB NOT NULL CHECK (length(seed) = 32));";

/// Schema of the tenant key table, version 3 (larry#89): one BLAKE3 digest per tenant,
/// as 64 hex characters; the key itself is shown once and never stored.
const TENANT_KEY_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS tenant_keys(tenant TEXT PRIMARY KEY, \
    digest TEXT NOT NULL CHECK (length(digest) = 64));";

/// Owner-only mode for the store and its write-ahead files: the seed is the key.
const OWNER_ONLY: u32 = 0o600;

/// Schema of the sealed-report table, version 4 (larry#182 defect 3): one row per
/// keyed run id, holding the report exactly as it was printed. A client retry
/// whose key derives that id is answered from this row, so the body never runs a
/// second time. The row is the run's own evidence, so it is never overwritten by
/// a later run of the same id.
const REPORT_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS reports(task_id TEXT PRIMARY KEY, \
    task_digest TEXT NOT NULL, report BLOB NOT NULL);";

/// Schema of the route table, version 5 (larry#201): the routes a tenant owns, at
/// most 256 each, with one default per tenant. A route holds the NAME of the
/// variable its key is read from, never the key. `temperature` and `residency`
/// are NULL when the route pins none.
const ROUTE_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS routes(tenant TEXT NOT NULL, id TEXT NOT NULL, \
    kind TEXT NOT NULL, base_url TEXT NOT NULL, key_env TEXT NOT NULL, model TEXT NOT NULL, \
    temperature REAL, request_bytes INTEGER NOT NULL, residency TEXT, describe TEXT NOT NULL, \
    is_default INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(tenant, id));";

/// The store's file names under the state directory, the database first.
const STORE_FILES: [&str; 3] = ["executor.db", "executor.db-wal", "executor.db-shm"];

/// The sqlite table of live process groups, keyed by pid and start time.
#[non_exhaustive]
pub(crate) struct Ledger {
    /// The open connection to `executor.db`.
    conn: rusqlite::Connection,
}

impl Ledger {
    /// Opens, creating if needed, the ledger under the state directory.
    ///
    /// Open and migration run under an exclusive lock on `executor.db.lock`, so
    /// executors starting together on one state directory open the store one at a
    /// time. Without it the second of two first opens was refused at once with
    /// "database is locked": `lgwks_deps::sqlite::open` switches a fresh file to
    /// WAL before it sets its busy timeout (logicalworks-crates#873), and
    /// `migrate` reads and writes `user_version` outside one transaction (#872).
    /// The kernel releases the lock when its holder dies, so a `kill -9` mid-open
    /// leaves nothing to clear; a live holder keeps it for one open and one
    /// migration, each under the store's own 60 s busy bound.
    ///
    /// The store holds the receipt key, so it is created owner-only before SQLite
    /// opens it (SQLite gives the write-ahead files the database's mode), and a
    /// store left wider by an earlier release is narrowed on every open.
    pub(crate) fn open(state_dir: &Path) -> Result<Ledger, ExecError> {
        Self::open_within(state_dir, LEDGER_LOCK_BOUND)
    }

    /// [`Ledger::open`] with the lock wait bounded by `bound` (larry#214): a held lock
    /// is tried again, yielding between tries, until `bound` has elapsed, then refused.
    ///
    /// # Errors
    ///
    /// [`ExecError::LedgerLockTimeout`] when another executor holds the lock for the
    /// whole bound; the other ledger errors as [`Ledger::open`] documents.
    fn open_within(state_dir: &Path, bound: Duration) -> Result<Ledger, ExecError> {
        std::fs::create_dir_all(state_dir).map_err(ExecError::LedgerLock)?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(state_dir.join("executor.db.lock"))
            .map_err(ExecError::LedgerLock)?;
        lock_within(&lock, bound)?;
        let database = state_dir.join("executor.db");
        File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(OWNER_ONLY)
            .open(&database)
            .map_err(ExecError::KeyStore)?;
        let conn = lgwks_deps::sqlite::open(&database).map_err(ExecError::Ledger)?;
        lgwks_deps::sqlite::migrate(
            &conn,
            5,
            &[
                SCHEMA,
                KEY_SCHEMA,
                TENANT_KEY_SCHEMA,
                REPORT_SCHEMA,
                ROUTE_SCHEMA,
            ],
        )
        .map_err(ExecError::Ledger)?;
        for name in STORE_FILES {
            match std::fs::set_permissions(
                state_dir.join(name),
                std::fs::Permissions::from_mode(OWNER_ONLY),
            ) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(ExecError::KeyStore(error)),
            }
        }
        lock.unlock().map_err(ExecError::LedgerLock)?;
        Ok(Ledger { conn })
    }

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
    /// issuing again rotates the key.
    pub(crate) fn set_tenant_key(&self, tenant: &str, digest: &str) -> Result<(), ExecError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO tenant_keys (tenant, digest) VALUES (?1, ?2)",
                rusqlite::params![tenant, digest],
            )
            .map_err(ExecError::Sql)?;
        Ok(())
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

    /// The sealed report `task_id` already recorded, if this state directory holds
    /// one no longer than `limit` bytes. A keyed run id a retry derives reads its
    /// earlier run's report here, so the body is never dispatched twice (larry#182).
    ///
    /// # Errors
    ///
    /// [`ExecError::Sql`] when the statement fails, and [`ExecError::Receipt`] when
    /// the recorded bytes are longer than `limit`, refused rather than truncated.
    pub(crate) fn report(&self, task_id: &str, limit: usize) -> Result<Option<Vec<u8>>, ExecError> {
        let row = self
            .conn
            .query_row(
                "SELECT report FROM reports WHERE task_id = ?1",
                rusqlite::params![task_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(ExecError::Sql)?;
        match row {
            Some(recorded) if recorded.len() > limit => {
                Err(ExecError::Receipt(super::ReceiptError::TooLarge {
                    len: recorded.len(),
                }))
            }
            other => Ok(other),
        }
    }

    /// Records `report` as `task_id`'s evidence, unless that id already holds one:
    /// the first run's record is the run, and a later row under one id would be a
    /// second claim about one request (larry#182 defect 3).
    pub(crate) fn keep_report(
        &self,
        task_id: &str,
        task_digest: &str,
        report: &[u8],
    ) -> Result<(), ExecError> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO reports (task_id, task_digest, report) VALUES (?1, ?2, ?3)",
                rusqlite::params![task_id, task_digest, report],
            )
            .map_err(ExecError::Sql)?;
        Ok(())
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
            reap_orphaned_group(&identity).map_err(ExecError::Reap)?;
            // The row goes only once the group is gone (INVARIANTS): a group that
            // survives its bounded wait keeps its row, and the next start tries again.
            match await_group_gone(identity.pid()) {
                Ok(()) => {}
                Err(ExecError::GroupSurvived { .. }) => continue,
                Err(other) => return Err(other),
            }
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

/// Takes `lock` exclusively, trying again until `bound` has elapsed on a monotonic
/// clock, then answers [`ExecError::LedgerLockTimeout`].
///
/// # Errors
///
/// [`ExecError::LedgerLockTimeout`] when the lock stays held for `bound`, and
/// [`ExecError::LedgerLock`] when the lock itself fails.
fn lock_within(lock: &File, bound: Duration) -> Result<(), ExecError> {
    let started = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => {
                let waited = started.elapsed();
                if waited >= bound {
                    return Err(ExecError::LedgerLockTimeout { waited });
                }
                std::thread::yield_now();
            }
            Err(TryLockError::Error(cause)) => return Err(ExecError::LedgerLock(cause)),
        }
    }
}

/// The route table's reads and writes (larry#201), a child module so the
/// connection stays private to the ledger.
#[path = "ledger_routes.rs"]
mod routes;

/// Seeded adversarial simulations of the report rows a keyed retry answers from
/// (larry#182 defect 3).
#[cfg(test)]
#[path = "sim_ledger.rs"]
mod sim;

/// Ledger tests: a dead leader and a damaged row are both settled.
#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
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

    /// A group that survives its reap keeps its row, so the next start tries again. The
    /// row's identity names a start the live group does not carry, so the reap reads it
    /// as reused and signals nothing; the group stays up and its row stays with it (larry#214).
    #[test]
    fn a_surviving_group_keeps_its_row() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::process::CommandExt;
        let directory = scratch_directory();
        let ledger = Ledger::open(&directory)?;
        let mut survivor = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()?;
        let pid = i32::try_from(survivor.id())?;
        let stranger = ProcessIdentity::new(pid, PLATFORM_START)?;
        ledger.insert("task-s", &stranger)?;
        let settled = ledger.reap();
        let group_up = lgwks_std::process::process_group_exists(pid);
        drop(survivor.kill());
        drop(survivor.wait());
        drop(std::fs::remove_dir_all(&directory));
        assert_eq!(settled?, 0, "a surviving group settles no row");
        assert!(group_up?, "the group is still running after the reap");
        Ok(())
    }

    /// A ledger lock another executor holds for the whole bound is refused with the typed
    /// timeout, and the same open succeeds once the holder lets go (larry#214).
    #[test]
    fn a_held_ledger_lock_returns_the_typed_timeout() -> Result<(), Box<dyn std::error::Error>> {
        use std::time::Duration;
        let directory = scratch_directory();
        std::fs::create_dir_all(&directory)?;
        let holder = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(directory.join("executor.db.lock"))?;
        holder.lock()?;
        let refused = Ledger::open_within(&directory, Duration::from_millis(30));
        assert!(
            matches!(
                refused,
                Err(crate::executor::ExecError::LedgerLockTimeout { .. })
            ),
            "a held lock past its bound is the typed timeout"
        );
        drop(holder);
        let reopened = Ledger::open_within(&directory, Duration::from_millis(30));
        drop(std::fs::remove_dir_all(&directory));
        assert!(reopened.is_ok(), "a released lock opens the ledger");
        Ok(())
    }

    /// A recorded report is read back at exactly its byte bound and refused one
    /// byte past it, an id with no report is none, and a later record never
    /// replaces the first under one id (larry#182 defect 3).
    #[test]
    fn a_report_at_its_byte_bound_is_read_and_one_byte_past_is_refused()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = scratch_directory();
        let ledger = Ledger::open(&directory)?;
        let report = [7_u8; 16];
        ledger.keep_report("run-a", "digest-a", &report)?;
        assert_eq!(
            ledger.report("run-a", report.len())?,
            Some(report.to_vec()),
            "a report exactly at the bound is read whole"
        );
        assert!(
            matches!(
                ledger.report("run-a", report.len().saturating_sub(1)),
                Err(crate::executor::ExecError::Receipt(_))
            ),
            "a report one byte past the bound is refused, never truncated"
        );
        assert_eq!(
            ledger.report("run-b", report.len())?,
            None,
            "an id with no record reads as none"
        );
        ledger.keep_report("run-a", "digest-b", &[9_u8; 8])?;
        assert_eq!(
            ledger.report("run-a", report.len())?,
            Some(report.to_vec()),
            "the first record under one id is the run"
        );
        drop(std::fs::remove_dir_all(&directory));
        Ok(())
    }

    /// Openers racing on one fresh state directory: every first open waits for the
    /// one switching the store to WAL instead of failing "database is locked"
    /// (larry#131, logicalworks-crates#873). Each round is a new directory, so
    /// every round races the creation itself.
    #[test]
    fn concurrent_first_opens_of_one_fresh_ledger_all_succeed()
    -> Result<(), Box<dyn std::error::Error>> {
        /// Openers per round: past the two executors the journey starts.
        const OPENERS: usize = 16;
        /// Rounds, each on a fresh directory.
        const ROUNDS: usize = 8;
        for _round in 0..ROUNDS {
            let directory = scratch_directory();
            let barrier = std::sync::Barrier::new(OPENERS);
            let refusals = std::thread::scope(|scope| {
                let openers: Vec<_> = (0..OPENERS)
                    .map(|_opener| {
                        scope.spawn(|| {
                            barrier.wait();
                            Ledger::open(&directory)
                                .map(drop)
                                .map_err(|error| error.to_string())
                        })
                    })
                    .collect();
                openers
                    .into_iter()
                    .filter_map(|opener| match opener.join() {
                        Ok(Ok(())) => None,
                        Ok(Err(refusal)) => Some(refusal),
                        Err(_panicked) => Some("an opener panicked".to_owned()),
                    })
                    .collect::<Vec<String>>()
            });
            drop(std::fs::remove_dir_all(&directory));
            assert_eq!(
                refusals,
                Vec::<String>::new(),
                "every concurrent first open succeeds"
            );
        }
        Ok(())
    }

    /// Executors racing their first receipt on one fresh state directory all sign
    /// under one key, a reopen keeps it, the store is owner-only, and another
    /// directory (another tenant) holds another key (larry#86).
    #[test]
    fn sim_racing_first_uses_share_one_signing_key() -> Result<(), Box<dyn std::error::Error>> {
        /// Racers per round: past the two executors the journeys start together.
        const RACERS: usize = 16;
        /// Rounds, each on a fresh directory.
        const ROUNDS: usize = 8;
        let mut keys_per_round: Vec<[u8; 32]> = Vec::with_capacity(ROUNDS);
        for _round in 0..ROUNDS {
            let directory = scratch_directory();
            let barrier = std::sync::Barrier::new(RACERS);
            let drawn = std::thread::scope(|scope| {
                let racers: Vec<_> = (0..RACERS)
                    .map(|_racer| {
                        scope.spawn(|| {
                            barrier.wait();
                            Ledger::open(&directory)
                                .and_then(|ledger| ledger.signing_key())
                                .map(|key| key.public_key().to_bytes())
                                .map_err(|error| error.to_string())
                        })
                    })
                    .collect();
                racers
                    .into_iter()
                    .map(|racer| match racer.join() {
                        Ok(outcome) => outcome,
                        Err(_panicked) => Err("a racer panicked".to_owned()),
                    })
                    .collect::<Result<Vec<[u8; 32]>, String>>()
            })?;
            let first = *drawn.first().ok_or("every round draws a key")?;
            assert!(
                drawn.iter().all(|key| *key == first),
                "every racer on one directory signs under one key"
            );
            let reopened = Ledger::open(&directory)?
                .signing_key()?
                .public_key()
                .to_bytes();
            assert_eq!(reopened, first, "a reopen keeps the stored key");
            let mode = std::fs::metadata(directory.join("executor.db"))?
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "the store holding the seed is owner-only"
            );
            keys_per_round.push(first);
            drop(std::fs::remove_dir_all(&directory));
        }
        let mut distinct = keys_per_round.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            ROUNDS,
            "each state directory draws its own key"
        );
        Ok(())
    }
}
