//! E28 admission persistence and journal-derived pending input.
//!
//! This is a storage boundary, not a database implementation. The canonical
//! durable owner supplies the committed prefix and compare-and-append operation.

use std::collections::VecDeque;
use std::fmt::Debug;

use rocco_contracts::command::{Command, CommandKind};
use rocco_contracts::event::EventKind;
use rocco_contracts::ids::{RunId, TenantId};

use crate::error::RuntimeError;
use crate::journal::{Journal, JournalEntry};
use crate::payload::{CommandAcceptedPayload, WorkProposal};

/// Host-owned durable journal, shared only to preserve journal clone semantics.
///
/// The owner must atomically compare the committed entry count with `offset`,
/// append exactly this entry and make it durable before returning `Ok`. A stale
/// offset is refused. An error may mean committed-but-unacknowledged; the runtime
/// therefore stops writing until the host reloads the authoritative prefix.
/// Storage must enforce its own byte bounds and writer fencing. No database,
/// file format, background task or durability fallback is supplied here.
pub trait JournalStorage: Debug + Send + Sync {
    /// Compare-and-append one entry, returning only after durable commitment.
    ///
    /// # Errors
    /// Returns an I/O error for stale offsets, exhausted storage or uncertain
    /// persistence. The caller must recover; retrying blind is forbidden.
    fn append(&self, offset: usize, entry: &JournalEntry) -> std::io::Result<()>;
}

/// Refuses descriptor or payload-byte overflow before admitting a new identity.
pub(crate) fn check_capacity(
    ready: &VecDeque<Command>,
    command: &Command,
) -> Result<(), RuntimeError> {
    if ready.len() >= crate::MAX_READY_WORK {
        return Err(RuntimeError::ReadyWorkFull {
            capacity: crate::MAX_READY_WORK,
        });
    }
    let bytes = ready
        .iter()
        .fold(command.content().payload().len(), |total, queued| {
            total.saturating_add(queued.content().payload().len())
        });
    if bytes > crate::MAX_READY_BYTES {
        return Err(RuntimeError::ReadyBytesFull {
            capacity: crate::MAX_READY_BYTES,
        });
    }
    Ok(())
}

/// Reconstructs bounded work only from accepted, unpromoted journal entries.
pub(crate) fn pending(journal: &Journal) -> Result<VecDeque<Command>, RuntimeError> {
    let mut ready = VecDeque::new();
    for command in journal.pending() {
        check_capacity(&ready, command)?;
        ready.push_back(command.clone());
    }
    Ok(ready)
}

/// Verifies pending admission ownership before a run has any projected event.
pub(crate) fn check_pending_tenant(
    journal: &Journal,
    run: RunId,
    tenant: TenantId,
) -> Result<(), RuntimeError> {
    for command in journal.pending() {
        let proposal: WorkProposal = lgwks_std::json::from_slice(command.content().payload())?;
        if proposal.run == run && command.tenant() != tenant {
            return Err(RuntimeError::RunTenantMismatch {
                run,
                expected: command.tenant(),
                got: tenant,
            });
        }
    }
    Ok(())
}

/// Reads the most recently promoted correction, never an accepted pending one.
pub(crate) fn correction(
    journal: &Journal,
    tenant: TenantId,
    run: RunId,
) -> Result<Option<&Command>, RuntimeError> {
    for entry in journal.entries().iter().rev() {
        if let JournalEntry::Event(ref event) = *entry
            && event.tenant() == tenant
            && event.run() == run
            && event.kind() == EventKind::CommandAccepted
        {
            let payload: CommandAcceptedPayload = lgwks_std::json::from_slice(event.payload())?;
            if payload.kind == CommandKind::Steer {
                return Ok(journal.command(payload.command));
            }
        }
    }
    Ok(None)
}
