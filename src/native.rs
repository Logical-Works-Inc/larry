//! Native AppCUI presentation of the effect-free fixture state.
//!
//! The look is the Forge terminal language from
//! `forge-harness/forge-tui/src/{render,theme,brand}.rs` (read 2026-09-18,
//! re-implemented here — no Forge code is imported, copied or ported, per
//! the Director correction of 2026-09-16). What is reproduced is the
//! *structure*, which is what makes the surface recognisable:
//!
//! ```text
//!   [ raccoon mark ]                transcript, full width, no chrome
//!   Larry v0.1.0 · DEMO fixture · no model, no backend
//!   Welcome to Larry DEMO — …
//!     ✓ notice rows          ✖ failures
//!     ▸ changes              - / + diffs
//!   ❯ input                                  / cmds · Tab panels · ^C quit
//!   ───────────────────────────────────────────────────────────────────────
//!    MI DEMO · no model adapter │ MD events 3/3 · notes 1 · ready
//!   ✢ Ready… · DEMO fixture …   │ Tip: …rotating on each action
//! ```
//!
//! Three full-width regions, exactly as Forge splits them: transcript
//! (`Min 3`), input line (1 row), status hub (a top rule plus two rows). The
//! optional right-hand dashboard appears at Forge's 100-column threshold and
//! takes Forge's 34% share behind a left rule.
//!
//! Deviations are deliberate and listed in [`crate::theme`]: body text stays
//! on the terminal default so light and dark terminals share one branch-free
//! path, and the ANSI16 stand-ins keep the accents distinguishable. Both
//! Forge and Larry agree that text carries no background of its own.
//!
//! Exactly one region takes input at a time; the active panel is named in the
//! status hub. Every fixture action stays in memory, and every simulated path
//! stays visibly DEMO.

use super::{Demo, MAX_INPUT, Work};
use crate::theme::{ACCENT, BASE, CHIP, ERR, MI, MUTED, WARN};
use appcui::prelude::*;
use lgwks_deps::appcui;

mod input;
mod render;
mod workspace;

/// Below this the surface degrades to a single centred notice.
const MIN_WIDTH: i32 = 50;
/// Minimum rows required for the fixture's truthful compact layout.
const MIN_HEIGHT: i32 = 12;

/// Forge shows its dashboard at 100 columns and gives it a 34% share.
const DASHBOARD_MIN_WIDTH: i32 = 100;
/// Integer numerator of the dashboard's 34% share.
const DASHBOARD_SHARE_NUMERATOR: i32 = 34;
/// Common denominator for layout share calculations.
const LAYOUT_SHARE_DENOMINATOR: i32 = 100;

/// The narrowest the dashboard's content column ever gets: the share column at
/// [`DASHBOARD_MIN_WIDTH`], less the one-column divider and its two-column
/// gutter. Every dashboard row has to fit inside this, and
/// `every_authored_row_fits_the_narrowest_column` is the falsifier rather than this
/// comment. It grows with the terminal, so this is the worst case.
const PANEL_MIN_INNER: usize = 31;

/// Integer numerator of the status hub's MI/MD 44% split.
const HUB_MI_SHARE_NUMERATOR: i32 = 44;

/// `value` as `T`, or `ceiling` when it does not fit. Terminal geometry
/// saturates at a bound the caller names instead of wrapping; this is the one
/// conversion every width, height and index on the surface goes through.
pub(super) fn saturating<T: TryFrom<U>, U>(value: U, ceiling: T) -> T {
    match T::try_from(value) {
        Ok(fits) => fits,
        Err(_) => ceiling,
    }
}

/// Returns a terminal text width without narrowing a platform-sized count.
pub(super) fn text_width(text: &str) -> i32 {
    saturating(text.chars().count(), i32::MAX)
}

/// Converts a nonnegative terminal coordinate into an allocation length.
pub(super) fn allocation_width(width: i32) -> usize {
    saturating(width.max(0), usize::MAX)
}

/// Converts a bounded collection index into a terminal coordinate.
pub(super) fn index_coordinate(index: usize) -> i32 {
    saturating(index, i32::MAX)
}

/// Calculates Forge's dashboard share with integer, saturating arithmetic.
fn dashboard_width(width: i32) -> i32 {
    width
        .max(0)
        .saturating_mul(DASHBOARD_SHARE_NUMERATOR)
        .div_euclid(LAYOUT_SHARE_DENOMINATOR)
}

/// Calculates the MI half of the status hub with the same bounded arithmetic.
fn hub_mi_width(width: i32) -> i32 {
    width
        .max(0)
        .saturating_mul(HUB_MI_SHARE_NUMERATOR)
        .div_euclid(LAYOUT_SHARE_DENOMINATOR)
}

/// Converts AppCUI's unsigned surface dimension at its explicit terminal bound.
pub(super) fn terminal_dimension(value: u32) -> i32 {
    saturating(value, i32::MAX)
}

/// The tail every accepted command's transcript echo carries. Named here
/// because the width test has to tell an echo (which holds operator text and
/// may clip) apart from authored copy (which must not).
const ECHO_SUFFIX: &str = " — DEMO; no backend action executed";

/// Input prompt and the always-visible keybinding hint (Forge's `brand::HINT`).
const PROMPT: &str = "❯ ";
/// Always-visible shortcut reminder beside the fixture input.
const HINT: &str = "/ cmds · Tab panels · ^C quit";

/// Forge's activity line is `✢ {Verb}… · thinking with {EFFORT}`. Larry has no
/// model to be thinking with, so the suffix states the fixture truth instead.
const ACTIVITY_SUFFIX: &str = " · DEMO fixture · no model called";

/// Rotating tips. Forge rotates these on a timer; Larry has no timer and no
/// background work, so they advance once per fixture action instead — a real
/// interaction, never an idle wakeup.
const TIPS: [&str; 5] = [
    "Tip: / opens the command palette · Tab switches panels",
    "Tip: n next · c continue · i inspect · ? help",
    "Tip: /feedback TEXT holds the affected clone",
    "Tip: F5 Continue remembers the held anchor",
    "Tip: /scenario empty clears the list · /quit exits",
];

/// Upper bound on retained narration rows. The transcript is a bounded
/// snapshot like every other fixture structure, never an unbounded log.
const MAX_LOG: usize = 512;

/// Run the native demo. All fixture actions remain in memory.
pub fn run() -> Result<(), appcui::system::Error> {
    App::input_app(workspace::Workspace::default())
        // AppCUI defaults to Termios on macOS, which cannot receive mouse
        // events. The already-admitted CrossTerm backend enables the rail's
        // explicit pointer affordance without changing workspace semantics.
        .backend(appcui::backend::Type::CrossTerm)
        .title("Larry workspace — DEMO DATA")
        .auto_close(false)
        .run()
}

/// A named panel that can take input. `Input` is the transcript itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The transcript and note input own keyboard input.
    Input,
    /// The fixture change panel is selected.
    Changes,
    /// The fixture file panel is selected.
    Files,
    /// The fixture agent panel is selected.
    Agents,
}

/// One styled run of text inside a transcript row.
#[derive(Debug, Clone)]
struct Part {
    /// Text rendered by this run.
    text: String,
    /// ANSI foreground colour for this run.
    color: Color,
    /// Text emphasis flags for this run.
    flags: CharFlags,
}

/// Constructs one styled transcript run.
fn part(text: impl Into<String>, color: Color, flags: CharFlags) -> Part {
    Part {
        text: text.into(),
        color,
        flags,
    }
}

/// What a row promises about its own width.
///
/// A dashboard row is one of two kinds and they cannot share an invariant.
/// The panel's own copy — labels, counts, state words, the fixture's
/// provenance — is written for the panel, so it must fit
/// [`PANEL_MIN_INNER`] or it is a bug. Rows that carry text this code did not
/// author — a source line from a diff, or a note the operator typed — are the
/// other kind: clipping them at the panel edge is the honest rendering,
/// because wrapping a code line would invent a line break the diff never had.
/// Declaring which is which here lets `every_authored_row_fits_the_narrowest_column`
/// hold each to its own rule instead of the weaker rule both would survive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fit {
    /// Must fit the narrowest panel.
    Panel,
    /// May be clipped at the panel edge.
    Clip,
}

/// One transcript row: a sequence of styled runs.
#[derive(Debug, Clone)]
struct Row {
    /// Ordered styled runs making up the row.
    parts: Vec<Part>,
    /// Whether the row must fit or may visibly clip.
    fit: Fit,
}

impl Row {
    /// Creates an ordinary, unboldened authored row.
    fn plain(text: impl Into<String>, color: Color) -> Self {
        Self {
            parts: vec![part(text, color, CharFlags::None)],
            fit: Fit::Panel,
        }
    }

    /// Creates a bold authored row.
    fn bold(text: impl Into<String>, color: Color) -> Self {
        Self {
            parts: vec![part(text, color, CharFlags::Bold)],
            fit: Fit::Panel,
        }
    }

    /// Creates an authored row from supplied styled runs.
    fn of(parts: Vec<Part>) -> Self {
        Self {
            parts,
            fit: Fit::Panel,
        }
    }

    /// A row holding text this code did not author; see [`Fit`].
    fn clipping(parts: Vec<Part>) -> Self {
        Self {
            parts,
            fit: Fit::Clip,
        }
    }
}

/// A transcript row plus whether it opens a user-message group. Grouping is
/// what makes the 16-message cap evict whole messages instead of half of one.
#[derive(Debug, Clone)]
struct LogRow {
    /// Rendered row contents.
    row: Row,
    /// Whether this row begins a retained user-message group.
    starts_message: bool,
}

/// Disposable state for one native fixture session.
struct Native {
    /// Synthetic work fixture shown by the terminal surface.
    demo: Demo,
    /// Current unsent input buffer.
    input: String,
    /// Bounded fixture transcript.
    transcript: Vec<LogRow>,
    /// Count of user-message groups retained; drives eviction and the mark.
    user_messages: usize,
    /// Whether a selected fixture diff is expanded.
    inspection: bool,
    /// Whether the fixture command guide is visible.
    help: bool,
    /// Whether slash-command completion owns input.
    command_mode: bool,
    /// Bottom-relative transcript scroll offset.
    scroll: usize,
    /// Panel currently owning keyboard navigation.
    focus: Focus,
    /// Advanced once per fixture action: rotates the tip without a timer.
    tick: usize,
}

impl Default for Native {
    fn default() -> Self {
        let mut native = Self {
            demo: Demo::default(),
            input: String::new(),
            transcript: Vec::new(),
            user_messages: 0,
            inspection: false,
            help: false,
            command_mode: false,
            scroll: 0,
            focus: Focus::Input,
            tick: 0,
        };
        native.seed_header();
        native
    }
}

/// The status hub's left chip (` MI ` / ` MD `), Forge's `tag_span`.
fn chip(label: &str, color: Color) -> Part {
    part(format!(" {label} "), color, CharFlags::Bold)
}

impl Native {
    /// Forge seeds the transcript with the mark, a dim banner and a welcome
    /// line. Larry keeps the order and states the fixture truth in the banner.
    fn seed_header(&mut self) {
        self.push_row(
            Row::bold(
                format!(
                    "Larry v{} · DEMO fixture · no model, no backend",
                    env!("CARGO_PKG_VERSION")
                ),
                MUTED,
            ),
            false,
        );
        // Three short rows rather than one long one: the transcript column
        // narrows to 50 whenever the dashboard collapses, and a sentence
        // sliced at a word boundary reads as a broken build.
        self.push_row(Row::plain("Welcome to Larry DEMO.", MUTED), false);
        self.push_row(Row::plain("Type below, or / for commands.", MUTED), false);
        self.push_row(
            Row::plain("Safe fixture: nothing runs or changes on disk.", MUTED),
            false,
        );
    }

    /// Appends a row while evicting the oldest bounded transcript entry.
    fn push_row(&mut self, row: Row, starts_message: bool) {
        self.transcript.push(LogRow {
            row,
            starts_message,
        });
        while self.transcript.len() > MAX_LOG {
            drop(self.transcript.drain(..1));
        }
    }

    /// A `✓`/`●`/`✖` outcome row, mirroring Forge's run-outcome lines. The
    /// glyph and the word both carry the meaning, so colour is never the only
    /// signal.
    fn push_notice(&mut self, text: &str, color: Color) {
        let (mark, mark_color) = if color == ERR {
            ("✖", ERR)
        } else if color == WARN {
            ("●", WARN)
        } else {
            ("✓", ACCENT)
        };
        self.push_row(
            Row::of(vec![
                part(format!("  {mark} "), mark_color, CharFlags::Bold),
                part(text.to_owned(), color, CharFlags::None),
            ]),
            false,
        );
    }

    /// Panels that can currently take input, in reading order.
    fn panels(&self) -> Vec<Focus> {
        let mut panels = vec![Focus::Input, Focus::Changes];
        if self.demo.folders {
            panels.push(Focus::Files);
        }
        if self.demo.agents {
            panels.push(Focus::Agents);
        }
        panels
    }

    /// Returns focus to the transcript when a conditional panel disappeared.
    fn normalize_focus(&mut self) {
        if !self.panels().contains(&self.focus) {
            self.focus = Focus::Input;
        }
    }

    /// Moves focus through the currently available panels.
    fn cycle_focus(&mut self, forward: bool) {
        let panels = self.panels();
        let len = panels.len();
        let next = match (
            panels.iter().position(|panel| *panel == self.focus),
            forward,
        ) {
            (Some(at), true) => at.saturating_add(1).rem_euclid(len),
            (Some(at), false) => at.saturating_add(len).saturating_sub(1).rem_euclid(len),
            // A focus that left the panel list (a drawer just closed) moves as
            // if it stood on the transcript, which is always the first panel.
            (None, true) => 1_usize.rem_euclid(len),
            (None, false) => len.saturating_sub(1),
        };
        self.focus = panels[next];
    }

    /// The dashboard tab that is active, given the focused panel. `Input`
    /// keeps the transcript as the main view and shows Changes beside it.
    fn dashboard_tab(&self) -> Focus {
        match self.focus {
            Focus::Input | Focus::Changes => Focus::Changes,
            other => other,
        }
    }

    /// Available dashboard tabs, Forge's `Changes · Files · Agents`. Each
    /// carries a short label and the full one used while it is active, so the
    /// fixture marker rides on the tab the reader is actually looking at and
    /// the bar still fits Forge's 34% column.
    fn tabs(&self) -> Vec<(&'static str, &'static str, Focus)> {
        let mut tabs = vec![("Changes", "Changes", Focus::Changes)];
        if self.demo.folders {
            tabs.push(("Files", "Files [fixture]", Focus::Files));
        }
        if self.demo.agents {
            tabs.push(("Agents", "Agents [fixture]", Focus::Agents));
        }
        tabs
    }

    /// The canary: Forge's one-glance verdict strip, carrying Larry's real
    /// fixture state instead of sensor heat.
    fn canary(&self) -> (&'static str, Color, String) {
        match self.demo.work {
            Work::Held { owner, anchor } => (
                "HELD",
                WARN,
                format!("clone {owner} held at event {anchor}"),
            ),
            Work::Ready if self.demo.cursor == 0 => {
                ("CLEAR", ACCENT, "F4 reveals the change".to_owned())
            }
            Work::Ready if self.demo.cursor >= super::EVENTS.len() => {
                ("END", MUTED, "scenario complete".to_owned())
            }
            Work::Ready => (
                "CLEAR",
                ACCENT,
                format!(
                    "{}/{} changes revealed",
                    self.demo.cursor,
                    super::EVENTS.len()
                ),
            ),
        }
    }

    /// Applies one fixture command and records its typed result in the transcript.
    fn apply(&mut self, command: &str) {
        self.tick = self.tick.wrapping_add(1);
        match self.demo.command(command) {
            Ok(false) => App::close(),
            Ok(true) => {
                if matches!(command, "reset" | "scenario empty" | "scenario change") {
                    self.transcript.clear();
                    self.user_messages = 0;
                    self.scroll = 0;
                    self.seed_header();
                }
                // Revealing a drawer also looks at it: asking for Files and
                // then having to hunt for it would be its own small tax.
                match command {
                    "folders" if self.demo.folders => self.focus = Focus::Files,
                    "agents" if self.demo.agents => self.focus = Focus::Agents,
                    _ => {}
                }
                self.normalize_focus();
                let (text, color) = match self.demo.work {
                    Work::Ready => (format!("{command}{ECHO_SUFFIX}"), BASE),
                    Work::Held { owner, anchor } => (
                        format!("HELD clone {owner} at event {anchor} · DEMO hold."),
                        WARN,
                    ),
                };
                self.push_notice(&text, color);
            }
            Err(error) => {
                let text = error.to_string();
                self.push_notice(&text, ERR);
            }
        }
    }

    /// Submits the input buffer as a fixture command or simulated message.
    fn submit(&mut self) {
        let input = std::mem::take(&mut self.input);
        self.tick = self.tick.wrapping_add(1);
        if let Some(command) = input.strip_prefix('/') {
            match command {
                "help" => self.help = !self.help,
                "inspect" => self.inspection = !self.inspection,
                "changes" => self.focus = Focus::Changes,
                _ => self.apply(command),
            }
        } else if !input.trim().is_empty() {
            self.push_row(
                Row::of(vec![
                    part(PROMPT, ACCENT, CharFlags::Bold),
                    part(input.clone(), BASE, CharFlags::None),
                ]),
                true,
            );
            self.push_row(
                Row::plain("  Larry  [SIMULATED] Message received.", MUTED),
                false,
            );
            self.push_row(Row::plain("  F4 reveals the next change.", MUTED), false);
            self.user_messages = self.user_messages.saturating_add(1);
            self.evict_oldest_message();
        }
        self.scroll = 0;
    }

    /// Drop the oldest user-message group once 16 are retained. Eviction is
    /// whole messages: half a message would misrepresent the session.
    fn evict_oldest_message(&mut self) {
        if self.user_messages <= 16 {
            return;
        }
        let start = self.transcript.iter().position(|row| row.starts_message);
        if let Some(start) = start {
            let after_start = start.saturating_add(1);
            // The oldest message runs to the next one, or to the end when it is
            // the only message left.
            let end = match self.transcript[after_start..]
                .iter()
                .position(|row| row.starts_message)
            {
                Some(offset) => after_start.saturating_add(offset),
                None => self.transcript.len(),
            };
            self.transcript.drain(start..end);
            self.user_messages = self.user_messages.saturating_sub(1);
        }
    }

    /// Selects the adjacent revealed fixture event, if one exists.
    fn select_event(&mut self, down: bool) {
        if self.demo.cursor == 0 {
            self.push_notice("no events yet — F4 reveals the first change", WARN);
            return;
        }
        let last = self.demo.cursor.saturating_sub(1);
        let next = match (self.demo.selected, down) {
            (Some(selected), true) => selected.saturating_add(1).min(last),
            (Some(selected), false) => selected.saturating_sub(1),
            // Nothing selected yet: move as if the first revealed event were.
            (None, true) => 1_usize.min(last),
            (None, false) => 0,
        };
        self.apply(&format!("inspect {}", next.saturating_add(1)));
    }

    // ── panel content ────────────────────────────────────────────────────

    /// Builds the change-panel rows for the revealed fixture events.
    fn changes_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.demo.cursor == 0 {
            rows.push(Row::plain("No changes yet", BASE));
            rows.push(Row::plain("n — or F4 — adds the first.", MUTED));
            rows.push(Row::plain("Fake fixture: nothing runs.", MUTED));
            rows.push(Row::plain("Nothing is written to disk.", MUTED));
            return rows;
        }
        for (index, event) in super::EVENTS[..self.demo.cursor].iter().enumerate() {
            let selected = self.demo.selected == Some(index);
            // Two lines per change, not one: `src/retry.rs:12` and
            // `tests/retry.rs:8` differ in length, and a single line wide
            // enough for the longest fixture path would still clip on any
            // real path. Splitting keeps the anchor and its provenance
            // readable at [`PANEL_MIN_INNER`] with room to spare.
            rows.push(Row::of(vec![
                part(" ▸ ", ACCENT, CharFlags::None),
                part(
                    format!("{}  ", index.saturating_add(1)),
                    MUTED,
                    CharFlags::None,
                ),
                part(
                    format!("{}:{}", event.file, event.line),
                    if selected { ACCENT } else { BASE },
                    if selected {
                        CharFlags::Bold
                    } else {
                        CharFlags::None
                    },
                ),
            ]));
            rows.push(Row::of(vec![part(
                format!("      clone {} · +1 -1", event.owner),
                MUTED,
                CharFlags::None,
            )]));
            if selected && self.inspection {
                rows.extend(self.inspection_rows());
            }
        }
        rows
    }

    /// The selected change's exact anchor and diff, Forge's `-`/`+` colouring.
    fn inspection_rows(&self) -> Vec<Row> {
        let Some(index) = self.demo.selected else {
            return vec![Row::plain("Select an event first (n, then j/k).", MUTED)];
        };
        let event = &super::EVENTS[index];
        let mut rows = vec![
            Row::bold(format!("{}:{}", event.file, event.line), ACCENT),
            Row::plain(
                format!("  trace {} · clone {}", event.trace, event.owner),
                MUTED,
            ),
            Row::clipping(vec![part(
                format!("  - {}", event.before),
                ERR,
                CharFlags::None,
            )]),
            Row::clipping(vec![part(
                format!("  + {}", event.after),
                ACCENT,
                CharFlags::None,
            )]),
            Row::plain("  /feedback TEXT · fixture", MUTED),
        ];
        for note in self
            .demo
            .notes
            .iter()
            .filter(|note| note.event == index.saturating_add(1))
        {
            rows.push(Row::clipping(vec![part(
                format!("  anchored note: {}", note.text),
                WARN,
                CharFlags::None,
            )]));
        }
        rows
    }

    /// Builds the fixture-only workspace tree shown in the Files panel.
    fn files_rows(&self) -> Vec<Row> {
        vec![
            Row::plain("src/", MUTED),
            Row::plain("  retry.rs", BASE),
            Row::plain("tests/", MUTED),
            Row::plain("  retry.rs", BASE),
            Row::plain("", MUTED),
            Row::plain("Fixture hierarchy.", MUTED),
            Row::plain("No workspace is read.", MUTED),
        ]
    }

    /// Builds the fixture ownership rows shown in the Agents panel.
    fn agents_rows(&self) -> Vec<Row> {
        let held = match self.demo.work {
            Work::Held { owner, .. } => Some(owner),
            Work::Ready => None,
        };
        let line = |name: &str, id: usize, branch: &str| {
            let state = if held == Some(id) { "HELD" } else { "ready" };
            Row::of(vec![
                part(format!(" {branch} "), MUTED, CharFlags::None),
                part(format!("{name:<8}"), BASE, CharFlags::None),
                part(
                    state,
                    if held == Some(id) { WARN } else { MUTED },
                    if held == Some(id) {
                        CharFlags::Bold
                    } else {
                        CharFlags::None
                    },
                ),
            ])
        };
        vec![
            Row::plain("parent / owner", MUTED),
            line("code", 1, "├"),
            line("tests", 2, "└"),
            Row::plain("", MUTED),
            Row::plain("Clone delegates are not peers.", MUTED),
        ]
    }

    /// Forge prints `/help` as command rows in the transcript (`  {name:<12}`
    /// accent, `{args:<24}` dim, description base) rather than opening an
    /// overlay, so the guide scrolls with the session it belongs to.
    fn help_rows(&self) -> Vec<Row> {
        let entries = [
            ("/next", "n, F4", "reveal the next change"),
            ("/continue", "c, F5", "release the held clone"),
            ("/inspect N", "i, F6", "select an event and show its diff"),
            ("/feedback TEXT", "", "store a note and hold the owner"),
            ("/folders", "f, F2", "toggle the Files panel"),
            ("/agents", "t, F3", "toggle the Agents panel"),
            ("/scenario empty", "F7", "clear the change list"),
            ("/reset", "F8", "reset all fixture state"),
            ("/quit", "q, ^C", "leave the DEMO"),
        ];
        // Two rows per entry, not one three-column row: the transcript column
        // drops to 50 when the dashboard collapses, and a description sliced
        // mid-word is worse than a taller table.
        let mut rows = vec![
            Row::bold("Commands — DEMO fixture, no backend", BASE),
            Row::plain("Letters work outside the input; no F-keys needed.", MUTED),
        ];
        for (name, keys, description) in entries {
            let head = if keys.is_empty() {
                format!("  {name}")
            } else {
                format!("  {name:<14}  {keys}")
            };
            rows.push(Row::of(vec![part(head, ACCENT, CharFlags::Bold)]));
            rows.push(Row::of(vec![part(
                format!("      {description}"),
                BASE,
                CharFlags::None,
            )]));
        }
        rows.push(Row::plain("Notes and messages vanish on reset.", MUTED));
        rows.push(Row::plain("Nothing is applied.", MUTED));
        rows
    }

    /// Returns the current deterministic status-hub tip.
    fn tip(&self) -> &'static str {
        TIPS[self.tick.rem_euclid(TIPS.len())]
    }

    /// The activity line's honest verb: the fixture has two states and no
    /// model, so it names the state rather than narrating imaginary work.
    fn activity(&self) -> &'static str {
        match self.demo.work {
            Work::Held { .. } => "Held",
            Work::Ready => "Ready",
        }
    }

    /// Everything to paint, in order: transcript, then the help guide when it
    /// is toggled on.
    fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self.transcript.iter().map(|row| row.row.clone()).collect();
        if self.help {
            rows.push(Row::plain("", MUTED));
            rows.extend(self.help_rows());
        }
        rows
    }
}

#[cfg(test)]
mod tests;
