//! Queued turns and background commands (larry#262, ADR 0128): the turns Enter
//! queues while a session runs, bounded at eight and shown above the composer,
//! and the commands the model started, bounded at sixteen with their last five
//! output lines for `/ps`. `/stop` ends the running ones through the session's
//! cancel path, the same one Esc uses, so their process groups die. Pure state:
//! no terminal, no clock, no thread; the frame in [`super::app`] owns one.

use std::collections::VecDeque;

use super::cells::OUTPUT_TAIL;
use super::style::{Line, Span, Style, ellipsize, sanitize_line};

/// Most turns queued while a session runs; Enter queues the next turn.
pub(crate) const QUEUE_BOUND: usize = 8;
/// Most background commands `/ps` lists; the oldest drops first.
pub(crate) const JOBS_BOUND: usize = 16;

/// One background command's state: a running command has no verdict, a
/// finished one passed or failed. The type says so, so `passed: false` on a
/// running job is unrepresentable (larry#262).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobState {
    /// Its process group may still run; no verdict yet.
    Running,
    /// It exited 0.
    Passed,
    /// It ended any other way.
    Failed,
}

/// One background command the model started, tracked for `/ps` and `/stop`.
#[derive(Debug, Clone)]
struct Job {
    /// The command's first line, as the person approved it.
    command: String,
    /// Its last output lines, oldest first, at most [`OUTPUT_TAIL`].
    output: Vec<String>,
    /// Whether it still runs or how it ended.
    state: JobState,
}

/// The command's first line: `/ps` and the status row never show past it, so a
/// multi-line approval pastes as one row (larry#262).
pub(crate) fn first_line(command: &str) -> String {
    command.lines().take(1).collect()
}

/// Turns queued while a session runs and the background commands it started.
#[derive(Debug, Clone, Default)]
pub(crate) struct Pending {
    /// Queued turns, oldest first, at most [`QUEUE_BOUND`].
    queue: VecDeque<String>,
    /// Background commands, oldest first, at most [`JOBS_BOUND`].
    jobs: VecDeque<Job>,
}

impl Pending {
    /// Empty queues: no turn waiting, no command tracked.
    pub(crate) const fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            jobs: VecDeque::new(),
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

    /// Background commands still running.
    pub(crate) fn running_jobs(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| job.state == JobState::Running)
            .count()
    }

    /// Tracks `command` as a background command, dropping the oldest past [`JOBS_BOUND`].
    pub(crate) fn push_job(&mut self, command: &str) {
        self.push_bounded(Job {
            command: first_line(command),
            output: Vec::new(),
            state: JobState::Running,
        });
    }

    /// Drops the oldest past [`JOBS_BOUND`], then tracks `job`.
    fn push_bounded(&mut self, job: Job) {
        if self.jobs.len() >= JOBS_BOUND {
            self.jobs.pop_front();
        }
        self.jobs.push_back(job);
    }

    /// Records the end of the running background command: its last output lines and
    /// whether it exited 0. A finish with no running command still lists, so a pane
    /// that never saw its start keeps what the model was shown.
    pub(crate) fn finish_job(&mut self, command: &str, output: &[String], passed: bool) {
        let tail: Vec<String> = output
            .iter()
            .rev()
            .take(OUTPUT_TAIL)
            .rev()
            .cloned()
            .collect();
        let first = first_line(command);
        match self
            .jobs
            .iter_mut()
            .rev()
            .find(|job| job.state == JobState::Running)
        {
            Some(job) => {
                job.command.clone_from(&first);
                job.output = tail;
                job.state = if passed {
                    JobState::Passed
                } else {
                    JobState::Failed
                };
            }
            None => {
                self.push_bounded(Job {
                    command: first,
                    output: tail,
                    state: if passed {
                        JobState::Passed
                    } else {
                        JobState::Failed
                    },
                });
            }
        }
    }

    /// The session ended: no tracked group still runs.
    pub(crate) fn mark_all_done(&mut self) {
        for job in self
            .jobs
            .iter_mut()
            .filter(|job| job.state == JobState::Running)
        {
            job.state = JobState::Failed;
        }
    }

    /// `/ps` rows: the count, each command with its state, and its last lines.
    pub(crate) fn ps_rows(&self) -> Vec<String> {
        if self.jobs.is_empty() {
            return vec![String::from("No background commands.")];
        }
        let count = self.jobs.len();
        let mut rows = vec![format!(
            "{} background command{}:",
            count,
            if count == 1 { "" } else { "s" }
        )];
        for (index, job) in self.jobs.iter().enumerate() {
            let state = match job.state {
                JobState::Running => "running",
                JobState::Passed => "done",
                JobState::Failed => "ended",
            };
            rows.push(format!(
                "{}: {state} $ {}",
                index.saturating_add(1),
                sanitize_line(&job.command)
            ));
            for line in job.output.iter().take(OUTPUT_TAIL) {
                rows.push(format!("    {}", sanitize_line(line)));
            }
            if job.output.is_empty() {
                rows.push(String::from("    (no output yet)"));
            }
        }
        rows
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
