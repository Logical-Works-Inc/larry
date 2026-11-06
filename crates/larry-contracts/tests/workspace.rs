//! Public-interface falsifiers for the bounded workspace/Continue boundary.
//! Test owners exercise the adapter contract; they are not live Moo evidence.

use larry_contracts::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId, WorkspaceId};
use larry_contracts::state::RunState;
use larry_contracts::workspace::{
    ContinueCoordinator, ContinueOutcome, ContinueReceipt, ContinueRequest, MAX_WORK_ITEMS,
    MAX_WORK_TITLE_BYTES, MissingMooAdapter, MooWorkspaceAdapter, WorkBoard, WorkItem, WorkVersion,
    WorkspaceError, WorkspaceScope,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn scope(tenant: u8, workspace: u8) -> WorkspaceScope {
    WorkspaceScope::new(
        TenantId::from_bytes([tenant; 16]),
        WorkspaceId::from_bytes([workspace; 16]),
    )
}

fn run(number: usize) -> RunId {
    let mut bytes = [0; 16];
    let encoded = number.to_be_bytes();
    let offset = 8usize.saturating_sub(encoded.len());
    bytes[offset..8].copy_from_slice(&encoded);
    RunId::from_bytes(bytes)
}

fn command(number: usize) -> CommandId {
    CommandId::from_bytes(*run(number).as_bytes())
}

fn version(revision: u64) -> WorkVersion {
    WorkVersion::new(revision, ContentDigest::over(&[b"baseline"]), 7)
}

fn item(number: usize, revision: u64, state: RunState) -> Result<WorkItem, WorkspaceError> {
    WorkItem::new(
        scope(1, 1),
        run(number),
        "Resume my workspace",
        version(revision),
        state,
    )
}

fn coordinator() -> Result<ContinueCoordinator, WorkspaceError> {
    let mut board = WorkBoard::new(scope(1, 1));
    board.upsert(item(1, 1, RunState::Admitted)?)?;
    Ok(ContinueCoordinator::new(board))
}

fn request(number: usize) -> ContinueRequest {
    ContinueRequest::new(command(number), scope(1, 1), run(1), 1)
}

#[derive(Default)]
struct RecordingOwner {
    calls: usize,
    refusal: Option<WorkspaceError>,
    current: Option<WorkVersion>,
    corrupt_receipt: bool,
}

impl MooWorkspaceAdapter for RecordingOwner {
    fn continue_work(
        &mut self,
        request: ContinueRequest,
        version: WorkVersion,
    ) -> Result<ContinueOutcome, WorkspaceError> {
        self.calls = self.calls.saturating_add(1);
        if let Some(error) = self.refusal {
            return Err(error);
        }
        if let Some(current) = self.current {
            version.verify_current(current)?;
        }
        let mut receipt = ContinueReceipt::new(
            request,
            version,
            OperationId::from_bytes(*request.command.as_bytes()),
        );
        if self.corrupt_receipt {
            receipt.request.scope = scope(9, 9);
        }
        Ok(ContinueOutcome::Queued(receipt))
    }
}

#[test]
fn title_byte_bound_and_controls_are_enforced_at_construction() -> TestResult {
    for invalid in ["", "  ", "line\nbreak", "\u{1b}[31mspoof", "delete\u{7f}"] {
        assert_eq!(
            WorkItem::new(scope(1, 1), run(1), invalid, version(1), RunState::Draft),
            Err(WorkspaceError::InvalidTitle)
        );
    }
    let title = "é".repeat(128);
    assert_eq!(
        title.len(),
        MAX_WORK_TITLE_BYTES,
        "fixture fills the UTF-8 byte bound exactly"
    );
    assert_eq!(
        WorkItem::new(scope(1, 1), run(1), &title, version(1), RunState::Draft)?.title(),
        title
    );
    assert_eq!(
        WorkItem::new(
            scope(1, 1),
            run(1),
            &format!("{title}x"),
            version(1),
            RunState::Draft
        ),
        Err(WorkspaceError::InvalidTitle)
    );
    Ok(())
}

#[test]
fn board_rejects_scope_staleness_and_conflicting_snapshots_without_mutation() -> TestResult {
    let mut board = WorkBoard::new(scope(1, 1));
    board.upsert(item(1, 2, RunState::Running)?)?;
    board.upsert(item(1, 2, RunState::Running)?)?;
    assert_eq!(
        board.upsert(item(1, 1, RunState::Running)?),
        Err(WorkspaceError::StaleWork)
    );
    assert_eq!(
        board.upsert(item(1, 2, RunState::Failed)?),
        Err(WorkspaceError::ConflictingSnapshot)
    );
    for other in [scope(2, 1), scope(1, 2)] {
        assert_eq!(
            board.upsert(WorkItem::new(
                other,
                run(1),
                "other",
                version(3),
                RunState::Running
            )?),
            Err(WorkspaceError::ScopeMismatch)
        );
    }
    assert_eq!(board.get(run(1)), Some(&item(1, 2, RunState::Running)?));
    assert_eq!(board.items().len(), 1);
    Ok(())
}

#[test]
fn terminal_projection_cannot_be_resurrected_with_newer_revision() -> TestResult {
    for terminal in RunState::ALL
        .into_iter()
        .filter(|state| state.is_terminal())
    {
        let mut board = WorkBoard::new(scope(1, 1));
        board.upsert(item(1, 1, terminal)?)?;
        assert_eq!(
            board.upsert(item(1, 2, RunState::Running)?),
            Err(WorkspaceError::TerminalWork)
        );
        assert_eq!(board.get(run(1)).map(WorkItem::state), Some(terminal));
    }
    Ok(())
}

#[test]
fn full_board_keeps_all_rows_and_permits_existing_row_refresh() -> TestResult {
    let mut board = WorkBoard::new(scope(1, 1));
    for number in (0..MAX_WORK_ITEMS).rev() {
        board.upsert(item(number, 1, RunState::Draft)?)?;
    }
    assert_eq!(
        board.upsert(item(MAX_WORK_ITEMS, 1, RunState::Draft)?),
        Err(WorkspaceError::CapacityExceeded)
    );
    board.upsert(item(0, 2, RunState::Admitted)?)?;
    assert_eq!(board.items().len(), MAX_WORK_ITEMS);
    assert_eq!(
        board.items().map(WorkItem::run).collect::<Vec<_>>(),
        (0..MAX_WORK_ITEMS).map(run).collect::<Vec<_>>()
    );
    assert!(board.get(run(MAX_WORK_ITEMS)).is_none());
    Ok(())
}

#[test]
fn disconnected_continue_is_real_awaiting_dependency_not_running_or_success() -> TestResult {
    let mut controller = coordinator()?;
    for _ in 0..100 {
        assert_eq!(
            controller.continue_work(&mut MissingMooAdapter::default(), request(1))?,
            ContinueOutcome::AwaitingDependency
        );
        assert_eq!(
            controller.board().get(run(1)).map(WorkItem::state),
            Some(RunState::Admitted)
        );
    }
    // Reconnection retries the exact action, with no fake success cached.
    let mut owner = RecordingOwner::default();
    assert!(matches!(
        controller.continue_work(&mut owner, request(1))?,
        ContinueOutcome::Queued(_)
    ));
    assert_eq!(owner.calls, 1);
    Ok(())
}

#[test]
fn queued_identity_replays_original_receipt_without_redispatch_or_completion_claim() -> TestResult {
    let mut controller = coordinator()?;
    let mut owner = RecordingOwner::default();
    let first = controller.continue_work(&mut owner, request(1))?;
    assert_eq!(controller.continue_work(&mut owner, request(1))?, first);
    assert_eq!(owner.calls, 1);
    assert_eq!(
        controller.board().get(run(1)).map(WorkItem::state),
        Some(RunState::Admitted)
    );
    // Later observation does not rewrite the original queue acknowledgement.
    controller
        .board_mut()
        .upsert(item(1, 2, RunState::Completed)?)?;
    assert_eq!(controller.continue_work(&mut owner, request(1))?, first);
    assert_eq!(owner.calls, 1);
    Ok(())
}

#[test]
fn invalid_selection_never_reaches_the_owner() -> TestResult {
    let mut controller = coordinator()?;
    let mut owner = RecordingOwner::default();
    for (change, error) in [
        (
            ContinueRequest::new(command(1), scope(2, 1), run(1), 1),
            WorkspaceError::ScopeMismatch,
        ),
        (
            ContinueRequest::new(command(1), scope(1, 2), run(1), 1),
            WorkspaceError::ScopeMismatch,
        ),
        (
            ContinueRequest::new(command(1), scope(1, 1), run(2), 1),
            WorkspaceError::MissingWork,
        ),
        (
            ContinueRequest::new(command(1), scope(1, 1), run(1), 0),
            WorkspaceError::StaleWork,
        ),
    ] {
        assert_eq!(controller.continue_work(&mut owner, change), Err(error));
    }
    assert_eq!(owner.calls, 0);
    Ok(())
}

#[test]
fn terminal_and_cancelling_work_cannot_dispatch() -> TestResult {
    let mut owner = RecordingOwner::default();
    for state in RunState::ALL
        .into_iter()
        .filter(|state| state.is_terminal() || *state == RunState::Cancelling)
    {
        let mut board = WorkBoard::new(scope(1, 1));
        board.upsert(item(1, 1, state)?)?;
        let expected = if state.is_terminal() {
            WorkspaceError::TerminalWork
        } else {
            WorkspaceError::CancellingWork
        };
        assert_eq!(
            ContinueCoordinator::new(board).continue_work(&mut owner, request(1)),
            Err(expected)
        );
    }
    assert_eq!(owner.calls, 0);
    Ok(())
}

#[test]
fn uncertain_or_refused_attempt_cannot_reuse_identity_for_other_content() -> TestResult {
    for error in [WorkspaceError::OutcomeUnknown, WorkspaceError::OwnerRefused] {
        let mut controller = coordinator()?;
        let mut owner = RecordingOwner {
            refusal: Some(error),
            ..RecordingOwner::default()
        };
        assert_eq!(controller.continue_work(&mut owner, request(1)), Err(error));
        assert_eq!(
            controller.continue_work(
                &mut owner,
                ContinueRequest::new(command(1), scope(1, 1), run(2), 1)
            ),
            Err(WorkspaceError::ConflictingRetry)
        );
        assert_eq!(owner.calls, 1);
        owner.refusal = None;
        assert!(matches!(
            controller.continue_work(&mut owner, request(1))?,
            ContinueOutcome::Queued(_)
        ));
        assert_eq!(owner.calls, 2);
    }
    Ok(())
}

#[test]
fn owner_binding_changes_and_foreign_receipts_are_refused() -> TestResult {
    for current in [
        version(2),
        WorkVersion::new(1, version(1).baseline, 8),
        WorkVersion::new(1, ContentDigest::over(&[b"changed"]), 7),
    ] {
        let mut controller = coordinator()?;
        let mut owner = RecordingOwner {
            current: Some(current),
            ..RecordingOwner::default()
        };
        assert_eq!(
            controller.continue_work(&mut owner, request(1)),
            Err(WorkspaceError::StaleWork)
        );
        assert_eq!(
            controller.board().get(run(1)).map(WorkItem::state),
            Some(RunState::Admitted)
        );
    }
    let mut controller = coordinator()?;
    let mut owner = RecordingOwner {
        corrupt_receipt: true,
        ..RecordingOwner::default()
    };
    assert_eq!(
        controller.continue_work(&mut owner, request(1)),
        Err(WorkspaceError::InvalidReceipt)
    );
    assert_eq!(
        controller.continue_work(&mut owner, request(1)),
        Err(WorkspaceError::InvalidReceipt)
    );
    assert_eq!(owner.calls, 2);
    Ok(())
}

#[test]
fn retry_capacity_never_silently_evicts_an_existing_identity() -> TestResult {
    let mut controller = coordinator()?;
    for number in 0..MAX_WORK_ITEMS {
        assert_eq!(
            controller.continue_work(&mut MissingMooAdapter::default(), request(number))?,
            ContinueOutcome::AwaitingDependency
        );
    }
    assert_eq!(
        controller.continue_work(&mut MissingMooAdapter::default(), request(MAX_WORK_ITEMS)),
        Err(WorkspaceError::CapacityExceeded)
    );
    assert_eq!(
        controller.continue_work(&mut MissingMooAdapter::default(), request(0))?,
        ContinueOutcome::AwaitingDependency
    );
    assert_eq!(
        controller.continue_work(
            &mut MissingMooAdapter::default(),
            ContinueRequest::new(command(0), scope(1, 1), run(1), 2)
        ),
        Err(WorkspaceError::ConflictingRetry)
    );
    Ok(())
}

#[test]
fn identical_run_and_command_ids_in_different_tenants_remain_separate() -> TestResult {
    let mut first = coordinator()?;
    let mut second_board = WorkBoard::new(scope(2, 1));
    second_board.upsert(WorkItem::new(
        scope(2, 1),
        run(1),
        "Private second tenant",
        version(9),
        RunState::Admitted,
    )?)?;
    let mut second = ContinueCoordinator::new(second_board);
    let mut owner = RecordingOwner::default();
    let first_outcome = first.continue_work(&mut owner, request(1))?;
    let second_outcome = second.continue_work(
        &mut owner,
        ContinueRequest::new(command(1), scope(2, 1), run(1), 9),
    )?;
    assert_ne!(first_outcome, second_outcome);
    assert_eq!(
        first.board().get(run(1)).map(WorkItem::title),
        Some("Resume my workspace")
    );
    assert_eq!(
        second.board().get(run(1)).map(WorkItem::title),
        Some("Private second tenant")
    );
    assert_eq!(owner.calls, 2);
    Ok(())
}
