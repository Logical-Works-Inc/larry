//! The harness tree, its canonical encoding and its content hash.

use std::collections::BTreeMap;

use crate::ids::ContentDigest;

use super::component::HarnessComponent;
use super::error::HarnessError;
use super::{FORMAT_VERSION, HASH_DOMAIN, MAX_ENCODED_BYTES, MAX_NAME_BYTES, MAX_TOTAL_ENTRIES};

/// The BLAKE3 identity of a harness tree: a hash over the domain tag and the canonical bytes.
///
/// The hash covers the tree only. Which tenant adopted it is a separate record (larry#61 part 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(lgwks_std::json::serde::Serialize))]
#[cfg_attr(feature = "serde", derive(lgwks_std::json::serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
pub struct HarnessHash(ContentDigest);

impl HarnessHash {
    /// The raw 32 hash bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    /// Lowercase hex form (64 characters).
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }
}

impl core::fmt::Display for HarnessHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(&self.0, f)
    }
}

/// A validated harness: bounded entries keyed by component and name, kept in canonical order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HarnessTree {
    /// Entries sorted by (component tag, name bytes); the map keeps that order for any insertion order.
    entries: BTreeMap<(HarnessComponent, String), Vec<u8>>,
}

/// Whether a name obeys the charset and length rule: 1 to 64 bytes of `a-z`, `0-9`, `-`, `_`,
/// not starting with `-`.
pub(super) fn name_is_valid(name: &str) -> bool {
    let bytes = name.as_bytes();
    let first_ok = bytes.first().is_some_and(|first| *first != b'-');
    first_ok
        && bytes.len() <= MAX_NAME_BYTES
        && bytes
            .iter()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'))
}

/// A cursor over encoded bytes that never indexes and never panics.
struct Reader<'bytes> {
    /// What has not been read yet.
    rest: &'bytes [u8],
}

impl<'bytes> Reader<'bytes> {
    /// Takes the next `count` bytes.
    fn take(&mut self, count: usize) -> Result<&'bytes [u8], HarnessError> {
        let (head, tail) = self
            .rest
            .split_at_checked(count)
            .ok_or(HarnessError::Truncated)?;
        self.rest = tail;
        Ok(head)
    }

    /// Takes one byte.
    fn byte(&mut self) -> Result<u8, HarnessError> {
        let (first, tail) = self.rest.split_first().ok_or(HarnessError::Truncated)?;
        self.rest = tail;
        Ok(*first)
    }

    /// Takes a big-endian `u32` length as a `usize`.
    fn length(&mut self) -> Result<usize, HarnessError> {
        let (head, tail) = self
            .rest
            .split_first_chunk::<4>()
            .ok_or(HarnessError::Truncated)?;
        self.rest = tail;
        usize::try_from(u32::from_be_bytes(*head)).map_err(|_| HarnessError::Truncated)
    }
}

/// A length as four big-endian bytes. Every length written is under the bounds, so the saturated
/// fallback is unreachable; if it were reached, decoding refuses the result.
fn length_bytes(len: usize) -> [u8; 4] {
    u32::try_from(len).map_or([u8::MAX; 4], u32::to_be_bytes)
}

impl HarnessTree {
    /// The empty tree.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Puts one entry, replacing any entry of the same component and name.
    ///
    /// # Errors
    ///
    /// [`HarnessError::InvalidName`], [`HarnessError::Oversize`] or [`HarnessError::TooManyEntries`];
    /// the tree is unchanged on refusal.
    pub fn insert(
        &mut self,
        component: HarnessComponent,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), HarnessError> {
        if !name_is_valid(name) {
            return Err(HarnessError::InvalidName { component });
        }
        let limit = component.max_entry_bytes();
        if bytes.len() > limit {
            return Err(HarnessError::Oversize {
                component,
                limit,
                actual: bytes.len(),
            });
        }
        let key = (component, name.to_owned());
        if !self.entries.contains_key(&key) && self.count(component) >= component.max_entries() {
            return Err(HarnessError::TooManyEntries {
                component,
                limit: component.max_entries(),
            });
        }
        self.entries.insert(key, bytes.to_vec());
        Ok(())
    }

    /// Removes one entry; absent entries are a no-op. Returns whether an entry was removed.
    pub(super) fn remove(&mut self, component: HarnessComponent, name: &str) -> bool {
        self.entries.remove(&(component, name.to_owned())).is_some()
    }

    /// The bytes of one entry.
    #[must_use]
    pub fn get(&self, component: HarnessComponent, name: &str) -> Option<&[u8]> {
        self.entries
            .get(&(component, name.to_owned()))
            .map(Vec::as_slice)
    }

    /// Entries held of one component.
    #[must_use]
    pub fn count(&self, component: HarnessComponent) -> usize {
        self.entries.keys().filter(|key| key.0 == component).count()
    }

    /// Entries held in all.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the tree holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The canonical bytes: version byte, `u32` entry count, then per entry in sorted order the
    /// component tag, a name length byte, the name, a `u32` data length and the data. Lengths are
    /// big-endian.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![FORMAT_VERSION];
        out.extend_from_slice(&length_bytes(self.entries.len()));
        for (key, bytes) in &self.entries {
            out.push(key.0.tag());
            // Names are at most 64 bytes, so the low byte of the big-endian length is the length.
            out.extend(length_bytes(key.1.len()).iter().skip(3));
            out.extend_from_slice(key.1.as_bytes());
            out.extend_from_slice(&length_bytes(bytes.len()));
            out.extend_from_slice(bytes);
        }
        out
    }

    /// Reads canonical bytes. Anything `encode` could not have produced is refused, so
    /// `encode(decode(bytes)) == bytes` for every accepted input.
    ///
    /// # Errors
    ///
    /// A [`HarnessError`] naming the first fault: version, count, tag, name, size, order, truncation
    /// or trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, HarnessError> {
        if bytes.len() > MAX_ENCODED_BYTES {
            return Err(HarnessError::EncodingTooLarge {
                limit: MAX_ENCODED_BYTES,
                actual: bytes.len(),
            });
        }
        let mut reader = Reader { rest: bytes };
        let version = reader.byte()?;
        if version != FORMAT_VERSION {
            return Err(HarnessError::UnsupportedVersion { got: version });
        }
        let declared = reader.length()?;
        if declared > MAX_TOTAL_ENTRIES {
            return Err(HarnessError::TreeTooLarge {
                limit: MAX_TOTAL_ENTRIES,
                actual: declared,
            });
        }
        let mut tree = Self::empty();
        let mut previous: Option<(HarnessComponent, &str)> = None;
        for _ in 0..declared {
            let tag = reader.byte()?;
            let component =
                HarnessComponent::from_tag(tag).ok_or(HarnessError::UnknownComponent { tag })?;
            let name_len = usize::from(reader.byte()?);
            let name = core::str::from_utf8(reader.take(name_len)?)
                .map_err(|_| HarnessError::InvalidName { component })?;
            let data_len = reader.length()?;
            if data_len > component.max_entry_bytes() {
                return Err(HarnessError::Oversize {
                    component,
                    limit: component.max_entry_bytes(),
                    actual: data_len,
                });
            }
            let data = reader.take(data_len)?;
            if let Some(before) = previous {
                match (component, name).cmp(&before) {
                    core::cmp::Ordering::Less => return Err(HarnessError::Unsorted),
                    core::cmp::Ordering::Equal => return Err(HarnessError::Duplicate),
                    core::cmp::Ordering::Greater => {}
                }
            }
            tree.insert(component, name, data)?;
            previous = Some((component, name));
        }
        if !reader.rest.is_empty() {
            return Err(HarnessError::TrailingBytes);
        }
        Ok(tree)
    }

    /// The content hash: BLAKE3 over the domain tag and the canonical bytes.
    #[must_use]
    pub fn hash(&self) -> HarnessHash {
        HarnessHash(ContentDigest::over(&[HASH_DOMAIN, &self.encode()]))
    }
}
