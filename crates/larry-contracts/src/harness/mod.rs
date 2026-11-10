//! The harness as a typed, content-addressed value (larry#61 part 1, ADR 0028).
//!
//! A harness is a [`HarnessTree`]: bounded entries of eight closed kinds. Its identity is a
//! [`HarnessHash`], BLAKE3 over a domain tag and the canonical bytes, so the same tree always has the
//! same hash whatever order it was built in. A [`HarnessVersion`] records a hash with its parent,
//! author and reason. [`validate_edit`] is the one way an edit becomes a new tree: it is pure, and a
//! refusal is a typed [`HarnessError`] with the current tree untouched. Adoption through the fence is
//! part 2; nothing here reads a disk or a clock.
//!
//! Surface rule: a path is exactly `<component>/<name>`. Anything else (a third segment, an unknown
//! component, an empty, `.` or `..` name, a backslash) is [`HarnessError::OffSurface`]. A name on the
//! surface must be 1 to 64 bytes of `a-z`, `0-9`, `-`, `_` and not start with `-`, or it is
//! [`HarnessError::InvalidName`]. The charset has no `.`, so no name can climb or hide.
//!
//! | component | tag | dir | max entries | max bytes per entry | reserved names |
//! |---|---|---|---|---|---|
//! | `SystemPrompt` | 0 | `system-prompt` | 1 | 32768 | none |
//! | `Guardrails` | 1 | `guardrails` | 32 | 8192 | none |
//! | `Memory` | 2 | `memory` | 64 | 16384 | none |
//! | `ToolGuidance` | 3 | `tool-guidance` | 64 | 8192 | none |
//! | `Subagents` | 4 | `subagents` | 16 | 16384 | `larry`, `root` |
//! | `Skills` | 5 | `skills` | 64 | 32768 | none |
//! | `Scripts` | 6 | `scripts` | 32 | 16384 | `entrypoint`, `setup`, `teardown` |
//! | `Plugins` | 7 | `plugins` | 16 | 65536 | none |
//!
//! The tree holds at most 289 entries and about 5.8 MB of content. An edit holds at most 128
//! operations and a version reason at most 512 bytes. The encoding is a version byte, a big-endian
//! `u32` entry count, then entries sorted by (tag, name): tag, name length byte, name, `u32` data
//! length, data. Decoding refuses anything `encode` could not produce.

mod component;
mod edit;
mod error;
mod tree;
mod version;

pub use component::HarnessComponent;
pub use edit::{HarnessEdit, HarnessOp, HarnessPolicy, validate_edit};
pub use error::HarnessError;
pub use tree::{HarnessHash, HarnessTree};
pub use version::{HarnessAuthor, HarnessVersion};

/// The encoding format version byte.
const FORMAT_VERSION: u8 = 1;
/// The domain tag hashed ahead of the canonical bytes, so a harness hash is never another digest.
const HASH_DOMAIN: &[u8] = b"larry.harness.tree.v1\0";
/// The longest entry name in bytes.
const MAX_NAME_BYTES: usize = 64;
/// The most entries any tree may hold (the sum of the per-component limits).
const MAX_TOTAL_ENTRIES: usize = 289;
/// The most operations in one edit.
const MAX_EDIT_OPS: usize = 128;
/// The longest version reason in bytes.
const MAX_REASON_BYTES: usize = 512;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_and_tags_agree_with_the_module_doc() {
        let sum: usize = HarnessComponent::ALL
            .iter()
            .map(|kind| kind.max_entries())
            .sum();
        assert_eq!(sum, MAX_TOTAL_ENTRIES, "module doc and bound must agree");
        let tags: Vec<u8> = HarnessComponent::ALL
            .iter()
            .map(|kind| kind.tag())
            .collect();
        assert_eq!(tags, vec![0, 1, 2, 3, 4, 5, 6, 7], "tags are 0..8 in order");
        assert!(
            HarnessComponent::ALL.windows(2).all(|pair| pair
                .first()
                .zip(pair.get(1))
                .is_some_and(|(low, high)| low < high)),
            "derived Ord must follow tag order"
        );
        for kind in HarnessComponent::ALL {
            assert_eq!(
                HarnessComponent::from_tag(kind.tag()),
                Some(kind),
                "tag roundtrip"
            );
            assert_eq!(
                HarnessComponent::from_dir(kind.dir()),
                Some(kind),
                "dir roundtrip"
            );
        }
    }
}
