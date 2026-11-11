//! The closed set of harness components, their stable tags and their bounds.

/// One of the eight kinds of thing a harness holds.
///
/// The set is closed: a ninth kind is a specification change. Declaration order equals tag order,
/// so the derived `Ord` is the canonical sort order of the encoding.
#[cfg_attr(
    feature = "serde",
    derive(lgwks_std::json::serde::Serialize, lgwks_std::json::serde::Deserialize)
)]
#[cfg_attr(feature = "serde", serde(crate = "lgwks_std::json::serde"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum HarnessComponent {
    /// The system prompt (one entry).
    SystemPrompt,
    /// Guardrail text the model is held to.
    Guardrails,
    /// Standing memory notes.
    Memory,
    /// Per-tool usage guidance.
    ToolGuidance,
    /// Subagent definitions.
    Subagents,
    /// Reusable skills.
    Skills,
    /// Scripts the harness may run.
    Scripts,
    /// Plugin manifests.
    Plugins,
}

impl HarnessComponent {
    /// Every kind, in tag order.
    pub const ALL: [Self; 8] = [
        Self::SystemPrompt,
        Self::Guardrails,
        Self::Memory,
        Self::ToolGuidance,
        Self::Subagents,
        Self::Skills,
        Self::Scripts,
        Self::Plugins,
    ];

    /// Stable canonical tag; independent of Rust enum layout.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::SystemPrompt => 0,
            Self::Guardrails => 1,
            Self::Memory => 2,
            Self::ToolGuidance => 3,
            Self::Subagents => 4,
            Self::Skills => 5,
            Self::Scripts => 6,
            Self::Plugins => 7,
        }
    }

    /// The kind a tag names, or `None` for an unknown tag.
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::SystemPrompt),
            1 => Some(Self::Guardrails),
            2 => Some(Self::Memory),
            3 => Some(Self::ToolGuidance),
            4 => Some(Self::Subagents),
            5 => Some(Self::Skills),
            6 => Some(Self::Scripts),
            7 => Some(Self::Plugins),
            _ => None,
        }
    }

    /// The directory segment of this kind's surface paths (`skills/<name>`).
    #[must_use]
    pub const fn dir(self) -> &'static str {
        match self {
            Self::SystemPrompt => "system-prompt",
            Self::Guardrails => "guardrails",
            Self::Memory => "memory",
            Self::ToolGuidance => "tool-guidance",
            Self::Subagents => "subagents",
            Self::Skills => "skills",
            Self::Scripts => "scripts",
            Self::Plugins => "plugins",
        }
    }

    /// The kind whose directory segment this is, or `None` when off the surface.
    #[must_use]
    pub fn from_dir(dir: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.dir() == dir)
    }

    /// The most entries one tree may hold of this kind.
    #[must_use]
    pub const fn max_entries(self) -> usize {
        match self {
            Self::SystemPrompt => 1,
            Self::Guardrails | Self::Scripts => 32,
            Self::Memory | Self::ToolGuidance | Self::Skills => 64,
            Self::Subagents | Self::Plugins => 16,
        }
    }

    /// The most bytes one entry of this kind may hold.
    #[must_use]
    pub const fn max_entry_bytes(self) -> usize {
        match self {
            Self::SystemPrompt | Self::Skills => 32 * 1024,
            Self::Guardrails | Self::ToolGuidance => 8 * 1024,
            Self::Memory | Self::Subagents | Self::Scripts => 16 * 1024,
            Self::Plugins => 64 * 1024,
        }
    }

    /// Names the host owns for this kind; a tenant policy decides whether an edit may write them.
    #[must_use]
    pub const fn reserved_names(self) -> &'static [&'static str] {
        match self {
            Self::Scripts => &["entrypoint", "setup", "teardown"],
            Self::Subagents => &["larry", "root"],
            Self::SystemPrompt
            | Self::Guardrails
            | Self::Memory
            | Self::ToolGuidance
            | Self::Skills
            | Self::Plugins => &[],
        }
    }
}
