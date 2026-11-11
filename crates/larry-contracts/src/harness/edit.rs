//! Edits, the tenant policy and the pure validating reducer.

use std::collections::BTreeSet;

use super::MAX_EDIT_OPS;
use super::component::HarnessComponent;
use super::error::HarnessError;
use super::tree::{HarnessTree, name_is_valid};

/// One operation of an edit, addressed by surface path `<component>/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HarnessOp {
    /// Write the bytes at the path, replacing any entry there.
    Put {
        /// The surface path, for example `skills/refactor`.
        path: String,
        /// The entry content.
        bytes: Vec<u8>,
    },
    /// Remove the entry at the path; removing an absent entry is a no-op.
    Remove {
        /// The surface path.
        path: String,
    },
}

/// A bounded, ordered list of operations applied as one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessEdit {
    /// The operations, applied in order.
    ops: Vec<HarnessOp>,
}

impl HarnessEdit {
    /// Builds an edit.
    ///
    /// # Errors
    ///
    /// [`HarnessError::EditTooLarge`] past [`MAX_EDIT_OPS`] operations.
    pub fn new(ops: Vec<HarnessOp>) -> Result<Self, HarnessError> {
        if ops.len() > MAX_EDIT_OPS {
            return Err(HarnessError::EditTooLarge {
                limit: MAX_EDIT_OPS,
                actual: ops.len(),
            });
        }
        Ok(Self { ops })
    }
}

/// What a tenant forbids. The default forbids no component and refuses reserved names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HarnessPolicy {
    /// Components whose entries may not be written.
    forbidden: BTreeSet<HarnessComponent>,
    /// Whether reserved names (scripts and subagents the host owns) may be written.
    reserved_allowed: bool,
}

impl HarnessPolicy {
    /// The default policy: nothing forbidden, reserved names refused.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forbids writing any entry of `component`. Removing entries stays allowed.
    #[must_use]
    pub fn forbidding(mut self, component: HarnessComponent) -> Self {
        self.forbidden.insert(component);
        self
    }

    /// Lets edits write reserved names.
    #[must_use]
    pub const fn allowing_reserved(mut self) -> Self {
        self.reserved_allowed = true;
        self
    }
}

/// Splits a surface path into its component and a valid name.
///
/// Off the surface: not exactly two segments, an unknown component, an empty, `.` or `..` name, or a
/// backslash. On the surface with a bad charset or length: [`HarnessError::InvalidName`].
fn parse_path(path: &str) -> Result<(HarnessComponent, &str), HarnessError> {
    let mut parts = path.split('/');
    let (Some(dir), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(HarnessError::OffSurface);
    };
    let component = HarnessComponent::from_dir(dir).ok_or(HarnessError::OffSurface)?;
    if matches!(name, "" | "." | "..") || name.contains('\\') {
        return Err(HarnessError::OffSurface);
    }
    if !name_is_valid(name) {
        return Err(HarnessError::InvalidName { component });
    }
    Ok((component, name))
}

/// Applies `edit` to `current` under `policy` and returns the new tree; `current` is never touched.
///
/// Operations run in order and the first refusal ends the edit with nothing applied. A put is checked
/// for surface, name, forbidden component, reserved name, entry size and entry count, in that order.
///
/// # Errors
///
/// The [`HarnessError`] of the first refused operation.
pub fn validate_edit(
    current: &HarnessTree,
    edit: &HarnessEdit,
    policy: &HarnessPolicy,
) -> Result<HarnessTree, HarnessError> {
    let mut next = current.clone();
    for op in &edit.ops {
        match *op {
            HarnessOp::Put {
                ref path,
                ref bytes,
            } => {
                let (component, name) = parse_path(path)?;
                if policy.forbidden.contains(&component) {
                    return Err(HarnessError::Forbidden { component });
                }
                if !policy.reserved_allowed && component.reserved_names().contains(&name) {
                    return Err(HarnessError::Reserved {
                        component,
                        name: name.to_owned(),
                    });
                }
                next.insert(component, name, bytes)?;
            }
            HarnessOp::Remove { ref path } => {
                let (component, name) = parse_path(path)?;
                next.remove(component, name);
            }
        }
    }
    Ok(next)
}
