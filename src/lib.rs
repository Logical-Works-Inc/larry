//! Effect-free fixture state behind the `rocco` workspace demo and its native AppCUI view.
//! Every command changes only this owned session; no backend is connected.

use lgwks_std::json::{self, Serialize};

pub mod native;
pub mod theme;

/// Maximum command bytes, excluding the line terminator.
pub const MAX_INPUT: usize = 4096;
/// Maximum anchored notes retained by one disposable session.
const MAX_NOTES: usize = 16;

/// One immutable synthetic change, never an executable workspace operation.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(crate = "lgwks_std::json::serde")]
struct Event {
    /// Synthetic source path displayed by the inspection pane.
    file: &'static str,
    /// One-based source line displayed for this change.
    line: usize,
    /// Stable fixture trace label.
    trace: &'static str,
    /// Synthetic clone identity used by the hold boundary.
    owner: usize,
    /// Text shown as the removed side of the fixture diff.
    before: &'static str,
    /// Text shown as the added side of the fixture diff.
    after: &'static str,
}

/// Fixed script revealed by successive accepted `next` commands.
const EVENTS: [Event; 3] = [
    Event {
        file: "src/retry.rs",
        line: 12,
        trace: "fixture-01",
        owner: 1,
        before: "retry(request)",
        after: "retry_with_budget(request, 3)",
    },
    Event {
        file: "tests/retry.rs",
        line: 8,
        trace: "fixture-02",
        owner: 2,
        before: "assert!(result.is_ok())",
        after: "assert_eq!(attempts, 3)",
    },
    Event {
        file: "src/retry.rs",
        line: 18,
        trace: "fixture-03",
        owner: 1,
        before: "sleep(delay)",
        after: "sleep(delay.min(remaining))",
    },
];

/// Session-only feedback pinned to the originally selected change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(crate = "lgwks_std::json::serde")]
struct Note {
    /// One-based fixture change number.
    event: usize,
    /// Bounded operator feedback, never sent to a provider.
    text: String,
}

/// Whether the synthetic owner may reveal its next fixture change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(crate = "lgwks_std::json::serde")]
enum Work {
    /// No owner is currently held by feedback.
    Ready,
    /// Feedback prevents this owner from advancing until explicit continuation.
    Held {
        /// Synthetic owner whose subsequent changes are held.
        owner: usize,
        /// Original one-based change number receiving the feedback.
        anchor: usize,
    },
}

/// A disposable synthetic session. Construction performs no I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Demo {
    /// Number of revealed entries in the fixed fixture script.
    cursor: usize,
    /// Zero-based revealed entry selected for inspection and feedback.
    selected: Option<usize>,
    /// Whether the explicitly empty scenario is active.
    empty: bool,
    /// Whether the synthetic folder drawer is requested.
    folders: bool,
    /// Whether the synthetic agent rail is requested.
    agents: bool,
    /// Current feedback hold, independent of which entry is selected.
    work: Work,
    /// Session-owned feedback with capacity bounded by `MAX_NOTES`.
    notes: Vec<Note>,
}

impl Default for Demo {
    fn default() -> Self {
        Self {
            cursor: 0,
            selected: None,
            empty: false,
            folders: false,
            agents: false,
            work: Work::Ready,
            notes: Vec::new(),
        }
    }
}

/// Safe rejection leaves the session unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Rejected {
    /// The command exceeded [`MAX_INPUT`] bytes.
    InputBound,
    /// The command contained a control character.
    ControlCharacter,
    /// The command is not in the fixture's grammar.
    UnknownCommand,
    /// `inspect N` named an event that is not revealed, or was not a number.
    InvalidAnchor,
    /// `feedback` arrived before anything was selected to attach it to.
    NoSelection,
    /// `continue` arrived with no owner held.
    NoHeldWork,
    /// A different owner is held, so this one's work may not advance.
    HeldWork,
    /// The change list is exhausted, or the scenario is empty.
    EndOfFixture,
    /// The session already holds [`MAX_NOTES`](crate::MAX_NOTES) notes.
    NoteBound,
    /// `feedback` carried no text.
    EmptyFeedback,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}; DEMO state unchanged")
    }
}

impl std::error::Error for Rejected {}

impl Demo {
    /// Apply a command without executing the text as code or accessing a workspace.
    /// Returns false for `quit`. Rejections preserve the complete prior state.
    pub fn command(&mut self, input: &str) -> Result<bool, Rejected> {
        if input.len() > MAX_INPUT {
            return Err(Rejected::InputBound);
        }
        if input.chars().any(char::is_control) {
            return Err(Rejected::ControlCharacter);
        }
        let input = input.trim();
        match input {
            "quit" => return Ok(false),
            "" | "help" | "status" => {}
            "next" => self.advance()?,
            "folders" => self.folders = !self.folders,
            "agents" => self.agents = !self.agents,
            "continue" => self.resume()?,
            "reset" | "scenario change" => *self = Self::default(),
            "scenario empty" => {
                *self = Self {
                    empty: true,
                    ..Self::default()
                }
            }
            _ => self.targeted_command(input)?,
        }
        Ok(true)
    }

    /// Parse an inspection anchor or feedback without changing state on refusal.
    fn targeted_command(&mut self, input: &str) -> Result<(), Rejected> {
        if let Some(anchor) = input.strip_prefix("inspect ") {
            let index = anchor
                .parse::<usize>()
                .ok()
                .and_then(|number| number.checked_sub(1))
                .filter(|number| *number < self.cursor)
                .ok_or(Rejected::InvalidAnchor)?;
            self.selected = Some(index);
            Ok(())
        } else if let Some(text) = input.strip_prefix("feedback") {
            if !text.is_empty() && !text.starts_with(' ') {
                return Err(Rejected::UnknownCommand);
            }
            self.feedback(text.trim())
        } else {
            Err(Rejected::UnknownCommand)
        }
    }

    /// Reveal one scripted event unless its owner is held or the script ended.
    fn advance(&mut self) -> Result<(), Rejected> {
        if self.empty {
            return Err(Rejected::EndOfFixture);
        }
        let event = EVENTS.get(self.cursor).ok_or(Rejected::EndOfFixture)?;
        if matches!(self.work, Work::Held { owner, .. } if owner == event.owner) {
            return Err(Rejected::HeldWork);
        }
        self.selected = Some(self.cursor);
        self.cursor = self.cursor.saturating_add(1);
        Ok(())
    }

    /// Retain a bounded note and hold its selected synthetic owner atomically.
    fn feedback(&mut self, text: &str) -> Result<(), Rejected> {
        if text.is_empty() {
            return Err(Rejected::EmptyFeedback);
        }
        let index = self.selected.ok_or(Rejected::NoSelection)?;
        if self.notes.len() == MAX_NOTES {
            return Err(Rejected::NoteBound);
        }
        if self.work != Work::Ready {
            return Err(Rejected::HeldWork);
        }
        self.notes.push(Note {
            event: index.saturating_add(1),
            text: text.to_owned(),
        });
        self.work = Work::Held {
            owner: EVENTS[index].owner,
            anchor: index.saturating_add(1),
        };
        Ok(())
    }

    /// Release an existing fixture hold without dispatching any real work.
    fn resume(&mut self) -> Result<(), Rejected> {
        if self.work == Work::Ready {
            return Err(Rejected::NoHeldWork);
        }
        self.work = Work::Ready;
        Ok(())
    }

    /// Serialize the view contract through the published Logical Works JSON facade.
    /// Synthetic labels are present on every snapshot, including empty scenarios.
    pub fn snapshot(&self) -> Result<String, json::Error> {
        let inspection = self.selected.map(|index| &EVENTS[index]);
        let value = json::json!({
            "mode": "DEMO — JSON fixture harness; native AppCUI view: cargo run --bin rocco",
            "effects": "none: no execution, providers, filesystem reads or writes",
            "persistence": "session only; reset and restart discard synthetic notes",
            "scenario": if self.empty { "empty" } else { "change" },
            "events": &EVENTS[..self.cursor], "inspection": inspection,
            "work": self.work, "feedback": self.notes,
            "folder_drawer_requested": self.folders, "agent_rail_requested": self.agents,
            "agents": if self.agents { json::json!([
                {"id":0,"role":"parent/owner","parent":null},
                {"id":1,"role":"clone delegate: code","parent":0},
                {"id":2,"role":"clone delegate: tests","parent":0}
            ]) } else { json::json!(null) },
            "folders": if self.folders { json::json!(["src/retry.rs", "tests/retry.rs"]) }
                else { json::json!(null) },
            "help": "next | inspect N | feedback TEXT | continue | folders | agents | scenario empty/change | reset | status | quit"
        });
        json::to_string(&value)
    }
}
