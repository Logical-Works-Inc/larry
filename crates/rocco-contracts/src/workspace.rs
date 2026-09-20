//! Bounded workspace projection and the real Continue dispatch boundary.
//!
//! R20/R44/R51/R52/R64, D14/D25/D28: surfaces share one coordinator,
//! while Moo owns workspace claims and effects. This disposable projection
//! is not durable admission, a memory store, or an authority issuer. A
//! selection and an explicit Continue command must call the same method.
//!
//! Reuse: baseline/fence refusal adapted from forge-harness
//! `crates/forge-runtime/src/coordinator/scheduler/moo.rs:201-239`.
//! Its synthetic pasture allocation is deliberately excluded: the real Moo
//! owner must atomically revalidate authority, baseline and fence at dispatch.
//!
//! Adapted portions: Copyright (c) 2026 srinji, MIT License.
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to deal
//! in the Software without restriction, including without limitation the rights
//! to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//! The above copyright notice and this permission notice shall be included in all
//! copies or substantial portions of the Software.
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//! SOFTWARE.

use std::collections::BTreeMap;

use crate::ids::{CommandId, ContentDigest, OperationId, RunId, TenantId, WorkspaceId};
use crate::state::RunState;

/// Maximum work descriptors and retained Continue identities per view.
pub const MAX_WORK_ITEMS: usize = 256;
/// Maximum UTF-8 bytes in a work title. Bodies belong in a paged detail view.
pub const MAX_WORK_TITLE_BYTES: usize = 256;

/// Scope supplied by the authenticated host, never inferred from model text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct WorkspaceScope {
    /// Tenant isolation boundary.
    pub tenant: TenantId,
    /// Workspace owned by Moo.
    pub workspace: WorkspaceId,
}

/// Owner-observed version binding; none of these fields grants authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct WorkVersion {
    /// Monotonic projection revision, used to reject stale selection.
    pub revision: u64,
    /// Baseline the owner must compare before admitting continuation.
    pub baseline: ContentDigest,
    /// Moo claim fence the owner must revalidate atomically with admission.
    pub fence: u64,
}

impl WorkVersion {
    /// Refuses a changed revision, baseline or fence before dispatch.
    ///
    /// # Errors
    /// Returns [`WorkspaceError::StaleWork`] if any binding differs. The
    /// owner must repeat this check at its transaction/effect boundary.
    pub fn verify_current(self, current: Self) -> Result<(), WorkspaceError> {
        if self != current {
            return Err(WorkspaceError::StaleWork);
        }
        Ok(())
    }
}

/// One validated, bounded work descriptor. It is a projection, not a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItem {
    scope: WorkspaceScope,
    run: RunId,
    title: Box<str>,
    version: WorkVersion,
    state: RunState,
}

impl WorkItem {
    /// Builds a descriptor without accepting terminal escape/control bytes.
    ///
    /// # Errors
    /// Returns [`WorkspaceError::InvalidTitle`] for empty, blank, overlong
    /// or control-containing titles. The bound is bytes, not graphemes.
    pub fn new(
        scope: WorkspaceScope,
        run: RunId,
        title: &str,
        version: WorkVersion,
        state: RunState,
    ) -> Result<Self, WorkspaceError> {
        if title.trim().is_empty()
            || title.len() > MAX_WORK_TITLE_BYTES
            || title.chars().any(char::is_control)
        {
            return Err(WorkspaceError::InvalidTitle);
        }
        Ok(Self {
            scope,
            run,
            title: title.into(),
            version,
            state,
        })
    }

    /// Descriptor identity.
    #[must_use]
    pub const fn run(&self) -> RunId {
        self.run
    }
    /// Display label; never an instruction or authority input.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
    /// Observed owner binding.
    #[must_use]
    pub const fn version(&self) -> WorkVersion {
        self.version
    }
    /// Observed run state, never advanced by merely clicking a card.
    #[must_use]
    pub const fn state(&self) -> RunState {
        self.state
    }
}

/// Tenant/workspace-isolated, bounded projection rebuilt from its owner.
#[derive(Debug)]
pub struct WorkBoard {
    scope: WorkspaceScope,
    items: BTreeMap<RunId, WorkItem>,
}

impl WorkBoard {
    /// Starts an empty view. No process-global state, I/O or worker is created.
    #[must_use]
    pub fn new(scope: WorkspaceScope) -> Self {
        Self {
            scope,
            items: BTreeMap::new(),
        }
    }

    /// Scope of all rows in this view.
    #[must_use]
    pub const fn scope(&self) -> WorkspaceScope {
        self.scope
    }

    /// Adds or replaces an owner-provided snapshot atomically.
    ///
    /// # Errors
    /// Refuses another scope, capacity overflow, stale/conflicting revisions,
    /// or rewriting a terminal disposition. A rejected update changes nothing.
    /// Gaps in snapshot revisions are permitted: this is not an event log.
    pub fn upsert(&mut self, item: WorkItem) -> Result<(), WorkspaceError> {
        if item.scope != self.scope {
            return Err(WorkspaceError::ScopeMismatch);
        }
        if let Some(previous) = self.items.get(&item.run) {
            if item.version.revision < previous.version.revision {
                return Err(WorkspaceError::StaleWork);
            }
            if item.version.revision == previous.version.revision && previous != &item {
                return Err(WorkspaceError::ConflictingSnapshot);
            }
            if previous.state.is_terminal() && previous.state != item.state {
                return Err(WorkspaceError::TerminalWork);
            }
        } else if self.items.len() >= MAX_WORK_ITEMS {
            return Err(WorkspaceError::CapacityExceeded);
        }
        self.items.insert(item.run, item);
        Ok(())
    }

    /// Looks up work only inside this view's fixed scope.
    #[must_use]
    pub fn get(&self, run: RunId) -> Option<&WorkItem> {
        self.items.get(&run)
    }

    /// Stable identity ordering, independent of arrival order.
    #[must_use]
    pub fn items(&self) -> impl ExactSizeIterator<Item = &WorkItem> {
        self.items.values()
    }
}

/// A human Continue action, bound to the exact card the human selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContinueRequest {
    /// Caller-retained idempotency identity; reuse unchanged on transport retry.
    pub command: CommandId,
    /// Authenticated scope provided by the host.
    pub scope: WorkspaceScope,
    /// Work selected by the human.
    pub run: RunId,
    /// Snapshot revision visible at selection time.
    pub expected_revision: u64,
}

/// Owner acknowledgement of a queued request; not proof of completed work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContinueReceipt {
    /// Exact request accepted by the owner.
    pub request: ContinueRequest,
    /// Exact baseline/fence binding accepted by the owner.
    pub version: WorkVersion,
    /// Stable owner operation identity for reconciliation.
    pub operation: OperationId,
}

/// Visible dispatch outcome, kept separate from the observed run state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContinueOutcome {
    /// No owner connection is ready; nothing was admitted or executed.
    AwaitingDependency,
    /// The owner acknowledged the queue entry; execution is still unverified.
    Queued(ContinueReceipt),
}

/// Narrow owner boundary, replaceable without changing the UI coordinator.
///
/// Implementations must authenticate scope, revalidate current authority and
/// Moo claim/baseline/fence, and durably deduplicate the exact request inside
/// their admission transaction. They may not turn a UI snapshot into a grant.
/// A transport ambiguity is an error; retries preserve identity. Logical-DB
/// and real Moo integration remain external prerequisites, not supplied here.
pub trait MooWorkspaceAdapter {
    /// Attempts the actual owner handoff, with no fabricated success fallback.
    ///
    /// # Errors
    /// Returns a typed owner refusal or unavailable/ambiguous transport error.
    fn continue_work(
        &mut self,
        request: ContinueRequest,
        version: WorkVersion,
    ) -> Result<ContinueOutcome, WorkspaceError>;
}

/// Honest disconnected boundary used until the real owner adapter is ready.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct MissingMooAdapter;

impl MooWorkspaceAdapter for MissingMooAdapter {
    fn continue_work(
        &mut self,
        _request: ContinueRequest,
        _version: WorkVersion,
    ) -> Result<ContinueOutcome, WorkspaceError> {
        Ok(ContinueOutcome::AwaitingDependency)
    }
}

#[derive(Debug)]
struct Attempt {
    request: ContinueRequest,
    version: WorkVersion,
    receipt: Option<ContinueReceipt>,
}

/// Single Continue route shared by selection, keyboard and explicit commands.
///
/// Owns at most 256 descriptors and 256 retry identities until view teardown.
/// It never evicts dedup evidence to admit more work. Durable retry identity
/// across restart belongs to the owner, not this disposable view.
#[derive(Debug)]
pub struct ContinueCoordinator {
    board: WorkBoard,
    attempts: BTreeMap<CommandId, Attempt>,
}

impl ContinueCoordinator {
    /// Takes ownership of an already-scoped projection.
    #[must_use]
    pub fn new(board: WorkBoard) -> Self {
        Self {
            board,
            attempts: BTreeMap::new(),
        }
    }

    /// Current read-only projection.
    #[must_use]
    pub const fn board(&self) -> &WorkBoard {
        &self.board
    }

    /// Receives later snapshots through the board's validated update boundary.
    pub fn board_mut(&mut self) -> &mut WorkBoard {
        &mut self.board
    }

    /// Issues Continue to the real owner boundary, or reports its absence.
    ///
    /// # Errors
    /// Refuses wrong scope, unknown/terminal/cancelling work, stale revision,
    /// changed content under one command ID, capacity exhaustion, mismatched
    /// owner receipts, and owner errors. Every refusal preserves observed run
    /// state. A queued replay returns its original receipt without redispatch.
    pub fn continue_work(
        &mut self,
        adapter: &mut impl MooWorkspaceAdapter,
        request: ContinueRequest,
    ) -> Result<ContinueOutcome, WorkspaceError> {
        if request.scope != self.board.scope {
            return Err(WorkspaceError::ScopeMismatch);
        }
        if let Some(attempt) = self.attempts.get(&request.command) {
            if attempt.request != request {
                return Err(WorkspaceError::ConflictingRetry);
            }
            if let Some(receipt) = attempt.receipt {
                return Ok(ContinueOutcome::Queued(receipt));
            }
        } else if self.attempts.len() >= MAX_WORK_ITEMS {
            return Err(WorkspaceError::CapacityExceeded);
        }
        let item = self
            .board
            .get(request.run)
            .ok_or(WorkspaceError::MissingWork)?;
        if item.version.revision != request.expected_revision {
            return Err(WorkspaceError::StaleWork);
        }
        if item.state.is_terminal() {
            return Err(WorkspaceError::TerminalWork);
        }
        if item.state == RunState::Cancelling {
            return Err(WorkspaceError::CancellingWork);
        }
        let version = item.version;
        let attempt = self.attempts.entry(request.command).or_insert(Attempt {
            request,
            version,
            receipt: None,
        });
        attempt.version.verify_current(version)?;
        let outcome = adapter.continue_work(request, version)?;
        if let ContinueOutcome::Queued(receipt) = outcome {
            if receipt.request != request || receipt.version != version {
                return Err(WorkspaceError::InvalidReceipt);
            }
            attempt.receipt = Some(receipt);
        }
        Ok(outcome)
    }
}

/// Explicit safe failures of projection and continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WorkspaceError {
    /// Wrong authenticated tenant or workspace.
    #[error("work belongs to another tenant or workspace")]
    ScopeMismatch,
    /// Empty, control-containing or oversized title.
    #[error("work title must be nonblank, control-free and at most 256 UTF-8 bytes")]
    InvalidTitle,
    /// Unknown run in this scoped projection.
    #[error("work is absent from this workspace projection")]
    MissingWork,
    /// Revision, baseline or fence no longer matches.
    #[error("work changed; refresh before continuing")]
    StaleWork,
    /// Same revision describes different content.
    #[error("same work revision carries conflicting content")]
    ConflictingSnapshot,
    /// Terminal state cannot be continued or rewritten.
    #[error("terminal work cannot be continued or rewritten")]
    TerminalWork,
    /// Cancellation is still settling owned work.
    #[error("cancellation is in progress; continuation is refused")]
    CancellingWork,
    /// Bounded view or identity retention is full.
    #[error("workspace projection capacity reached; resynchronize with the owner")]
    CapacityExceeded,
    /// A retry changed its target or binding.
    #[error("continue identity was reused with different content")]
    ConflictingRetry,
    /// Owner acknowledgement did not bind the requested operation.
    #[error("owner receipt does not match the continuation request")]
    InvalidReceipt,
    /// Owner refused current authority, claim or effect admission.
    #[error("workspace owner refused continuation")]
    OwnerRefused,
    /// Transport cannot establish whether the owner accepted the request.
    #[error("continuation outcome is unknown; reconcile with the same identity")]
    OutcomeUnknown,
}
