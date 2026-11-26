//! Issue #216 benchmark: the three hot paths must scale with open work, not history.
//!
//! The benchmark drives the public runtime ingress and tick. It builds recorded
//! runs, settled questions, or terminal receipts at each requested size, then
//! samples the named operation 64 times. Peak RSS is sampled with `ps` outside
//! the timed operation.

mod support;

use std::collections::BTreeSet;
use std::io::Write;
use std::time::{Duration, Instant};

use larry_contracts::authority::Scope;
use larry_contracts::command::{Command, CommandContent, CommandKind};
use larry_contracts::effect::{EffectOutcome, ReconciliationStrategy};
use larry_contracts::event::{Event, EventKind, EventSequence};
use larry_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId, WorkerId};
use larry_runtime::journal::Journal;
use larry_runtime::owner::fixture::ScriptedOwner;
use larry_runtime::payload::{CommandAcceptedPayload, ReceiptPayload};
use larry_runtime::runtime::{Answer, Decision, Question, RuntimeSettings, Subject, restore};

use support::{TestRuntime, clock, proposal, settings, submit, tenant};

const SAMPLES: usize = 64;
const BATCH: usize = 128;
const MAX_BENCH_RUNS: usize = 60_000;

fn wide(index: usize, lead: u8) -> [u8; 16] {
    let mut bytes = [lead; 16];
    for (slot, byte) in bytes.iter_mut().skip(8).zip(index.to_be_bytes()) {
        *slot = byte;
    }
    bytes
}

fn bench_settings() -> Result<RuntimeSettings, Box<dyn std::error::Error>> {
    let mut configured = settings(&[Scope::FileRead], &[Scope::FileRead], 1_000_000)?;
    configured.journal_capacity = 1_000_000;
    configured.run_capacity = 65_536;
    configured.max_runs_per_tick = 256;
    Ok(configured)
}

fn command(
    id: CommandId,
    tenant_id: TenantId,
    kind: CommandKind,
    body: impl Into<Vec<u8>>,
) -> Result<Command, Box<dyn std::error::Error>> {
    Ok(Command::new(
        id,
        tenant_id,
        Vec::new(),
        CommandContent::new(kind, body.into().into_boxed_slice()),
    )?)
}

fn idle_command(index: usize, tenant_id: TenantId) -> Result<Command, Box<dyn std::error::Error>> {
    let body = proposal(
        RunId::from_bytes(wide(index, 1)),
        tenant_id,
        OperationId::from_bytes(wide(index, 2)),
        &[],
        0,
        ReconciliationStrategy::Queryable,
    );
    command(
        CommandId::from_bytes(wide(index, 3)),
        tenant_id,
        CommandKind::Steer,
        lgwks_std::json::to_vec(&body)?,
    )
}

fn accepted_event(
    tenant_id: TenantId,
    run_id: RunId,
    command: CommandId,
    kind: CommandKind,
    sequence: EventSequence,
) -> Result<Event, Box<dyn std::error::Error>> {
    let payload = CommandAcceptedPayload::new(command, run_id, kind);
    Ok(Event::new(
        tenant_id,
        run_id,
        sequence,
        Vec::new(),
        EventKind::CommandAccepted,
        lgwks_std::json::to_vec(&payload)?.into_boxed_slice(),
    )?)
}

fn restore_journal(
    journal: Journal,
    owner: ScriptedOwner,
) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    let settings = bench_settings()?;
    let mut restored = restore(settings, owner, clock()?, journal)?;
    let epoch = restored.fence().epoch()?;
    restored.fence_mut().authenticate(epoch)?;
    Ok(restored)
}

fn recorded_runs(size: usize) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    let configured = bench_settings()?;
    let tenant_id = tenant(1);
    let mut runtime =
        larry_runtime::runtime::Runtime::new(configured, ScriptedOwner::new(), clock()?)?;
    for index in 0..size {
        runtime.submit(idle_command(index, tenant_id)?)?;
        if index.saturating_add(1).is_multiple_of(BATCH) {
            runtime.tick()?;
        }
    }
    if !size.is_multiple_of(BATCH) {
        runtime.tick()?;
    }
    Ok(runtime)
}

fn worker(byte: u8) -> WorkerId {
    WorkerId::from_bytes([byte; 16])
}

fn question_command(
    index: usize,
    tenant_id: TenantId,
) -> Result<(Command, Subject), Box<dyn std::error::Error>> {
    let run_id = RunId::from_bytes(wide(index, 11));
    let operation = OperationId::from_bytes(wide(index, 12));
    let work_digest = ContentDigest::over(&[b"larry216-work", &wide(index, 13)]);
    let subject = Subject::new(
        run_id,
        operation,
        work_digest,
        ContentDigest::over(&[b"larry216-schema"]),
    );
    let expires = std::time::SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_003_600))
        .ok_or("benchmark expiry overflow")?;
    let question = Question::new(subject, worker(0xA0), worker(1), expires, BTreeSet::new());
    let ask = command(
        CommandId::from_bytes(wide(index, 14)),
        tenant_id,
        CommandKind::Ask,
        lgwks_std::json::to_vec(&question)?,
    )?;
    Ok((ask, subject))
}

fn human(tenant_id: TenantId) -> larry_runtime::protocol::ProtocolOffer {
    larry_runtime::protocol::ProtocolOffer::new(
        tenant_id,
        larry_runtime::protocol::ProtocolVersionRange::new(1, 1),
        [
            larry_runtime::protocol::ProtocolCapability::SubmitCommand,
            larry_runtime::protocol::ProtocolCapability::HumanQuestion,
        ]
        .into_iter()
        .collect(),
        false,
        false,
    )
    .authenticated(worker(1))
}

fn settled_questions(size: usize) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    let mut journal = Journal::new(1_000_000)?;
    let tenant_id = tenant(1);
    for index in 0..size {
        let (ask, subject) = question_command(index, tenant_id)?;
        let run_id = RunId::from_bytes(wide(index, 11));
        let ask_event = accepted_event(
            tenant_id,
            run_id,
            ask.id(),
            CommandKind::Ask,
            EventSequence::FIRST,
        )?;
        journal.accept_command(ask)?;
        journal.record(ask_event)?;
        let answer = Answer::new(
            CommandId::from_bytes(wide(index, 14)),
            worker(1),
            subject,
            Decision::Reject,
        );
        let answer = command(
            CommandId::from_bytes(wide(index, 15)),
            tenant_id,
            CommandKind::Answer,
            lgwks_std::json::to_vec(&answer)?,
        )?;
        let answer_event = accepted_event(
            tenant_id,
            run_id,
            answer.id(),
            CommandKind::Answer,
            EventSequence::new(1),
        )?;
        journal.accept_command(answer)?;
        journal.record(answer_event)?;
    }
    restore_journal(journal, ScriptedOwner::new())
}

fn receipts(size: usize) -> Result<TestRuntime, Box<dyn std::error::Error>> {
    let mut journal = Journal::new(1_000_000)?;
    let tenant_id = tenant(1);
    for index in 0..size {
        let run_id = RunId::from_bytes(wide(index, 21));
        let operation = OperationId::from_bytes(wide(index, 22));
        let receipt = larry_contracts::effect::TerminalReceipt::publish(
            operation,
            run_id,
            tenant_id,
            larry_contracts::effect::TerminalDisposition::Succeeded,
        );
        let event = Event::new(
            tenant_id,
            run_id,
            EventSequence::FIRST,
            Vec::new(),
            EventKind::TerminalReceiptPublished,
            lgwks_std::json::to_vec(&ReceiptPayload::new(receipt))?.into_boxed_slice(),
        )?;
        journal.record(event)?;
    }
    let outcomes: Vec<_> = (0..SAMPLES)
        .map(|index| {
            let sample = size.checked_add(index).ok_or("sample identity overflow")?;
            Ok::<_, Box<dyn std::error::Error>>(EffectOutcome::Succeeded {
                operation: OperationId::from_bytes(wide(sample, 42)),
            })
        })
        .collect::<Result<_, _>>()?;
    restore_journal(journal, ScriptedOwner::scripted(outcomes))
}
fn rss_kib() -> Result<u64, Box<dyn std::error::Error>> {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()?;
    Ok(String::from_utf8(output.stdout)?.trim().parse()?)
}

fn percentile(samples: &[Duration], rank: usize) -> Result<f64, Box<dyn std::error::Error>> {
    let position = samples
        .len()
        .checked_mul(rank)
        .ok_or("percentile overflow")?
        .div_ceil(100)
        .saturating_sub(1);
    Ok(samples
        .get(position)
        .ok_or("no benchmark samples")?
        .as_secs_f64()
        * 1_000_000.0)
}

fn measure(
    size: usize,
    metric: &str,
    mut operation: impl FnMut(usize) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut samples = Vec::with_capacity(SAMPLES);
    let mut peak_rss = 0_u64;
    for index in 0..SAMPLES {
        let started = Instant::now();
        operation(index)?;
        samples.push(started.elapsed());
        peak_rss = peak_rss.max(rss_kib()?);
    }
    samples.sort();
    let line = format!(
        "open-work-bench runs={size} metric={metric} samples={SAMPLES} p50_us={:.1} p95_us={:.1} p99_us={:.1} peak_rss_kib={peak_rss}\n",
        percentile(&samples, 50)?,
        percentile(&samples, 95)?,
        percentile(&samples, 99)?,
    );
    std::io::stderr().lock().write_all(line.as_bytes())?;
    Ok(())
}

/// Measures the three public hot paths at every size required by #216.
#[test]
fn open_work_benchmark_prints_all_required_percentiles() -> Result<(), Box<dyn std::error::Error>> {
    for size in [1_usize, 1_000, MAX_BENCH_RUNS] {
        let mut tick_runtime = recorded_runs(size)?;
        let tenant_id = tenant(1);
        let open_index = size.checked_add(1).ok_or("open identity overflow")?;
        let open_body = proposal(
            RunId::from_bytes(wide(open_index, 31)),
            tenant_id,
            OperationId::from_bytes(wide(open_index, 32)),
            &[Scope::FileRead],
            1,
            ReconciliationStrategy::Queryable,
        );
        tick_runtime.submit(submit(
            CommandId::from_bytes(wide(open_index, 33)),
            &open_body,
        )?)?;
        tick_runtime.tick()?;
        measure(size, "tick", |_| {
            tick_runtime.tick()?;
            Ok(())
        })?;

        let mut ask_runtime = settled_questions(size)?;
        let offer = human(tenant_id);
        measure(size, "ask", |index| {
            let ask_index = size.checked_add(index).ok_or("ask identity overflow")?;
            let (ask, _) = question_command(ask_index, tenant_id)?;
            ask_runtime.submit_protocol(&offer, ask, None)?;
            Ok(())
        })?;

        let mut receipt_runtime = receipts(size)?;
        measure(size, "receipt", |index| {
            let sample = size.checked_add(index).ok_or("receipt identity overflow")?;
            let body = proposal(
                RunId::from_bytes(wide(sample, 41)),
                tenant_id,
                OperationId::from_bytes(wide(sample, 42)),
                &[Scope::FileRead],
                1,
                ReconciliationStrategy::Queryable,
            );
            receipt_runtime.submit(submit(CommandId::from_bytes(wide(sample, 43)), &body)?)?;
            receipt_runtime.tick()?;
            Ok(())
        })?;
    }
    Ok(())
}
