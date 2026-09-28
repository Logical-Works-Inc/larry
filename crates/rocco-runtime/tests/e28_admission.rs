//! E28 public-runtime crash, replay and safe-boundary falsifiers.
//! File storage here is a test owner, not a production persistence adapter.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use rocco_contracts::authority::{BudgetUnit, Scope};
use rocco_contracts::command::{Command, CommandAcceptance, CommandContent, CommandKind};
use rocco_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use rocco_contracts::error::CommandError;
use rocco_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId};
use rocco_contracts::replay::{CapabilitySnapshot, PolicySnapshot, ValiditySnapshot};
use rocco_runtime::JournalStorage;
use rocco_runtime::authority::fixture::FixtureAuthority;
use rocco_runtime::clock::FixedClock;
use rocco_runtime::error::RuntimeError;
use rocco_runtime::journal::{Journal, JournalEntry};
use rocco_runtime::owner::fixture::ScriptedOwner;
use rocco_runtime::payload::WorkProposal;
use rocco_runtime::projection::{FoldProjection, ProjectionBackend};
use rocco_runtime::runtime::{Runtime, RuntimeSettings, restore};

#[path = "sim/e28_admission.rs"]
mod simulation;

/// Test errors preserve the failing public boundary.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
/// Runtime driven by deterministic owner responses.
type TestRuntime = Runtime<FoldProjection, ScriptedOwner, FixedClock, FixtureAuthority>;
/// Bounded test journal, with injected acknowledgement loss.
#[derive(Debug, Default)]
struct Store {
    /// The committed prefix survives replacement of the runtime.
    entries: Mutex<Vec<JournalEntry>>,
    /// Simulate a committed append whose acknowledgement is lost.
    lose_ack: bool,
}

impl JournalStorage for Store {
    fn append(&self, offset: usize, entry: &JournalEntry) -> std::io::Result<()> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| std::io::Error::other("poisoned test store"))?;
        if offset != entries.len() || offset >= 4096 {
            return Err(std::io::Error::other("offset or capacity refused"));
        }
        entries.push(entry.clone());
        if self.lose_ack {
            return Err(std::io::Error::other("committed acknowledgement lost"));
        }
        Ok(())
    }
}

/// A fixed configuration, varied by the simulation's one seed.
fn settings(seed: u64) -> RuntimeSettings {
    RuntimeSettings::with_defaults(
        PolicySnapshot::new(seed, [Scope::FileRead].into_iter().collect()),
        CapabilitySnapshot::new([Scope::FileRead].into_iter().collect()),
        ValiditySnapshot::new(
            SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(100_000))
                .unwrap_or(SystemTime::UNIX_EPOCH),
        ),
        BudgetUnit::Requests,
        100,
    )
}

/// Effect authority for the one legitimate submitter, tenant 1, taken from
/// the test's own snapshots. Tenant 2, the cross-tenant correction case in the
/// simulation module, is deliberately untrusted.
fn authority(settings: &RuntimeSettings) -> TestResult<FixtureAuthority> {
    let mut source =
        FixtureAuthority::new(settings.security_epoch, settings.offline_dispatch_bound);
    source.trust(
        TenantId::from_bytes(raw(1)),
        settings.policy.clone(),
        settings.capabilities.clone(),
        settings.validity,
    )?;
    Ok(source)
}

/// Seed-derived identities do not depend on process state or random devices.
fn raw(seed: u64) -> [u8; 16] {
    u128::from(seed).to_be_bytes()
}

/// Constructs independently varied typed work and corrections.
fn command(seed: u64, tenant: u64, kind: CommandKind, cost: u64) -> TestResult<Command> {
    let body = WorkProposal::new(
        RunId::from_bytes(raw(seed)),
        TenantId::from_bytes(raw(tenant)),
        OperationId::from_bytes(raw(seed)),
        [Scope::FileRead].into_iter().collect(),
        cost,
        BudgetUnit::Requests,
        ReconciliationStrategy::Queryable,
    );
    Ok(Command::new(
        CommandId::from_bytes(raw(seed
            .saturating_mul(4)
            .saturating_add(u64::from(kind.tag())))),
        body.tenant,
        Vec::new(),
        CommandContent::new(kind, lgwks_std::json::to_vec(&body)?.into_boxed_slice()),
    )?)
}

/// Restores a fresh runtime exclusively from the storage owner's committed prefix.
fn reopen(store: &Arc<Store>, seed: u64, outcomes: Vec<EffectOutcome>) -> TestResult<TestRuntime> {
    let entries = store
        .entries
        .lock()
        .map_err(|_| "poisoned test store")?
        .clone();
    let concrete = Arc::clone(store);
    let storage: Arc<dyn JournalStorage> = concrete;
    let settings = settings(seed);
    let authority = authority(&settings)?;
    Ok(restore(
        settings,
        ScriptedOwner::scripted(outcomes),
        FixedClock::new(SystemTime::UNIX_EPOCH),
        Journal::with_storage(4096, entries, storage)?,
        authority,
    )?)
}

/// Process-owned test file. No production file/database adapter is installed.
#[derive(Debug)]
struct FileStore {
    /// Existing test-owned journal file.
    path: PathBuf,
    /// Crash barrier after the first fsync, before append returns.
    crash_barrier: bool,
}

impl JournalStorage for FileStore {
    fn append(&self, offset: usize, entry: &JournalEntry) -> std::io::Result<()> {
        let bytes = std::fs::read(&self.path)?;
        let count = bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .count();
        if count != offset || bytes.len() > 1_048_576 {
            return Err(std::io::Error::other("test journal offset or size refused"));
        }
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        let encoded = lgwks_std::json::to_vec(entry).map_err(std::io::Error::other)?;
        file.write_all(&encoded)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        if self.crash_barrier {
            std::io::stdout().write_all(b"E28_DURABLE_BEFORE_ACK\n")?;
            std::io::stdout().flush()?;
            let mut release = [0_u8; 1];
            std::io::stdin().read_exact(&mut release)?;
            return Err(std::io::Error::other("crash barrier unexpectedly released"));
        }
        Ok(())
    }
}

/// Loads only committed test journal entries into a new runtime process.
fn file_runtime(path: &Path, crash_barrier: bool) -> TestResult<TestRuntime> {
    let mut entries = Vec::new();
    for line in std::fs::read(path)?
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        entries.push(lgwks_std::json::from_slice::<JournalEntry>(line)?);
    }
    let storage = Arc::new(FileStore {
        path: path.to_path_buf(),
        crash_barrier,
    });
    let settings = settings(1);
    let authority = authority(&settings)?;
    Ok(restore(
        settings,
        ScriptedOwner::scripted([EffectOutcome::Succeeded {
            operation: OperationId::from_bytes(raw(1)),
        }]),
        FixedClock::new(SystemTime::UNIX_EPOCH),
        Journal::with_storage(4096, entries, storage)?,
        authority,
    )?)
}

/// Child entrypoint: absent explicit child environment this performs no process work.
#[test]
fn e28_process_child() -> TestResult {
    let Some(path) = std::env::var_os("ROCCO_E28_JOURNAL") else {
        return Ok(());
    };
    let crash = std::env::var_os("ROCCO_E28_CRASH").is_some();
    let mut runtime = file_runtime(Path::new(&path), crash)?;
    let work = command(1, 1, CommandKind::Submit, 1)?;
    if crash {
        runtime.admit(work)?;
        return Err("admission acknowledged before crash".into());
    }
    assert_eq!(
        runtime.admit(work)?,
        CommandAcceptance::DuplicateReplay,
        "fresh process deduplicates"
    );
    assert!(
        matches!(
            runtime.admit(command(1, 1, CommandKind::Submit, 2)?),
            Err(RuntimeError::Command(CommandError::ConflictingRetry { .. }))
        ),
        "fresh process rejects conflict"
    );
    runtime.tick()?;
    assert_eq!(
        runtime.owner().dispatched().len(),
        1,
        "accepted work is actually recovered"
    );
    assert_eq!(
        runtime.projection().receipts().len(),
        1,
        "recovered work completes"
    );
    assert_eq!(
        runtime
            .journal()
            .entries()
            .iter()
            .filter(|entry| matches!(entry, JournalEntry::Command(_)))
            .count(),
        1,
        "one accepted identity in recovered history"
    );
    Ok(())
}

/// Ensures every spawned child is reaped even if an assertion fails.
struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        match self.0.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                let killed = self.0.kill();
                let waited = self.0.wait();
                assert!(
                    killed.is_ok() && waited.is_ok(),
                    "owned child cleanup: {killed:?} {waited:?}"
                );
            }
        }
    }
}

/// Polls only the owned process, with a fixed failure deadline.
fn wait_bounded(child: &mut Child) -> TestResult<std::process::ExitStatus> {
    let started = std::time::Instant::now();
    let (wake, waiting) = std::sync::mpsc::sync_channel::<()>(1);
    loop {
        if let Some(status) = child.try_wait()? {
            drop(wake);
            return Ok(status);
        }
        if started.elapsed() >= Duration::from_secs(10) {
            child.kill()?;
            child.wait()?;
            return Err("recovery child exceeded its deadline".into());
        }
        match waiting.recv_timeout(Duration::from_millis(5)) {
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            other => return Err(format!("unexpected process-wait wake: {other:?}").into()),
        }
    }
}

/// Terminates the real child after fsync, with no acknowledgement delivered.
#[test]
fn e28_real_crash_before_acknowledgement() -> TestResult {
    use std::io::BufRead;
    let path = std::env::temp_dir().join(format!("rocco-e28-{}.journal", std::process::id()));
    File::create_new(&path)?.sync_all()?;
    let executable = std::env::current_exe()?;
    let mut child = OwnedChild(
        Process::new(&executable)
            .args(["--exact", "e28_process_child", "--nocapture"])
            .env("ROCCO_E28_JOURNAL", &path)
            .env("ROCCO_E28_CRASH", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let stdout = child.0.stdout.take().ok_or("missing child stdout")?;
    let (barrier, arrived) = std::sync::mpsc::sync_channel(1);
    std::thread::scope(|scope| -> TestResult {
        let reader = scope.spawn(move || -> std::io::Result<()> {
            let mut reader = std::io::BufReader::new(stdout);
            for _ in 0..16 {
                let mut line = String::new();
                if reader.read_line(&mut line)? == 0 {
                    break;
                }
                if line.trim() == "E28_DURABLE_BEFORE_ACK" {
                    barrier.send(()).map_err(std::io::Error::other)?;
                    return Ok(());
                }
            }
            Err(std::io::Error::other(
                "child exited before durable admission",
            ))
        });
        let observed = arrived.recv_timeout(Duration::from_secs(10));
        // Killing the owned child closes stdout, so the reader is always joinable.
        child.0.kill()?;
        let killed = child.0.wait()?;
        let read_result = reader.join().map_err(|_| "barrier reader panicked")?;
        observed?;
        read_result?;
        assert!(!killed.success(), "the admission process really died");
        Ok(())
    })?;
    let mut recovery = OwnedChild(
        Process::new(executable)
            .args(["--exact", "e28_process_child", "--nocapture"])
            .env("ROCCO_E28_JOURNAL", &path)
            .env_remove("ROCCO_E28_CRASH")
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    assert!(
        wait_bounded(&mut recovery.0)?.success(),
        "fresh recovery process must pass all assertions"
    );
    // Retain crash evidence rather than deleting the journal. The test-owned path
    // is outside the repo and uniquely binds this test process.
    Ok(())
}

/// A host-owned tool with a bounded command mailbox and observable result.
struct ComputingTool {
    /// At most one launch can wait for the worker.
    launch: std::sync::mpsc::SyncSender<OperationId>,
    /// The actual worker result, independent of runtime projection.
    result: std::sync::mpsc::Receiver<ContentDigest>,
    /// Captured output proves the original effect was not rewritten.
    observed: Option<ContentDigest>,
}

impl rocco_runtime::owner::EffectOwner for ComputingTool {
    fn dispatch(
        &mut self,
        intent: &rocco_contracts::effect::EffectIntent,
        _at: SystemTime,
    ) -> EffectOutcome {
        match self.launch.try_send(intent.operation) {
            Ok(()) => EffectOutcome::Unknown {
                operation: intent.operation,
            },
            Err(_) => EffectOutcome::Failed {
                operation: intent.operation,
            },
        }
    }

    fn reconcile(
        &mut self,
        intent: &rocco_contracts::effect::EffectIntent,
        _at: SystemTime,
    ) -> EffectOutcome {
        match self.result.try_recv() {
            Ok(digest) => {
                self.observed = Some(digest);
                EffectOutcome::Succeeded {
                    operation: intent.operation,
                }
            }
            Err(_) => EffectOutcome::Unknown {
                operation: intent.operation,
            },
        }
    }
}

/// A real owned tool waits while input arrives, then completes the original work.
#[test]
fn e28_correction_during_real_tool() -> TestResult {
    let (launch, launched) = std::sync::mpsc::sync_channel(1);
    let (release, released) = std::sync::mpsc::sync_channel(1);
    let (results, result) = std::sync::mpsc::sync_channel(1);
    let store = Arc::new(Store::default());
    let storage: Arc<dyn JournalStorage> = store;
    let pinned = settings(19);
    let source = authority(&pinned)?;
    let mut runtime: Runtime<FoldProjection, ComputingTool, FixedClock, FixtureAuthority> =
        restore(
            pinned,
            ComputingTool {
                launch,
                result,
                observed: None,
            },
            FixedClock::new(SystemTime::UNIX_EPOCH),
            Journal::with_storage(4096, Vec::new(), storage)?,
            source,
        )?;
    std::thread::scope(|scope| -> TestResult {
        let tool = scope.spawn(move || -> Result<ContentDigest, &'static str> {
            let operation: OperationId = launched
                .recv_timeout(Duration::from_secs(10))
                .map_err(|_| "tool was never launched")?;
            released
                .recv_timeout(Duration::from_secs(10))
                .map_err(|_| "tool was never released")?;
            let digest =
                ContentDigest::over(&[b"original committed tool input", operation.as_bytes()]);
            results
                .send(digest)
                .map_err(|_| "tool result receiver disappeared")?;
            Ok(digest)
        });
        let work = command(19, 1, CommandKind::Submit, 1)?;
        runtime.admit(work.clone())?;
        runtime.tick()?;
        let correction = command(19, 1, CommandKind::Steer, 7)?;
        runtime.admit(correction.clone())?;
        let held = runtime.tick()?;
        assert_eq!(
            held.deferred(),
            &[correction.id()],
            "tool is in flight at correction submission"
        );
        assert_eq!(
            runtime.correction(work.tenant(), RunId::from_bytes(raw(19)))?,
            None,
            "in-flight correction is not active"
        );
        release.send(())?;
        let actual = tool.join().map_err(|_| "owned tool panicked")??;
        assert_eq!(
            actual,
            ContentDigest::over(&[b"original committed tool input", &raw(19)]),
            "committed tool input was not rewritten"
        );
        let reconciled = runtime.tick()?;
        assert!(
            reconciled.promoted().is_empty(),
            "reconciliation tick is not the earlier admission boundary"
        );
        assert_eq!(
            runtime.owner().observed,
            Some(actual),
            "runtime observed the real worker result"
        );
        let promoted = runtime.tick()?;
        assert_eq!(
            promoted.promoted(),
            &[correction.id()],
            "first safe admission boundary promotes correction"
        );
        assert_eq!(
            runtime.correction(work.tenant(), RunId::from_bytes(raw(19)))?,
            Some(&correction),
            "caller receives exact promoted content"
        );
        Ok(())
    })
}
