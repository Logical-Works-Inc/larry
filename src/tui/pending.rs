//! Turns queued while a session runs (larry#262, ADR 0128): the turns Enter
//! queues while a session runs, bounded at eight and shown above the
//! composer. Pure state: no terminal, no clock, no thread; the frame in
//! [`super::app`] owns one.

use std::collections::VecDeque;

use super::style::{Line, Span, Style, ellipsize, sanitize_line};

/// Most turns queued while a session runs; Enter queues the next turn.
pub(crate) const QUEUE_BOUND: usize = 8;

/// The command's first line: the status row never shows past it, so a
/// multi-line approval pastes as one row (larry#262).
pub(crate) fn first_line(command: &str) -> String {
    command.lines().take(1).collect()
}

/// Turns queued while a session runs.
#[derive(Debug, Clone, Default)]
pub(crate) struct Pending {
    /// Queued turns, oldest first, at most [`QUEUE_BOUND`].
    queue: VecDeque<String>,
}

impl Pending {
    /// Empty queue: no turn waiting.
    pub(crate) const fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    /// Whether an eighth turn already waits.
    pub(crate) fn queue_full(&self) -> bool {
        self.queue.len() >= QUEUE_BOUND
    }

    /// Queues `text` for the next turn.
    pub(crate) fn push_turn(&mut self, text: String) {
        if self.queue.len() < QUEUE_BOUND {
            self.queue.push_back(text);
        }
    }

    /// The next queued turn, oldest first.
    pub(crate) fn pop_next(&mut self) -> Option<String> {
        self.queue.pop_front()
    }

    /// Drops every queued turn: quitting starts none.
    pub(crate) fn clear_queue(&mut self) {
        self.queue.clear();
    }

    /// Queue rows for the frame to paint above the composer, one per waiting turn.
    pub(crate) fn queue_lines(&self, width: usize) -> Vec<Line> {
        self.queue
            .iter()
            .enumerate()
            .map(|(index, queued)| {
                let first = first_line(queued);
                vec![Span::new(
                    ellipsize(
                        &format!(
                            "queued {}: {}",
                            index.saturating_add(1),
                            sanitize_line(&first)
                        ),
                        width,
                    ),
                    Style::MUTED,
                )]
            })
            .collect()
    }
}
