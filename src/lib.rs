//! Effect-free fixture state for the pending native AppCUI presentation.
//! Every command changes only this owned session; no backend is connected.

use lgwks_std::json::{self, Serialize};

pub mod native;
pub mod theme;

/// Maximum command bytes, excluding the line terminator.
pub const MAX_INPUT: usize = 4096;
const MAX_NOTES: usize = 16;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(crate = "lgwks_std::json::serde")]
struct Event {
    file: &'static str,
    line: usize,
    trace: &'static str,
    owner: usize,
    before: &'static str,
    after: &'static str,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(crate = "lgwks_std::json::serde")]
struct Note {
    event: usize,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(crate = "lgwks_std::json::serde")]
enum Work {
    Ready,
    Held { owner: usize, anchor: usize },
}

/// A disposable synthetic session. Construction performs no I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Demo {
    cursor: usize,
    selected: Option<usize>,
    empty: bool,
    folders: bool,
    agents: bool,
    work: Work,
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
pub enum Rejected {
    InputBound,
    ControlCharacter,
    UnknownCommand,
    InvalidAnchor,
    NoSelection,
    NoHeldWork,
    HeldWork,
    EndOfFixture,
    NoteBound,
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

    fn targeted_command(&mut self, input: &str) -> Result<(), Rejected> {
        if let Some(anchor) = input.strip_prefix("inspect ") {
            let index = anchor
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .filter(|n| *n < self.cursor)
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

    fn advance(&mut self) -> Result<(), Rejected> {
        if self.empty {
            return Err(Rejected::EndOfFixture);
        }
        let event = EVENTS.get(self.cursor).ok_or(Rejected::EndOfFixture)?;
        if matches!(self.work, Work::Held { owner, .. } if owner == event.owner) {
            return Err(Rejected::HeldWork);
        }
        self.selected = Some(self.cursor);
        self.cursor += 1;
        Ok(())
    }

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
            event: index + 1,
            text: text.to_owned(),
        });
        self.work = Work::Held {
            owner: EVENTS[index].owner,
            anchor: index + 1,
        };
        Ok(())
    }

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
        let inspection = self.selected.map(|i| &EVENTS[i]);
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
