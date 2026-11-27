//! The ledger of process groups a run has started and not yet settled, the
//! receipt signing key and the tenant's model routes, in `<state>/executor.db`
//! (larry#85, larry#86, larry#201).

use std::fs::File;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use lgwks_bot::kernel::clock::Clock;
use lgwks_deps::storefront::rusqlite::{self, OptionalExtension};
use lgwks_std::process::{ProcessIdentity, reap_orphaned_group};

use super::ExecError;
use super::run::await_gone;

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

/// Schema of the tenant residency policy, version 6 (larry#203): one canonical
/// comma-separated ISO code list per tenant, kept beside that tenant's routes.
const RESIDENCY_SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS tenant_residency(tenant TEXT PRIMARY KEY, codes TEXT NOT NULL);";

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
    /// Executors starting together on one state directory open the store at once:
    /// `lgwks_deps::sqlite::open` sets its 60 s busy timeout before its first
    /// statement and moves a fresh file to WAL under a blocking kernel lock
    /// (logicalworks-crates#873), and `migrate` runs each step in one immediate
    /// transaction (#872). Every wait is the store's busy handler, which sleeps, so
    /// a waiter leaves the core to the holder (larry#398 removed the lock loop that
    /// spun beside it).
    ///
    /// The store holds the receipt key, so it is created owner-only before SQLite
    /// opens it (SQLite gives the write-ahead files the database's mode), and a
    /// store left wider by an earlier release is narrowed on every open.
    ///
    /// # Errors
    ///
    /// [`ExecError::StateDir`] when the state directory cannot be made, and the
    /// store, key-store and migration errors of each step.
    pub(crate) fn open(state_dir: &Path) -> Result<Ledger, ExecError> {
        std::fs::create_dir_all(state_dir).map_err(ExecError::StateDir)?;
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
            8,
            &[
                SCHEMA,
                KEY_SCHEMA,
                TENANT_KEY_SCHEMA,
                REPORT_SCHEMA,
                ROUTE_SCHEMA,
                RESIDENCY_SCHEMA,
                policy::POLICY_SCHEMA,
                policy::REPO_ROOT_SCHEMA,
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
        Ok(Ledger { conn })
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
    ///
    /// The group waits read one deadline from `clock`, the caller's, so one clock covers
    /// this reap and no wait is decided by a count of yields (larry#268). The run path
    /// builds it with [`larry_runtime::live_clock`], the way the gate builds its own; a
    /// simulation passes a virtual clock so the deadline passes with no real time
    /// (larry#308).
    ///
    /// A row whose task `held` says is still running belongs to a concurrent executor on this
    /// state directory, and is left alone: reaping it killed that run's command (larry#456).
    pub(crate) fn reap(
        &self,
        clock: &Clock,
        held: impl Fn(&str) -> Result<bool, ExecError>,
    ) -> Result<usize, ExecError> {
        self.reap_with(&held, |pgid| {
            await_gone(clock, || {
                lgwks_std::process::process_group_exists(pgid).map_err(ExecError::Reap)
            })
        })
    }

    /// The row loop over a group-gone answer: `true` when the group is gone, `false` when it
    /// outlived the caller's deadline. A row is dropped only once its group is gone, so a
    /// surviving group keeps its row and the next start tries again (INVARIANTS, larry#214).
    /// The probe is the caller's, so the simulation drives the same loop on a clock it moves.
    fn reap_with<E>(
        &self,
        held: &dyn Fn(&str) -> Result<bool, ExecError>,
        mut group_gone: impl FnMut(i32) -> Result<bool, E>,
    ) -> Result<usize, E>
    where
        E: From<ExecError>,
    {
        let rows = self.rows()?;
        if rows.is_empty() {
            return Ok(0);
        }
        let mut settled = 0_usize;
        for (task_id, pid, started) in rows {
            if held(&task_id)? {
                continue;
            }
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
            if !group_gone(identity.pid())? {
                continue;
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

    /// Every recorded (task, pid, start time) row.
    fn rows(&self) -> Result<Vec<(String, i64, String)>, ExecError> {
        let mut statement = self
            .conn
            .prepare("SELECT task_id, pid, started FROM live_groups")
            .map_err(ExecError::Sql)?;
        let mapped = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(ExecError::Sql)?;
        mapped
            .collect::<Result<Vec<(String, i64, String)>, rusqlite::Error>>()
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

/// The route table's reads and writes (larry#201), a child module so the
/// connection stays private to the ledger.
#[path = "ledger_routes.rs"]
mod routes;

/// The tenant policy and repository-root records, kept apart from process rows.
#[path = "ledger_policy.rs"]
mod policy;

/// Key custody: the receipt seed and the tenant key digests (larry#86, larry#89).
mod key;

/// Seeded adversarial simulations of the report rows a keyed retry answers from
/// (larry#182 defect 3).
#[cfg(test)]
#[path = "sim_ledger.rs"]
mod sim;

/// Ledger tests: a dead leader and a damaged row are both settled.
#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use larry_contracts::authority::Scope;
    use lgwks_deps::storefront::rusqlite;
    use lgwks_std::process::ProcessIdentity;

    use super::Ledger;

    /// A pid no live process holds: the largest positive `i32` less one.
    const DEAD_PID: i32 = 2_147_483_646;

    /// A fresh state directory: empty when it is made and removed when its guard drops, so a
    /// process id the system reuses never lands a test in a stale tree (larry#267).
    fn scratch_directory() -> Result<crate::executor::sim_tree::Scratch, crate::sim_error::SimError>
    {
        crate::executor::sim_tree::scratch("larry-ledger")
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
        let directory = scratch_directory()?;
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
        let clock = larry_runtime::live_clock()?;
        assert_eq!(
            ledger.reap(&clock, |_task| Ok(false))?,
            2,
            "the dead-leader row and the damaged row are both settled"
        );
        assert!(ledger.live_tasks()?.is_empty(), "reap leaves no row behind");
        Ok(())
    }

    /// A row whose task a live run still holds is another executor's command on this state
    /// directory: the start-up reap neither signals its group nor drops its row (larry#456).
    #[test]
    fn a_held_task_group_is_never_reaped() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::process::CommandExt;
        let directory = scratch_directory()?;
        let ledger = Ledger::open(&directory)?;
        let mut running = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()?;
        let pid = i32::try_from(running.id())?;
        let identity = lgwks_std::process::identify_process(pid)?.ok_or("the group vanished")?;
        ledger.insert("task-held", &identity)?;
        let clock = larry_runtime::live_clock()?;
        let settled = ledger.reap(&clock, |task| Ok(task == "task-held"));
        let group_up = lgwks_std::process::process_group_exists(pid);
        let killed = running.kill();
        let reaped = running.wait();
        assert_eq!(settled?, 0, "a held task's row is not settled");
        assert!(
            group_up?,
            "a held task's group is still running after the reap"
        );
        assert_eq!(
            ledger.live_tasks()?,
            vec!["task-held".to_owned()],
            "its row stays"
        );
        killed?;
        reaped?;
        Ok(())
    }

    /// A group that survives its reap keeps its row, so the next start tries again. The
    /// row's identity names a start the live group does not carry, so the reap reads it
    /// as reused and signals nothing; the group stays up and its row stays with it (larry#214).
    #[test]
    fn a_surviving_group_keeps_its_row() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::process::CommandExt;
        let directory = scratch_directory()?;
        let ledger = Ledger::open(&directory)?;
        let mut survivor = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()?;
        let pid = i32::try_from(survivor.id())?;
        let stranger = ProcessIdentity::new(pid, PLATFORM_START)?;
        ledger.insert("task-s", &stranger)?;
        let clock = larry_runtime::live_clock()?;
        let settled = ledger.reap(&clock, |_task| Ok(false));
        let group_up = lgwks_std::process::process_group_exists(pid);
        // The survivor is stopped before any assert, so a failing one leaves no process behind;
        // a stop that fails is the test's failure too.
        let killed = survivor.kill();
        let reaped = survivor.wait();
        assert_eq!(settled?, 0, "a surviving group settles no row");
        assert!(group_up?, "the group is still running after the reap");
        killed?;
        reaped?;
        Ok(())
    }

    /// At the cap a new tenant's key writes nothing and says so, while a keyed tenant still
    /// rotates; under the cap a new tenant is stored (larry#397).
    #[test]
    fn a_tenant_key_past_the_cap_writes_nothing_and_a_rotation_still_lands()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = scratch_directory()?;
        let ledger = Ledger::open(&directory)?;
        let digest = |nibble: char| nibble.to_string().repeat(64);
        assert!(
            ledger.set_tenant_key("ada", &digest('a'), 2)?,
            "under the cap"
        );
        assert!(
            ledger.set_tenant_key("bo", &digest('b'), 2)?,
            "the last slot"
        );
        assert!(
            !ledger.set_tenant_key("cy", &digest('c'), 2)?,
            "past the cap"
        );
        assert!(
            ledger.set_tenant_key("ada", &digest('d'), 2)?,
            "a rotation at the cap"
        );
        assert_eq!(
            ledger.tenant_keys(8)?,
            [
                (String::from("ada"), digest('d')),
                (String::from("bo"), digest('b'))
            ],
            "the refused tenant wrote no row and the rotation replaced the digest"
        );
        Ok(())
    }

    /// Policy and repository roots are durable tenant records. Authorization is
    /// component-bounded, rejects the state directory, and refuses another root.
    #[test]
    fn sim_tenant_policy_and_repository_roots_survive_reopen()
    -> Result<(), Box<dyn std::error::Error>> {
        let state = scratch_directory()?;
        let root = scratch_directory()?;
        let repository = root.join("repo");
        std::fs::create_dir_all(&repository)?;
        let policy = [Scope::FileRead, Scope::ProcessSpawn]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        let ledger = Ledger::open(&state)?;
        ledger.set_tenant_policy("tenant", &policy)?;
        ledger.set_tenant_repo_root("tenant", &root)?;
        assert_eq!(ledger.tenant_policy("tenant")?, policy);
        assert!(
            ledger
                .authorized_repo("tenant", &state, &repository, false)
                .is_ok()
        );
        assert!(matches!(
            ledger.authorized_repo("tenant", &state, &state.join("ws"), false),
            Err(crate::executor::ExecError::RepoPath { .. })
        ));
        drop(ledger);
        let reopened = Ledger::open(&state)?;
        assert_eq!(reopened.tenant_policy("tenant")?, policy);
        assert!(
            reopened
                .authorized_repo("tenant", &state, &root, false)
                .is_ok()
        );
        Ok(())
    }

    /// A recorded report is read back at exactly its byte bound and refused one
    /// byte past it, an id with no report is none, and a later record never
    /// replaces the first under one id (larry#182 defect 3).
    #[test]
    fn a_report_at_its_byte_bound_is_read_and_one_byte_past_is_refused()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = scratch_directory()?;
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
            let directory = scratch_directory()?;
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
            let directory = scratch_directory()?;
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
