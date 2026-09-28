//! E35: bounded, tenant-scoped journal catch-up with explicit resynchronization.
//!
//! Subscribe before reading history. Retain the cursor only after applying a
//! page; after a lost reply, subscribe from the last applied cursor and dedupe
//! by journal sequence. Notifications are implicit descriptors in the journal
//! index, not a second copy of facts. Overflow disconnects observation until
//! explicit resync. The host supplies authenticated tenant scope; a cursor is
//! neither a credential nor a grant. Persistence remains the journal owner's.

use core::fmt;
use std::collections::BTreeMap;
use std::mem::size_of;

use lgwks_std::json::serde::{Deserialize, Serialize};
use rocco_contracts::event::CausalRef;
use rocco_contracts::ids::{ContentDigest, TenantId};

use crate::journal::{Journal, JournalEntry};
use crate::runtime::Runtime;

/// D25 maximum page item count.
pub const MAX_PAGE_ITEMS: usize = 100;
/// D25 maximum decoded page bytes, including envelope and causal parents.
pub const MAX_PAGE_BYTES: usize = 2_097_152;
/// D25 maximum outstanding live notification descriptors.
pub const MAX_NOTIFICATIONS: usize = 256;
/// Cell 05 maximum client notification byte budget.
pub const MAX_NOTIFICATION_BYTES: usize = 8_388_608;

/// A refused consumer operation. No error advances its cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CursorError {
    /// Tenant supplied by the host differs from the cursor/subscription scope.
    TenantMismatch,
    /// The cursor's position or prefix is absent from this journal. Start an
    /// explicit full resynchronization with `subscribe(tenant, None, limits)`.
    StaleCursor,
    /// A zero, excessive or unusably small resource bound was requested.
    InvalidBounds,
    /// Live notifications overflowed. Call `resync_consumer`, which preserves
    /// the unread position while re-establishing the live subscription.
    ResyncRequired,
    /// One indivisible entry cannot fit the configured decoded page budget.
    EntryTooLarge {
        /// Required decoded bytes including envelope and causal parents.
        required: usize,
        /// Configured decoded byte ceiling.
        limit: usize,
    },
}

impl fmt::Display for CursorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TenantMismatch => formatter.write_str("consumer tenant mismatch; read refused"),
            Self::StaleCursor => formatter
                .write_str("stale cursor; explicitly subscribe from origin to resynchronize"),
            Self::InvalidBounds => formatter.write_str(
                "consumer bounds must be nonzero and within page and notification ceilings",
            ),
            Self::ResyncRequired => formatter
                .write_str("notification window overflowed; resync_consumer before reading again"),
            Self::EntryTooLarge { required, limit } => write!(
                formatter,
                "entry needs {required} decoded bytes, page permits {limit}; cursor unchanged"
            ),
        }
    }
}

impl std::error::Error for CursorError {}

/// Validated resource ceilings, immutable for a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsumerLimits {
    /// Maximum items returned by a page.
    page_items: usize,
    /// Maximum decoded bytes referenced by a page.
    page_bytes: usize,
    /// Descriptor count after intersecting count and byte budgets.
    window: usize,
}

impl Default for ConsumerLimits {
    fn default() -> Self {
        Self {
            page_items: MAX_PAGE_ITEMS,
            page_bytes: MAX_PAGE_BYTES,
            window: MAX_NOTIFICATIONS,
        }
    }
}

impl ConsumerLimits {
    /// Intersects the descriptor ceiling with the client byte budget. Each
    /// implicit descriptor is one journal offset (`usize`); no payload queues.
    ///
    /// # Errors
    /// Returns [`CursorError::InvalidBounds`] for zero or excessive ceilings,
    /// or a byte budget smaller than one descriptor.
    pub fn new(
        page_items: usize,
        page_bytes: usize,
        notifications: usize,
        notification_bytes: usize,
    ) -> Result<Self, CursorError> {
        let by_bytes = notification_bytes
            .checked_div(size_of::<usize>())
            .ok_or(CursorError::InvalidBounds)?;
        if page_items == 0
            || page_items > MAX_PAGE_ITEMS
            || page_bytes == 0
            || page_bytes > MAX_PAGE_BYTES
            || notifications == 0
            || notifications > MAX_NOTIFICATIONS
            || notification_bytes > MAX_NOTIFICATION_BYTES
            || by_bytes == 0
        {
            return Err(CursorError::InvalidBounds);
        }
        Ok(Self {
            page_items,
            page_bytes,
            window: notifications.min(by_bytes),
        })
    }
}

/// Portable checkpoint of a consumed tenant prefix. Serialized values are
/// validated against the journal on every use, never trusted as authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(crate = "lgwks_std::json::serde", deny_unknown_fields)]
pub struct ConsumerCursor {
    /// Host-authenticated scope at creation.
    tenant: TenantId,
    /// Number of consumed entries belonging to this tenant.
    consumed: usize,
    /// One-based global journal sequence of the last consumed entry, or zero.
    sequence: usize,
    /// Digest binding the entire consumed tenant prefix and global positions.
    prefix: ContentDigest,
}

impl ConsumerCursor {
    /// Last consumed global journal sequence; zero is the explicit origin.
    #[must_use]
    pub const fn sequence(&self) -> usize {
        self.sequence
    }
}

/// Caller-owned observation state. Dropping it cancels observation without
/// cancelling work, retaining payloads or leaving a registered background task.
#[derive(Debug)]
pub struct Consumer {
    /// Last delivered position; persist only after application succeeds.
    cursor: ConsumerCursor,
    /// Validated resource ceilings.
    limits: ConsumerLimits,
    /// Tenant position at subscription, or last drained live position.
    live_after: ConsumerCursor,
    /// Sticky overflow; only explicit resync can reconnect.
    disconnected: bool,
}

impl Consumer {
    /// Checkpoint to save after applying the corresponding page. Keep the
    /// previous checkpoint instead if application or transport fails.
    #[must_use]
    pub const fn cursor(&self) -> ConsumerCursor {
        self.cursor
    }
}

/// A fact borrowed directly from authoritative history, with a dedupe key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsumerRecord<'a> {
    /// Stable one-based position in the whole journal.
    sequence: usize,
    /// The original recorded command or event; never a reconstructed payload.
    entry: &'a JournalEntry,
}

impl<'a> ConsumerRecord<'a> {
    /// Global journal sequence for deduplication within this tenant/history.
    #[must_use]
    pub const fn sequence(&self) -> usize {
        self.sequence
    }
    /// Borrowed authoritative record.
    #[must_use]
    pub const fn entry(&self) -> &'a JournalEntry {
        self.entry
    }
}

/// A bounded page at one immutable journal observation.
#[derive(Debug)]
pub struct ConsumerPage<'a> {
    /// At most the configured item ceiling, with no payload copies.
    records: Vec<ConsumerRecord<'a>>,
    /// Accounted decoded bytes, including envelope and parent storage.
    decoded_bytes: usize,
    /// Whether this page consumed the tenant head observed during this call.
    caught_up: bool,
}

impl<'a> ConsumerPage<'a> {
    /// Ordered, tenant-filtered entries. Other tenants' sequence positions may
    /// form numeric gaps; no record of this tenant is omitted.
    #[must_use]
    pub fn records(&self) -> &[ConsumerRecord<'a>] {
        &self.records
    }
    /// Decoded entry bytes referenced by this page.
    #[must_use]
    pub const fn decoded_bytes(&self) -> usize {
        self.decoded_bytes
    }
    /// Caught up at this call, not a promise that publishing has stopped.
    #[must_use]
    pub const fn caught_up(&self) -> bool {
        self.caught_up
    }
}

/// One derived entry descriptor. Total descriptors never exceed journal length.
#[derive(Debug, Clone)]
struct IndexedEntry {
    /// Zero-based global journal offset.
    offset: usize,
    /// Cumulative tenant prefix digest including this entry.
    prefix: ContentDigest,
}

/// Disposable tenant lookup, never used by admission or authorization.
#[derive(Debug, Clone, Default)]
pub(crate) struct ConsumerIndex {
    /// One descriptor per retained fact, grouped by tenant in insertion order.
    tenants: BTreeMap<TenantId, Vec<IndexedEntry>>,
}

impl ConsumerIndex {
    /// Rebuilds solely from authoritative entries after journal reopen.
    pub(crate) fn rebuild(entries: &[JournalEntry]) -> Self {
        let mut index = Self::default();
        for (offset, entry) in entries.iter().enumerate() {
            index.record(entry, offset);
        }
        index
    }

    /// Adds one descriptor only after the journal has accepted an append.
    pub(crate) fn record(&mut self, entry: &JournalEntry, offset: usize) {
        let tenant = entry_tenant(entry);
        let entries = self.tenants.entry(tenant).or_default();
        let previous = entries
            .last()
            .map_or_else(|| origin(tenant).prefix, |entry| entry.prefix);
        let digest = entry_digest(entry);
        let sequence = u64::try_from(offset).unwrap_or(u64::MAX).to_be_bytes();
        let prefix = ContentDigest::over(&[previous.as_bytes(), &sequence, digest.as_bytes()]);
        entries.push(IndexedEntry { offset, prefix });
    }

    /// Looks up only the authenticated tenant's descriptors.
    fn entries(&self, tenant: TenantId) -> &[IndexedEntry] {
        self.tenants.get(&tenant).map_or(&[], Vec::as_slice)
    }
}

/// The explicit empty prefix is valid on any history for the named tenant.
fn origin(tenant: TenantId) -> ConsumerCursor {
    ConsumerCursor {
        tenant,
        consumed: 0,
        sequence: 0,
        prefix: ContentDigest::over(&[b"rocco-e35-v1", tenant.as_bytes()]),
    }
}

/// Pins the complete observed tenant head, including history identity.
fn head(tenant: TenantId, entries: &[IndexedEntry]) -> ConsumerCursor {
    match entries.last() {
        Some(entry) => ConsumerCursor {
            tenant,
            consumed: entries.len(),
            sequence: entry.offset.saturating_add(1),
            prefix: entry.prefix,
        },
        None => origin(tenant),
    }
}

/// Reads identity from the recorded envelope, never from the payload's claims.
fn entry_tenant(entry: &JournalEntry) -> TenantId {
    match *entry {
        JournalEntry::Command(ref command) => command.tenant(),
        JournalEntry::Event(ref event) => event.tenant(),
    }
}

/// Binds record identity and causal metadata as well as its content digest.
fn entry_digest(entry: &JournalEntry) -> ContentDigest {
    let (mut digest, parents) = match *entry {
        JournalEntry::Command(ref command) => {
            let payload = ContentDigest::over(&[command.content().payload()]);
            (
                ContentDigest::over(&[
                    b"command",
                    command.id().as_bytes(),
                    command.tenant().as_bytes(),
                    &[command.content().kind().tag()],
                    command.envelope_digest().as_bytes(),
                    command.content().digest().as_bytes(),
                    payload.as_bytes(),
                ]),
                command.causal_parents(),
            )
        }
        JournalEntry::Event(ref event) => {
            let payload = ContentDigest::over(&[event.payload()]);
            (
                ContentDigest::over(&[
                    b"event",
                    event.tenant().as_bytes(),
                    event.run().as_bytes(),
                    &event.sequence().value().to_be_bytes(),
                    &[event.kind().tag()],
                    event.payload_digest().as_bytes(),
                    payload.as_bytes(),
                ]),
                event.causal_parents(),
            )
        }
    };
    for parent in parents {
        digest = ContentDigest::over(&[
            digest.as_bytes(),
            parent.run.as_bytes(),
            &parent.sequence.value().to_be_bytes(),
        ]);
    }
    digest
}

/// Conservatively includes the full decoded enum, parent storage and payload.
fn decoded_size(entry: &JournalEntry) -> usize {
    let (payload, parents) = match *entry {
        JournalEntry::Command(ref command) => (
            command.content().payload().len(),
            command.causal_parents().len(),
        ),
        JournalEntry::Event(ref event) => (event.payload().len(), event.causal_parents().len()),
    };
    size_of::<JournalEntry>()
        .saturating_add(size_of::<ConsumerRecord<'_>>())
        .saturating_add(payload)
        .saturating_add(parents.saturating_mul(size_of::<CausalRef>()))
}

/// Authenticates the scope first, then validates all checkpoint fields.
fn validate<'a>(
    journal: &'a Journal,
    tenant: TenantId,
    cursor: &ConsumerCursor,
) -> Result<&'a [IndexedEntry], CursorError> {
    if tenant != cursor.tenant {
        return Err(CursorError::TenantMismatch);
    }
    let entries = journal.consumer_index.entries(tenant);
    let expected = cursor
        .consumed
        .checked_sub(1)
        .and_then(|index| entries.get(index));
    let valid = match expected {
        Some(entry) => {
            cursor.sequence == entry.offset.saturating_add(1) && cursor.prefix == entry.prefix
        }
        None => *cursor == origin(tenant),
    };
    if !valid {
        return Err(CursorError::StaleCursor);
    }
    Ok(entries)
}

impl<P, O, C> Runtime<P, O, C>
where
    P: crate::projection::ProjectionBackend + Default,
    O: crate::owner::EffectOwner,
    C: crate::clock::Clock,
{
    /// Captures the tenant's live watermark before history is read. `None`
    /// explicitly starts full replay; a supplied checkpoint resumes after it.
    ///
    /// # Errors
    /// Refuses cross-tenant and stale checkpoints with [`CursorError`].
    pub fn subscribe(
        &self,
        tenant: TenantId,
        cursor: Option<ConsumerCursor>,
        limits: ConsumerLimits,
    ) -> Result<Consumer, CursorError> {
        let cursor = cursor.unwrap_or_else(|| origin(tenant));
        let entries = validate(self.journal(), tenant, &cursor)?;
        Ok(Consumer {
            cursor,
            limits,
            live_after: head(tenant, entries),
            disconnected: false,
        })
    }

    /// Reconnects live observation after overflow without skipping unread facts.
    /// To discard a stale checkpoint explicitly start from origin instead.
    ///
    /// # Errors
    /// Refuses foreign scope or history that no longer contains the checkpoint.
    pub fn resync_consumer(
        &self,
        tenant: TenantId,
        consumer: &mut Consumer,
    ) -> Result<(), CursorError> {
        let entries = validate(self.journal(), tenant, &consumer.cursor)?;
        consumer.live_after = head(tenant, entries);
        consumer.disconnected = false;
        Ok(())
    }

    /// Reads one bounded page and advances only on success. Every append is
    /// visible through the journal index, including facts from a partial tick.
    /// A page's immutable borrow linearizes it with respect to publishing.
    /// Persist the resulting cursor only after application; retry from the old
    /// saved cursor on interruption. No journal fact is ever evicted here.
    ///
    /// # Errors
    /// Returns typed scope/staleness refusals, sticky `ResyncRequired` on live
    /// window overflow, or `EntryTooLarge` without advancing past that entry.
    pub fn consumer_page(
        &self,
        tenant: TenantId,
        consumer: &mut Consumer,
    ) -> Result<ConsumerPage<'_>, CursorError> {
        let journal = self.journal();
        let entries = validate(journal, tenant, &consumer.cursor)?;
        validate(journal, tenant, &consumer.live_after)?;
        if entries.len().saturating_sub(consumer.live_after.consumed) > consumer.limits.window {
            consumer.disconnected = true;
        }
        if consumer.disconnected {
            return Err(CursorError::ResyncRequired);
        }
        let mut cursor = consumer.cursor;
        let mut page = ConsumerPage {
            records: Vec::with_capacity(consumer.limits.page_items),
            decoded_bytes: 0,
            caught_up: false,
        };
        for indexed in entries
            .iter()
            .skip(cursor.consumed)
            .take(consumer.limits.page_items)
        {
            let entry = journal
                .entries()
                .get(indexed.offset)
                .ok_or(CursorError::StaleCursor)?;
            let bytes = decoded_size(entry);
            if bytes > consumer.limits.page_bytes && page.records.is_empty() {
                return Err(CursorError::EntryTooLarge {
                    required: bytes,
                    limit: consumer.limits.page_bytes,
                });
            }
            let total = page.decoded_bytes.saturating_add(bytes);
            if total > consumer.limits.page_bytes {
                break;
            }
            cursor.consumed = cursor.consumed.saturating_add(1);
            cursor.sequence = indexed.offset.saturating_add(1);
            cursor.prefix = indexed.prefix;
            page.records.push(ConsumerRecord {
                sequence: cursor.sequence,
                entry,
            });
            page.decoded_bytes = total;
        }
        page.caught_up = cursor.consumed == entries.len();
        consumer.cursor = cursor;
        if cursor.consumed > consumer.live_after.consumed {
            consumer.live_after = cursor;
        }
        Ok(page)
    }
}
