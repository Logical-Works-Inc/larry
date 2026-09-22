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
//!   Rocco v0.1.0 · DEMO fixture · no model, no backend
//!   Welcome to Rocco DEMO — …
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
//! Forge and Rocco agree that text carries no background of its own.
//!
//! Exactly one region takes input at a time; the active panel is named in the
//! status hub. Every fixture action stays in memory, and every simulated path
//! stays visibly DEMO.

use super::{Demo, MAX_INPUT, Work};
use crate::theme::{ACCENT, BASE, CHIP, ERR, MI, MUTED, WARN};
use appcui::prelude::*;
use lgwks_deps::appcui;

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

/// Returns a terminal text width without narrowing a platform-sized count.
pub(super) fn text_width(text: &str) -> i32 {
    i32::try_from(text.chars().count()).unwrap_or(i32::MAX)
}

/// Converts a nonnegative terminal coordinate into an allocation length.
pub(super) fn allocation_width(width: i32) -> usize {
    usize::try_from(width.max(0)).unwrap_or(usize::MAX)
}

/// Converts a bounded collection index into a terminal coordinate.
pub(super) fn index_coordinate(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
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
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The tail every accepted command's transcript echo carries. Named here
/// because the width test has to tell an echo (which holds operator text and
/// may clip) apart from authored copy (which must not).
const ECHO_SUFFIX: &str = " — DEMO; no backend action executed";

/// Input prompt and the always-visible keybinding hint (Forge's `brand::HINT`).
const PROMPT: &str = "❯ ";
/// Always-visible shortcut reminder beside the fixture input.
const HINT: &str = "/ cmds · Tab panels · ^C quit";

/// Forge's activity line is `✢ {Verb}… · thinking with {EFFORT}`. Rocco has no
/// model to be thinking with, so the suffix states the fixture truth instead.
const ACTIVITY_SUFFIX: &str = " · DEMO fixture · no model called";

/// Rotating tips. Forge rotates these on a timer; Rocco has no timer and no
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
        .title("Rocco workspace — DEMO DATA")
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
    /// line. Rocco keeps the order and states the fixture truth in the banner.
    fn seed_header(&mut self) {
        self.push_row(
            Row::bold(
                format!(
                    "Rocco v{} · DEMO fixture · no model, no backend",
                    env!("CARGO_PKG_VERSION")
                ),
                MUTED,
            ),
            false,
        );
        // Three short rows rather than one long one: the transcript column
        // narrows to 50 whenever the dashboard collapses, and a sentence
        // sliced at a word boundary reads as a broken build.
        self.push_row(Row::plain("Welcome to Rocco DEMO.", MUTED), false);
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
        let at = panels
            .iter()
            .position(|panel| *panel == self.focus)
            .unwrap_or(0);
        let next = if forward {
            at.saturating_add(1).rem_euclid(panels.len())
        } else {
            at.saturating_add(panels.len())
                .saturating_sub(1)
                .rem_euclid(panels.len())
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

    /// The canary: Forge's one-glance verdict strip, carrying Rocco's real
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
                Row::plain("  Rocco  [SIMULATED] Message received.", MUTED),
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
            let end = self.transcript[after_start..]
                .iter()
                .position(|row| row.starts_message)
                .map(|offset| after_start.saturating_add(offset))
                .unwrap_or(self.transcript.len());
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
        let selected = self.demo.selected.unwrap_or(0);
        let next = if down {
            selected
                .saturating_add(1)
                .min(self.demo.cursor.saturating_sub(1))
        } else {
            selected.saturating_sub(1)
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

// ── painting primitives ──────────────────────────────────────────────────

/// Blocky Rocco — the 24x24 pixel raccoon, hand-authored from the canonical
/// rocco-flat.webp (Director request 2026-09-08: flatter, symmetric Rocco) by
/// tools/rocco_pixel.py, which owns the blink frame and the Codex pet export.
/// Identity features preserved per Director history: paired dark-interior
/// ears, bandit mask, LARGE brown eyes with cream catchlights, dark nose,
/// open smile with tongue. Voice: warm, calm, concrete — never decoration.
const MASCOT: [&str; 24] = [
    "........................",
    "...dd..............dd...",
    "..dddd............dddd..",
    "..dddd............dddd..",
    "..dggd............dggd..",
    "..dggd............dggd..",
    "..dgg..............ggd..",
    "..dgggg..........ggggd..",
    "..dggggggggggggggggggd..",
    ".dggggggggggggggggggggd.",
    ".dggdddddddggdddddddggd.",
    ".dggddbbbbdggdbbbbddggd.",
    ".dggddbcbbdggdbbcbddggd.",
    ".dggddbcbbdggdbbcbddggd.",
    ".dggddbbbbdggdbbbbddggd.",
    ".dggdddddddggdddddddggd.",
    ".dggggggccccccccggggggd.",
    ".dgggggccccccccccgggggd.",
    ".dggggccccddddccccggggd.",
    ".dggggccccddddccccggggd.",
    ".dgggggccccccccccgggggd.",
    ".dgggggcddddddddcgggggd.",
    ".dgggggcdttttttdcgggggd.",
    "...dddddddddddddddddd...",
];

/// Brand palette -> ANSI16 (no TRUE_COLORS in the estate build; ANSI16 also
/// renders identically on every terminal). '.' = transparent (skip cell).
fn mascot_color(ch: char) -> Option<Color> {
    match ch {
        'd' => Some(Color::Black),
        'g' => Some(Color::Gray),
        'c' => Some(Color::White),
        'b' => Some(Color::Olive),
        't' => Some(Color::Red),
        _ => None,
    }
}

/// Draw the half-block mascot (24 cols x 12 rows) with top-left at (x, y).
/// Two grid rows merge into one cell row: '▀' with fg=top bg=bottom; a lone
/// bottom pixel uses '▄'. Transparent cells keep the terminal default.
fn draw_mascot(surface: &mut Surface, x: i32, y: i32) {
    for pair in 0..12usize {
        let top_row = MASCOT[pair.saturating_mul(2)];
        let bot_row = MASCOT[pair.saturating_mul(2).saturating_add(1)];
        for (column, (top_pixel, bottom_pixel)) in top_row.chars().zip(bot_row.chars()).enumerate()
        {
            let (top_foreground, bottom_foreground) =
                (mascot_color(top_pixel), mascot_color(bottom_pixel));
            if top_foreground.is_none() && bottom_foreground.is_none() {
                continue;
            }
            let (ch, foreground, background) = match (top_foreground, bottom_foreground) {
                (Some(foreground), None) => ('▀', foreground, Color::Transparent),
                (None, Some(foreground)) => ('▄', foreground, Color::Transparent),
                (Some(foreground), Some(background)) => ('▀', foreground, background),
                (None, None) => continue,
            };
            surface.write_char(
                x.saturating_add(index_coordinate(column)),
                y.saturating_add(index_coordinate(pair)),
                Character::new(ch, foreground, background, CharFlags::None),
            );
        }
    }
}

/// Writes one clipped styled string when its origin and width are visible.
fn write(
    surface: &mut Surface,
    x: i32,
    y: i32,
    width: i32,
    text: &str,
    color: Color,
    flags: CharFlags,
) {
    if width <= 0 || y < 0 || x < 0 {
        return;
    }
    let clipped: String = text.chars().take(allocation_width(width)).collect();
    surface.write_string(
        x,
        y,
        &clipped,
        CharAttribute::new(color, Color::Transparent, flags),
        false,
    );
}

/// Paint one transcript row, run by run, clipping at `width`.
///
/// A row that overruns its column loses text at the edge, which is normal for
/// a source line and a defect for anything this code wrote. Both cases get an
/// `…` in the last cell — otherwise a clipped diff reads as if it ended there
/// — and the [`Fit::Panel`] case additionally trips a debug assertion, so the
/// mistake surfaces in the build the tests run in rather than in a screenshot.
fn write_row(surface: &mut Surface, x: i32, y: i32, width: i32, row: &Row) {
    let mut cursor = x;
    let mut left = width;
    let mut truncated = false;
    for run in &row.parts {
        if left <= 0 {
            truncated = true;
            break;
        }
        write(surface, cursor, y, left, &run.text, run.color, run.flags);
        let used = text_width(&run.text);
        truncated |= used > left;
        cursor = cursor.saturating_add(used.min(left));
        left = left.saturating_sub(used);
    }
    if truncated && width > 0 {
        debug_assert!(
            row.fit == Fit::Clip,
            "an authored row overran its column: {row:?}"
        );
        write(
            surface,
            x.saturating_add(width).saturating_sub(1),
            y,
            1,
            "…",
            MUTED,
            CharFlags::None,
        );
    }
}

/// Draws a clipped horizontal panel rule.
fn rule(surface: &mut Surface, x: i32, y: i32, width: i32) {
    let line: String = "─".repeat(allocation_width(width));
    write(surface, x, y, width, &line, MUTED, CharFlags::None);
}

/// Square panel frame with an optional title on the top edge, matching
/// Forge's `Borders::ALL`. Content rows go inside at `x + 2`, starting at
/// `y + 1`. Returns the inner width.
fn frame(
    surface: &mut Surface,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    title: &str,
    accent: bool,
) -> i32 {
    if width < 4 || height < 3 || y < 0 {
        return 0;
    }
    let color = if accent { ACCENT } else { MUTED };
    let flags = if accent {
        CharFlags::Bold
    } else {
        CharFlags::None
    };
    let inner = width.saturating_sub(4);
    let mut top = if title.is_empty() {
        format!(
            "┌{}┐",
            "─".repeat(allocation_width(inner.saturating_add(2)))
        )
    } else {
        let label = format!(" {title} ");
        let taken = text_width(&label);
        let span = inner.saturating_add(2);
        let fill = allocation_width(span.saturating_sub(taken.min(span)));
        format!("┌{label}{}┐", "─".repeat(fill))
    };
    top = top.chars().take(allocation_width(width)).collect();
    write(surface, x, y, width, &top, color, flags);
    for row in 1..height.saturating_sub(1) {
        write(
            surface,
            x,
            y.saturating_add(row),
            1,
            "│",
            color,
            CharFlags::None,
        );
        write(
            surface,
            x.saturating_add(width).saturating_sub(1),
            y.saturating_add(row),
            1,
            "│",
            color,
            CharFlags::None,
        );
    }
    let bottom: String = format!(
        "└{}┘",
        "─".repeat(allocation_width(inner.saturating_add(2)))
    )
    .chars()
    .take(allocation_width(width))
    .collect();
    write(
        surface,
        x,
        y.saturating_add(height).saturating_sub(1),
        width,
        &bottom,
        color,
        CharFlags::None,
    );
    inner
}

/// Writes a string centered in the available terminal width.
fn center(surface: &mut Surface, y: i32, width: i32, text: &str, color: Color, flags: CharFlags) {
    let remaining = width.saturating_sub(text_width(text));
    let pad = remaining.div_euclid(2).max(0);
    write(
        surface,
        pad,
        y,
        width.saturating_sub(pad),
        text,
        color,
        flags,
    );
}

/// Forge's three full-width regions: transcript, input line, status hub.
struct Regions {
    /// First transcript row.
    content_top: i32,
    /// Exclusive transcript bottom row.
    content_bottom: i32,
    /// Input row.
    input_y: i32,
    /// Separator immediately above the status hub.
    hub_rule_y: i32,
    /// First status-hub row.
    hub_top_y: i32,
    /// Final status-hub row.
    hub_bottom_y: i32,
}

/// Computes the terminal rows assigned to transcript, input, and status hub.
fn regions(height: i32) -> Regions {
    Regions {
        content_top: 0,
        content_bottom: height.saturating_sub(4),
        input_y: height.saturating_sub(4),
        hub_rule_y: height.saturating_sub(3),
        hub_top_y: height.saturating_sub(2),
        hub_bottom_y: height.saturating_sub(1),
    }
}

/// The transcript, scrolled from the bottom like Forge's `follow_bottom`.
fn draw_transcript(surface: &mut Surface, width: i32, area: (i32, i32), ui: &Native) {
    let (top, bottom) = area;
    let space = allocation_width(bottom.saturating_sub(top));
    if space == 0 {
        return;
    }
    let show_mascot = ui.user_messages == 0
        && ui.scroll == 0
        && !ui.help
        && bottom.saturating_sub(top)
            > index_coordinate(MASCOT.len())
                .div_euclid(2)
                .saturating_add(4);
    let mut y = top;
    if show_mascot {
        draw_mascot(surface, 0, y);
        y = y.saturating_add(index_coordinate(MASCOT.len()).div_euclid(2));
    }
    let rows = ui.rows();
    let space = allocation_width(bottom.saturating_sub(y));
    let total = rows.len();
    let start = total
        .saturating_sub(space)
        .saturating_sub(ui.scroll.min(total));
    for (y, row) in (y..bottom).zip(rows.iter().skip(start)) {
        write_row(surface, 0, y, width, row);
    }
}

/// One fixture panel inside an area, Forge's bordered block with a title.
fn draw_panel(surface: &mut Surface, width: i32, area: (i32, i32), title: &str, rows: &[Row]) {
    let (top, bottom) = area;
    let height = bottom.saturating_sub(top);
    if height < 4 || width < 8 {
        for (y, row) in (top..bottom).zip(rows.iter()) {
            write_row(surface, 0, y, width, row);
        }
        return;
    }
    frame(surface, 0, top, width, height, title, true);
    for (index, row) in rows
        .iter()
        .take(allocation_width(height.saturating_sub(2)))
        .enumerate()
    {
        write_row(
            surface,
            2,
            top.saturating_add(1)
                .saturating_add(index_coordinate(index)),
            width.saturating_sub(4),
            row,
        );
    }
}

/// The canary strip: a solid chip carrying the fixture verdict, then its
/// reason in the same colour. Forge's one-glance strip, Rocco's real state.
fn draw_canary(surface: &mut Surface, x: i32, y: i32, width: i32, ui: &Native) {
    let (word, color, reason) = ui.canary();
    let label = format!(" {} ", word);
    let label_w = text_width(&label);
    for (offset, ch) in label.chars().enumerate() {
        let cell = Character::new(ch, CHIP, color, CharFlags::Bold);
        surface.write_char(x.saturating_add(index_coordinate(offset)), y, cell);
    }
    write(
        surface,
        x.saturating_add(label_w).saturating_add(1),
        y,
        width.saturating_sub(label_w).saturating_sub(1),
        &reason,
        color,
        CharFlags::Bold,
    );
}

/// The dashboard's border title, in Forge's `brand::DASH_TITLE` position on
/// the left rule. Forge names its instrument panel; this names the fixture
/// state the panel actually carries, because "flight controls" over a fixture
/// with no telemetry and no flight would be costume.
const DASH_TITLE: &str = " fixture state";

/// Forge's `·`-divided tab bar; the active tab keeps the `▸` marker so the
/// selection is not carried by colour alone.
fn draw_tabs(surface: &mut Surface, x: i32, y: i32, width: i32, ui: &Native) {
    let active = ui.dashboard_tab();
    let mut cursor = x;
    let tabs = ui.tabs();
    for (index, &(label, active_label, focus)) in tabs.iter().enumerate() {
        let is_active = focus == active;
        let text = if is_active {
            format!("▸ {active_label}")
        } else {
            label.to_owned()
        };
        let used = text_width(&text);
        if cursor.saturating_add(used) >= x.saturating_add(width) {
            break;
        }
        write(
            surface,
            cursor,
            y,
            used,
            &text,
            if is_active { ACCENT } else { MUTED },
            if is_active {
                CharFlags::Bold
            } else {
                CharFlags::None
            },
        );
        cursor = cursor.saturating_add(used);
        if index < tabs.len().saturating_sub(1) {
            write(surface, cursor, y, 3, " · ", MUTED, CharFlags::None);
            cursor = cursor.saturating_add(3);
        }
    }
}

/// The right-hand dashboard behind a left rule, Forge's 34% column.
fn draw_dashboard(surface: &mut Surface, width: i32, area: (i32, i32), ui: &Native) {
    let (top, bottom) = area;
    let dash_w = dashboard_width(width);
    let x = width.saturating_sub(dash_w);
    if dash_w < 24 {
        return;
    }
    // Left rule the height of the content area.
    for y in top..bottom {
        write(surface, x, y, 1, "│", MUTED, CharFlags::None);
    }
    let inner_x = x.saturating_add(2);
    let inner_w = dash_w.saturating_sub(3);
    // The dashboard is only reached at [`DASHBOARD_MIN_WIDTH`], so this is the
    // floor of the column every panel row is written to fit.
    debug_assert!(
        inner_w >= i32::try_from(PANEL_MIN_INNER).unwrap_or(i32::MAX)
            || width < DASHBOARD_MIN_WIDTH,
        "the dashboard threshold must leave room for the narrowest panel"
    );
    if bottom.saturating_sub(top) < 7 || inner_w < 12 {
        return;
    }
    // Forge titles the panel on its left border, above the canary strip
    // (`Block::borders(LEFT).title(brand::DASH_TITLE)`). Forge's title reads
    // "flight controls"; there is no flight here to control and no telemetry
    // to read, so this names what the panel actually is.
    write(surface, x, top, 1, "│", MUTED, CharFlags::None);
    write(
        surface,
        inner_x,
        top,
        inner_w,
        DASH_TITLE,
        MUTED,
        CharFlags::None,
    );
    draw_canary(surface, inner_x, top.saturating_add(1), inner_w, ui);
    draw_tabs(surface, inner_x, top.saturating_add(2), inner_w, ui);
    let body = (top.saturating_add(4), bottom);
    let body_w = inner_w;
    let rows = match ui.dashboard_tab() {
        Focus::Changes => ui.changes_rows(),
        Focus::Files => ui.files_rows(),
        Focus::Agents => ui.agents_rows(),
        Focus::Input => Vec::new(),
    };
    for (y, row) in (body.0..body.1).zip(rows.iter()) {
        write_row(surface, inner_x, y, body_w, row);
    }
}

/// The main area. With the dashboard up, Forge's split holds: the transcript
/// keeps the left and the dashboard carries the focused panel. Below Forge's
/// 100-column threshold the dashboard collapses and the focused panel takes
/// the full width, so every panel stays reachable in a narrow terminal.
fn draw_content(surface: &mut Surface, width: i32, area: (i32, i32), ui: &Native) {
    let (top, bottom) = area;
    let dashboard = width >= DASHBOARD_MIN_WIDTH && bottom.saturating_sub(top) >= 6;
    if dashboard {
        let main_w = width.saturating_sub(dashboard_width(width));
        draw_transcript(surface, main_w, (top, bottom), ui);
        draw_dashboard(surface, width, (top, bottom), ui);
        return;
    }
    match ui.focus {
        Focus::Input => draw_transcript(surface, width, (top, bottom), ui),
        Focus::Changes => draw_panel(
            surface,
            width,
            (top, bottom),
            "Changes [fixture]",
            &ui.changes_rows(),
        ),
        Focus::Files => draw_panel(
            surface,
            width,
            (top, bottom),
            "Files [fixture]",
            &ui.files_rows(),
        ),
        Focus::Agents => draw_panel(
            surface,
            width,
            (top, bottom),
            "Agents [fixture]",
            &ui.agents_rows(),
        ),
    }
}

/// Forge's input line: accent prompt, the input, then the dim hint pinned
/// right. A block cursor marks the insertion point (AppCUI has no caret here).
fn draw_input(surface: &mut Surface, width: i32, y: i32, ui: &Native) {
    let focused = ui.focus == Focus::Input || ui.command_mode;
    let hint_width = text_width(HINT);
    let tail_width = allocation_width(width.saturating_sub(hint_width).saturating_sub(10).max(4));
    let tail: String = ui
        .input
        .chars()
        .rev()
        .take(tail_width)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let placeholder = ui.input.is_empty() && focused;
    let shown = if placeholder {
        "Ask anything — fixture only, nothing runs…".to_owned()
    } else {
        tail
    };
    // The caret belongs to the focused input, as in Forge: an unfocused line
    // showing a text cursor claims an insertion point it does not have.
    let caret = if focused { "▏" } else { "" };
    write(surface, 0, y, 2, PROMPT, ACCENT, CharFlags::Bold);
    write(
        surface,
        2,
        y,
        width.saturating_sub(hint_width).saturating_sub(4),
        &format!("{shown}{caret}"),
        if placeholder { MUTED } else { BASE },
        CharFlags::None,
    );
    let hint_x = width.saturating_sub(hint_width);
    write(
        surface,
        hint_x.max(2),
        y,
        hint_width,
        HINT,
        MUTED,
        CharFlags::None,
    );
}

/// Forge's protected status hub: a top rule, then MI (the assistant side) and
/// MD (the fixture's machine side) split by a vertical divider. Rocco already
/// names these halves MI and MD (D01), so the hub is a direct fit.
fn draw_status(surface: &mut Surface, width: i32, regions: &Regions, ui: &Native) {
    rule(surface, 0, regions.hub_rule_y, width);
    let mi_w = hub_mi_width(width);
    write(
        surface,
        mi_w,
        regions.hub_top_y,
        1,
        "│",
        MUTED,
        CharFlags::None,
    );
    write(
        surface,
        mi_w,
        regions.hub_bottom_y,
        1,
        "│",
        MUTED,
        CharFlags::None,
    );

    // MI: what the assistant side is, and what it is doing.
    write_parts(
        surface,
        0,
        regions.hub_top_y,
        mi_w,
        &[
            chip("MI", MI),
            part(
                "DEMO · no model adapter · session only",
                BASE,
                CharFlags::None,
            ),
        ],
    );
    write_parts(
        surface,
        0,
        regions.hub_bottom_y,
        mi_w,
        &[
            part("✢ ", ACCENT, CharFlags::None),
            part(format!("{}…", ui.activity()), BASE, CharFlags::Bold),
            part(ACTIVITY_SUFFIX, MUTED, CharFlags::None),
        ],
    );

    // MD: the fixture's machine-readable state, then a rotating tip.
    let md_x = mi_w.saturating_add(2);
    let md_w = width.saturating_sub(md_x);
    let held = match ui.demo.work {
        Work::Held { owner, .. } => Some(owner),
        Work::Ready => None,
    };
    let state = match held {
        Some(owner) => format!("clone {owner} held"),
        None => "ready".to_owned(),
    };
    write_parts(
        surface,
        md_x,
        regions.hub_top_y,
        md_w,
        &[
            chip("MD", WARN),
            part(
                format!(
                    "events {}/{} · notes {} · {state}",
                    ui.demo.cursor,
                    super::EVENTS.len(),
                    ui.demo.notes.len()
                ),
                BASE,
                CharFlags::None,
            ),
        ],
    );
    write(
        surface,
        md_x,
        regions.hub_bottom_y,
        md_w,
        ui.tip(),
        MUTED,
        CharFlags::None,
    );
}

/// Paint styled runs at (x, y), clipped at `width`; returns the columns used.
fn write_parts(surface: &mut Surface, x: i32, y: i32, width: i32, parts: &[Part]) -> i32 {
    let mut cursor = x;
    let mut left = width;
    for run in parts {
        if left <= 0 {
            break;
        }
        write(surface, cursor, y, left, &run.text, run.color, run.flags);
        let used = text_width(&run.text);
        cursor = cursor.saturating_add(used);
        left = left.saturating_sub(used);
    }
    cursor.saturating_sub(x)
}

/// Forge's command palette: 46 columns, sitting just above the input line,
/// accent border, `❯` on the selection.
fn draw_palette(surface: &mut Surface, width: i32, input_y: i32, ui: &Native) {
    if !ui.command_mode || ui.input.is_empty() {
        return;
    }
    let commands = [
        ("/changes", "", "switch to the Changes panel"),
        ("/inspect", "", "toggle the selected diff"),
        ("/next", "", "reveal the next fixture change"),
        ("/continue", "", "release the held clone"),
        ("/folders", "", "toggle the Files panel"),
        ("/agents", "", "toggle the Agents panel"),
        ("/reset", "", "reset all fixture state"),
        ("/scenario empty", "", "clear the change list"),
        ("/help", "", "toggle the command guide"),
    ];
    let filter = ui.input.to_lowercase();
    let matches: Vec<(&str, &str, &str)> = commands
        .iter()
        .filter(|command| command.0.starts_with(&filter))
        .copied()
        .collect();
    if matches.is_empty() {
        return;
    }
    let palette_w = 46.min(width.saturating_sub(4)).max(20);
    let palette_h = index_coordinate(matches.len()).saturating_add(2);
    let y = input_y.saturating_sub(palette_h);
    if y < 0 {
        return;
    }
    frame(surface, 2, y, palette_w, palette_h, "Commands", true);
    for (index, &(name, keys, description)) in matches.iter().enumerate() {
        let selected = index == 0;
        let marker = if selected { "❯ " } else { "  " };
        write_parts(
            surface,
            4,
            y.saturating_add(1).saturating_add(index_coordinate(index)),
            palette_w.saturating_sub(4),
            &[
                part(marker, ACCENT, CharFlags::Bold),
                part(format!("{name:<16}"), ACCENT, CharFlags::Bold),
                part(format!("{keys:<6}"), MUTED, CharFlags::None),
                part(description, BASE, CharFlags::None),
            ],
        );
    }
}

impl InputApp for Native {
    fn on_key_event(&mut self, key: Key, ch: char) -> EventProcessStatus {
        // Ctrl chords keep their Mac terminal meaning: readline editing in
        // the input, interrupt/EOF to quit. Fixture actions live on single
        // letters outside the input (below), where nothing can be typed over.
        // No platform branch: this behaves the same on every terminal.
        if key.modifier == KeyModifier::Ctrl {
            match key.code {
                KeyCode::C | KeyCode::D => App::close(),
                KeyCode::U if self.focus == Focus::Input => self.input.clear(),
                KeyCode::W if self.focus == Focus::Input => {
                    while self.input.ends_with(' ') {
                        self.input.pop();
                    }
                    while !self.input.is_empty() && !self.input.ends_with(' ') {
                        self.input.pop();
                    }
                }
                KeyCode::H if self.focus == Focus::Input => {
                    self.input.pop();
                }
                _ => return EventProcessStatus::Ignored,
            }
            return EventProcessStatus::Processed;
        }
        match key.code {
            KeyCode::F1 => self.help = !self.help,
            KeyCode::F2 => self.apply("folders"),
            KeyCode::F3 => self.apply("agents"),
            KeyCode::F4 => self.apply("next"),
            KeyCode::F5 => self.apply("continue"),
            KeyCode::F6 => self.inspection = !self.inspection,
            KeyCode::F7 => self.apply("scenario empty"),
            KeyCode::F8 => self.apply("reset"),
            KeyCode::Tab => {
                self.command_mode = false;
                self.cycle_focus(true);
            }
            KeyCode::Left => {
                self.command_mode = false;
                self.cycle_focus(false);
            }
            KeyCode::Right => {
                self.command_mode = false;
                self.cycle_focus(true);
            }
            KeyCode::Up => self.select_event(false),
            KeyCode::Down => self.select_event(true),
            KeyCode::Home => {
                if self.demo.cursor > 0 {
                    self.apply("inspect 1");
                }
            }
            KeyCode::End => {
                if self.demo.cursor > 0 {
                    self.apply(&format!("inspect {}", self.demo.cursor));
                }
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(5);
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(5);
            }
            KeyCode::Backspace => {
                if self.focus == Focus::Input || self.command_mode {
                    self.input.pop();
                    if self.input.is_empty() {
                        self.command_mode = false;
                    }
                }
            }
            KeyCode::Enter => {
                if self.command_mode {
                    self.command_mode = false;
                    self.submit();
                } else {
                    match self.focus {
                        Focus::Input => self.submit(),
                        Focus::Changes => self.inspection = !self.inspection,
                        Focus::Files | Focus::Agents => self.focus = Focus::Input,
                    }
                }
            }
            KeyCode::Escape => {
                if self.command_mode {
                    self.command_mode = false;
                    self.input.clear();
                } else {
                    match self.focus {
                        Focus::Input | Focus::Changes => App::close(),
                        Focus::Files | Focus::Agents => self.focus = Focus::Input,
                    }
                }
            }
            KeyCode::Space => {
                if self.command_mode || self.focus == Focus::Input {
                    if self.input.len() < MAX_INPUT {
                        self.input.push(' ');
                    }
                } else if self.focus == Focus::Changes {
                    self.inspection = !self.inspection;
                } else {
                    self.focus = Focus::Input;
                }
            }
            _ if ch != '\0' && !ch.is_control() => {
                if self.focus == Focus::Input {
                    if self
                        .input
                        .len()
                        .checked_add(ch.len_utf8())
                        .is_some_and(|length| length <= MAX_INPUT)
                    {
                        self.input.push(ch);
                    } else {
                        self.push_notice("input limit: 4096 bytes", ERR);
                    }
                } else {
                    match ch {
                        '?' => self.help = !self.help,
                        '/' => {
                            self.command_mode = true;
                            self.input.push('/');
                        }
                        'j' if !self.command_mode => self.select_event(true),
                        'k' if !self.command_mode => self.select_event(false),
                        'n' if !self.command_mode => self.apply("next"),
                        'c' if !self.command_mode => self.apply("continue"),
                        'i' if !self.command_mode => self.inspection = !self.inspection,
                        'f' if !self.command_mode => self.apply("folders"),
                        't' if !self.command_mode => self.apply("agents"),
                        'q' if !self.command_mode => App::close(),
                        ch if self.command_mode
                            && self
                                .input
                                .len()
                                .checked_add(ch.len_utf8())
                                .is_some_and(|length| length <= MAX_INPUT) =>
                        {
                            self.input.push(ch);
                        }
                        _ => {}
                    }
                }
            }
            _ => return EventProcessStatus::Ignored,
        }
        EventProcessStatus::Processed
    }

    fn on_paint(&self, surface: &mut Surface) {
        let width = terminal_dimension(surface.size().width);
        let height = terminal_dimension(surface.size().height);
        // Default terminal background and foreground everywhere: reset to
        // transparent cells, then paint glyphs only.
        surface.reset(Character::new(
            ' ',
            Color::Transparent,
            Color::Transparent,
            CharFlags::None,
        ));
        if height < MIN_HEIGHT || width < MIN_WIDTH {
            center(surface, 2, width, "✳ Rocco DEMO", BASE, CharFlags::Bold);
            center(
                surface,
                4,
                width,
                "DEMO: resize to 50x12 or larger. Esc exits.",
                WARN,
                CharFlags::None,
            );
            return;
        }
        let layout = regions(height);
        draw_content(
            surface,
            width,
            (layout.content_top, layout.content_bottom),
            self,
        );
        draw_input(surface, width, layout.input_y, self);
        draw_status(surface, width, &layout, self);
        draw_palette(surface, width, layout.input_y, self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::TOKENS;

    /// Read every expected cell; missing cells or unrepresentable sizes fail the probe.
    fn painted(ui: &Native, width: u32, height: u32) -> std::io::Result<String> {
        let rows = i32::try_from(height).map_err(std::io::Error::other)?;
        let columns = i32::try_from(width).map_err(std::io::Error::other)?;
        let mut surface = Surface::new(width, height);
        ui.on_paint(&mut surface);
        let mut text = String::new();
        for y in 0..rows {
            for x in 0..columns {
                text.push(
                    surface
                        .char(x, y)
                        .ok_or_else(|| {
                            std::io::Error::other(format!("missing rendered cell ({x}, {y})"))
                        })?
                        .code,
                );
            }
            text.push('\n');
        }
        Ok(text)
    }

    /// Read a rendered color without hiding an invalid coordinate behind a default.
    fn background_at(
        ui: &Native,
        width: u32,
        height: u32,
        x: i32,
        y: i32,
    ) -> std::io::Result<Color> {
        let mut surface = Surface::new(width, height);
        ui.on_paint(&mut surface);
        surface
            .char(x, y)
            .map(|cell| cell.background)
            .ok_or_else(|| std::io::Error::other(format!("missing rendered cell ({x}, {y})")))
    }

    #[test]
    fn surface_probes_reject_missing_cells_and_unrepresentable_sizes() {
        let ui = Native::default();
        assert_eq!(
            background_at(&ui, 2, 2, 2, 0).map_err(|error| error.to_string()),
            Err("missing rendered cell (2, 0)".to_owned())
        );
        assert_eq!(
            painted(&ui, u32::MAX, 1).map_err(|error| error.kind()),
            Err(std::io::ErrorKind::Other)
        );
    }

    fn press(ui: &mut Native, code: KeyCode) {
        ui.on_key_event(Key::new(code, KeyModifier::None), '\0');
    }

    fn type_text(ui: &mut Native, text: &str) {
        for ch in text.chars() {
            // Punctuation arrives with a neutral code; the handler routes on `ch`.
            let code = match ch {
                'j' => KeyCode::J,
                'k' => KeyCode::K,
                'q' => KeyCode::Q,
                _ => KeyCode::None,
            };
            ui.on_key_event(Key::new(code, KeyModifier::None), ch);
        }
    }

    /// The transcript text only, for assertions about what was said.
    fn transcript(ui: &Native) -> String {
        ui.transcript
            .iter()
            .flat_map(|row| row.row.parts.iter())
            .map(|run| run.text.clone())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn forge_chrome_carries_header_input_hub_and_dashboard() -> std::io::Result<()> {
        let ui = Native::default();
        let shown = painted(&ui, 120, 32)?;
        // Banner, welcome and the DEMO truth.
        assert!(shown.contains("Rocco v"));
        assert!(shown.contains("no model, no backend"));
        assert!(shown.contains("Safe fixture"));
        // Input line with the accent prompt and the right-hand hint.
        assert!(shown.contains("❯"));
        assert!(shown.contains("Tab panels"));
        // Status hub: rule, MI/MD chips, divider, activity line.
        assert!(shown.contains("MI"));
        assert!(shown.contains("MD"));
        assert!(shown.contains("│"));
        assert!(shown.contains("✢ Ready…"));
        assert!(shown.contains("no model called"));
        // Dashboard at Forge's threshold: canary, tabs, panel body.
        assert!(shown.contains("CLEAR"));
        assert!(shown.contains("▸ Changes"));
        assert!(shown.contains("No changes yet"));
        assert!(shown.contains("Tip:"));
        Ok(())
    }

    /// The end state of the fixture journey: two changes behind us, both
    /// drawers revealed, the dashboard back on Changes and the transcript
    /// carrying every action that produced this picture.
    #[test]
    fn surface_after_two_changes_with_the_drawers_open() -> std::io::Result<()> {
        let mut ui = Native::default();
        ui.apply("folders");
        ui.apply("agents");
        ui.focus = Focus::Changes;
        ui.apply("next");
        ui.apply("next");
        let shown = painted(&ui, 120, 32)?;
        assert!(shown.contains("1  src/retry.rs:12"));
        assert!(shown.contains("2  tests/retry.rs:8"));
        assert!(shown.contains("events 2/3 · notes 0 · ready"));
        assert!(
            shown.contains("▸ Changes · Files · Agents"),
            "revealed drawers stay on the tab bar"
        );
        assert!(painted(&ui, 100, 32)?.contains("src/retry.rs:12"));
        let mut held = Native::default();
        held.apply("next");
        held.inspection = true;
        held.apply("feedback keep the retry bounded");
        held.help = true;
        assert!(painted(&held, 100, 32)?.contains("HELD"));
        Ok(())
    }

    /// The dashboard column at a default 120-column terminal — the width a
    /// reader most likely has. Clip rows are held here.
    const DEFAULT_PANEL_INNER: usize = 37;

    /// The transcript column once the dashboard has collapsed and the
    /// transcript has the terminal to itself, i.e. at [`MIN_WIDTH`] — the
    /// narrowest width that still paints this layout. Authored transcript
    /// copy is held here.
    const TRANSCRIPT_MIN_INNER: usize = 50;

    fn row_width(row: &Row) -> usize {
        row.parts.iter().map(|run| run.text.chars().count()).sum()
    }

    /// The dashboard is the one place rows cannot simply be truncated: a
    /// clipped `+1 -1` reads as a different diff. This walks every fixture
    /// state and every panel and fails if any row overruns the floor, so
    /// [`PANEL_MIN_INNER`] is measured rather than asserted by comment.
    #[test]
    fn every_authored_row_fits_the_narrowest_column() {
        let floor = allocation_width(dashboard_width(DASHBOARD_MIN_WIDTH).saturating_sub(3));
        assert_eq!(
            floor, PANEL_MIN_INNER,
            "the declared floor must equal the real share column"
        );
        assert_eq!(
            allocation_width(dashboard_width(120).saturating_sub(3)),
            DEFAULT_PANEL_INNER,
            "the default-terminal column must match a 120-column layout"
        );

        let mut states = vec![("empty", Native::default())];
        for (advanced, label) in [(1usize, "one"), (2, "two"), (3, "three")] {
            let mut ui = Native::default();
            ui.apply("folders");
            ui.apply("agents");
            for _ in 0..advanced {
                ui.apply("next");
            }
            states.push((label, ui));
        }
        let mut held = Native::default();
        held.apply("next");
        held.apply("feedback keep it bounded");
        states.push(("held", held));
        let mut inspected = Native::default();
        inspected.apply("next");
        inspected.inspection = true;
        states.push(("inspected", inspected));
        states.push((
            "help",
            Native {
                help: true,
                ..Native::default()
            },
        ));

        // The transcript is not covered by the loop below: it holds operator
        // input too, and that is allowed to clip. Its authored copy is not.
        for &(state, ref ui) in &states {
            for row in ui.transcript.iter().map(|logged| &logged.row) {
                let text: String = row.parts.iter().map(|run| run.text.as_str()).collect();
                if text.ends_with(ECHO_SUFFIX) {
                    continue; // a command echo; the operator's text, not ours
                }
                assert!(
                    row_width(row) <= TRANSCRIPT_MIN_INNER,
                    "transcript row overruns the narrow column in {state}: {text:?} is {} wide",
                    row_width(row)
                );
            }
        }

        for &(state, ref ui) in &states {
            let (word, _, reason) = ui.canary();
            let canary = word.chars().count() + 2 + 1 + reason.chars().count();
            assert!(
                canary <= PANEL_MIN_INNER,
                "canary overruns the panel in {state}: {canary} > {PANEL_MIN_INNER}"
            );
            let mut panels = vec![ui.changes_rows(), ui.files_rows(), ui.agents_rows()];
            if ui.inspection {
                panels.push(ui.inspection_rows());
            }
            for rows in panels {
                for row in &rows {
                    let limit = match row.fit {
                        Fit::Panel => PANEL_MIN_INNER,
                        // Clip rows get the default terminal's column instead,
                        // so a diff that grew past the floor is still caught —
                        // just against the width a reader actually has.
                        Fit::Clip => DEFAULT_PANEL_INNER,
                    };
                    assert!(
                        row_width(row) <= limit,
                        "row overruns its panel in {state}: {row:?} is {} wide, limit {limit}",
                        row_width(row)
                    );
                }
            }
        }
    }

    #[test]
    fn dashboard_collapses_below_the_forge_threshold() -> std::io::Result<()> {
        let ui = Native::default();
        let narrow = painted(&ui, 80, 24)?;
        assert!(!narrow.contains("CLEAR"));
        assert!(narrow.contains("✢ Ready…"));
        assert_eq!(narrow.lines().count(), 24);
        Ok(())
    }

    #[test]
    fn canary_reports_the_real_fixture_verdict() -> std::io::Result<()> {
        let mut ui = Native::default();
        assert_eq!(ui.canary().0, "CLEAR");
        assert!(painted(&ui, 120, 32)?.contains("CLEAR"));

        press(&mut ui, KeyCode::F4);
        ui.apply("feedback keep retry bounded");
        assert_eq!(ui.canary().0, "HELD");
        let held = painted(&ui, 120, 32)?;
        assert!(held.contains("HELD"));
        assert!(held.contains("clone 1 held at event 1"));
        assert!(held.contains("✢ Held…"));

        press(&mut ui, KeyCode::F5);
        assert_eq!(ui.canary().0, "CLEAR");

        press(&mut ui, KeyCode::F4);
        press(&mut ui, KeyCode::F4);
        assert_eq!(ui.canary().0, "END");
        assert!(painted(&ui, 120, 32)?.contains("scenario complete"));
        Ok(())
    }

    #[test]
    fn notices_and_diffs_are_narrated_into_the_transcript() -> std::io::Result<()> {
        let mut ui = Native::default();
        press(&mut ui, KeyCode::Tab);
        assert_eq!(ui.focus, Focus::Changes);
        type_text(&mut ui, "n");
        assert_eq!(ui.demo.cursor, 1);
        assert!(transcript(&ui).contains("next — DEMO; no backend action executed"));
        type_text(&mut ui, "i");
        assert!(ui.inspection);
        let shown = painted(&ui, 120, 32)?;
        assert!(shown.contains("fixture-01"));
        assert!(shown.contains("retry_with_budget(request, 3)"));
        assert!(shown.contains("- retry(request)"));
        type_text(&mut ui, "/");
        assert!(ui.command_mode);
        type_text(&mut ui, "feedback pacing note");
        press(&mut ui, KeyCode::Enter);
        assert!(!ui.command_mode);
        assert!(transcript(&ui).contains("HELD clone 1"));
        let shown = painted(&ui, 120, 32)?;
        assert!(shown.contains("anchored note: pacing note"));
        Ok(())
    }

    #[test]
    fn rejected_commands_are_logged_without_changing_fixture_state() -> std::io::Result<()> {
        let mut ui = Native::default();
        ui.apply("next");
        ui.apply("feedback keep bounded");
        // The clone that is not held may still advance.
        ui.apply("next");
        let before = ui.demo.clone();
        // The next change belongs to the held clone, so it is refused and the
        // entire prior state survives.
        ui.apply("next");
        assert_eq!(ui.demo, before);
        assert_eq!(ui.demo.cursor, 2);
        assert!(transcript(&ui).contains("HeldWork"));
        assert!(painted(&ui, 120, 32)?.contains("✖"));
        Ok(())
    }

    #[test]
    fn help_prints_forge_style_command_rows() -> std::io::Result<()> {
        let mut ui = Native::default();
        press(&mut ui, KeyCode::Tab);
        type_text(&mut ui, "?");
        assert!(ui.help);
        let shown = painted(&ui, 120, 32)?;
        assert!(shown.contains("Commands — DEMO fixture, no backend"));
        assert!(shown.contains("/feedback"));
        assert!(shown.contains("Letters work outside the input; no F-keys needed."));
        type_text(&mut ui, "?");
        assert!(!ui.help);
        Ok(())
    }

    #[test]
    fn palette_sits_above_the_input_with_the_selection_marker() -> std::io::Result<()> {
        let mut ui = Native::default();
        press(&mut ui, KeyCode::Tab);
        type_text(&mut ui, "/");
        assert!(ui.command_mode);
        type_text(&mut ui, "f");
        let shown = painted(&ui, 120, 32)?;
        assert!(shown.contains("Commands"));
        assert!(shown.contains("❯ /folders"));
        Ok(())
    }

    #[test]
    fn input_keeps_mac_editing_meaning_for_ctrl_chords() {
        let mut ui = Native::default();
        let ctrl = |code| Key::new(code, KeyModifier::Ctrl);
        ui.input = "hello brave new world".into();
        ui.on_key_event(ctrl(KeyCode::W), '\0');
        assert_eq!(ui.input, "hello brave new ");
        ui.on_key_event(ctrl(KeyCode::U), '\0');
        assert!(ui.input.is_empty());
        // Ctrl chords never act as fixture actions: cursor does not advance.
        ui.on_key_event(ctrl(KeyCode::N), '\0');
        assert_eq!(ui.demo.cursor, 0);
    }

    #[test]
    fn paint_keeps_terminal_default_background() -> std::io::Result<()> {
        let ui = Native::default();
        assert_eq!(background_at(&ui, 120, 32, 5, 0)?, Color::Transparent);
        assert_eq!(background_at(&ui, 120, 32, 5, 30)?, Color::Transparent);
        Ok(())
    }

    #[test]
    fn mascot_perches_on_the_empty_landing_and_leaves_after_first_message() -> std::io::Result<()> {
        let mut ui = Native::default();
        let shown = painted(&ui, 120, 32)?;
        assert!(shown.contains('▀'));
        assert!(shown.contains("Safe fixture"));
        ui.input = "hello".into();
        press(&mut ui, KeyCode::Enter);
        assert_eq!(ui.user_messages, 1);
        let shown = painted(&ui, 120, 32)?;
        assert!(!shown.contains('▀'));
        assert!(!shown.contains('▄'));
        assert!(shown.contains("[SIMULATED]"));
        // Tiny terminals fall back to text only — the mascot never clips.
        let tiny = painted(&ui, 60, 12)?;
        assert!(!tiny.contains('▀'));
        Ok(())
    }

    #[test]
    fn mascot_brand_pixels_carry_the_canonical_palette() -> std::io::Result<()> {
        let mut surface = Surface::new(24, 12);
        // on_paint resets every cell to Transparent first; replicate that so
        // the probe proves the mascot merges into the default background.
        surface.reset(Character::new(
            ' ',
            Color::Transparent,
            Color::Transparent,
            CharFlags::None,
        ));
        draw_mascot(&mut surface, 0, 0);
        let at = |x: i32, y: i32| {
            surface
                .char(x, y)
                .ok_or_else(|| std::io::Error::other(format!("missing mascot cell ({x}, {y})")))
        };
        // Eye catchlight (row 12, col 7) is cream = White on gray fur.
        assert_eq!(at(7, 6)?.foreground, Color::White);
        // Eye body (row 12, col 6) is brown = Olive (as the cell's top half).
        assert_eq!(at(6, 6)?.foreground, Color::Olive);
        // Nose (rows 18-19, cols 10-13) is dark = Black.
        assert_eq!(at(10, 9)?.foreground, Color::Black);
        // Tongue (row 22, cols 9-14) is Red; mouth outline around it is Black.
        assert_eq!(at(9, 11)?.foreground, Color::Red);
        assert_eq!(at(8, 11)?.foreground, Color::Black);
        // Every painted glyph keeps the terminal default where transparent:
        // cells outside the 24x12 mascot box stay untouched.
        assert_eq!(at(23, 11)?.background, Color::Transparent);
        Ok(())
    }

    #[test]
    fn native_keys_drive_same_hold_continue_and_panes() -> std::io::Result<()> {
        let mut ui = Native::default();
        press(&mut ui, KeyCode::F2);
        press(&mut ui, KeyCode::F3);
        // Revealing the drawers focuses them, so return to the input the way
        // a user would before typing a command.
        press(&mut ui, KeyCode::Tab);
        assert_eq!(ui.focus, Focus::Input);
        press(&mut ui, KeyCode::F4);
        ui.input = "/feedback keep retry bounded".into();
        press(&mut ui, KeyCode::Enter);
        assert!(transcript(&ui).contains("HELD clone 1"));
        press(&mut ui, KeyCode::F4);
        press(&mut ui, KeyCode::F4);
        // Second F4 is rejected: owner 1 is held (HELD clone 1).
        assert_eq!(ui.demo.cursor, 2);
        assert!(ui.demo.folders && ui.demo.agents);
        assert_eq!(
            ui.demo.work,
            Work::Held {
                owner: 1,
                anchor: 1
            }
        );
        press(&mut ui, KeyCode::F5);
        assert!(transcript(&ui).contains("no backend action executed"));
        press(&mut ui, KeyCode::F4);
        press(&mut ui, KeyCode::F6);
        assert_eq!(ui.demo.cursor, 3);
        // Back to the Changes panel to see the selected diff's trace.
        press(&mut ui, KeyCode::Tab);
        assert_eq!(ui.focus, Focus::Changes);
        let shown = painted(&ui, 120, 32)?;
        assert!(shown.contains("fixture-03"));
        Ok(())
    }

    #[test]
    fn focus_cycle_gives_visible_panel_focus_with_vim_movement() -> std::io::Result<()> {
        let mut ui = Native::default();
        assert_eq!(ui.focus, Focus::Input);
        press(&mut ui, KeyCode::Tab);
        assert_eq!(ui.focus, Focus::Changes);
        type_text(&mut ui, "j");
        assert_eq!(ui.demo.selected, None);
        press(&mut ui, KeyCode::F4);
        type_text(&mut ui, "j");
        assert_eq!(ui.demo.selected, Some(0));
        type_text(&mut ui, "/");
        assert!(ui.command_mode);
        assert_eq!(ui.focus, Focus::Changes);
        press(&mut ui, KeyCode::Escape);
        assert!(!ui.command_mode);
        assert_eq!(ui.focus, Focus::Changes);
        // Revealing a drawer also focuses it, so asking for Files does not
        // leave the reader hunting for the panel they just opened.
        press(&mut ui, KeyCode::F2);
        assert_eq!(ui.focus, Focus::Files);
        press(&mut ui, KeyCode::F3);
        assert_eq!(ui.focus, Focus::Agents);
        press(&mut ui, KeyCode::Tab);
        assert_eq!(ui.focus, Focus::Input);
        // The active tab carries the fixture marker; the others stay short so
        // the bar still fits Forge's narrow column.
        let shown = painted(&ui, 120, 32)?;
        assert!(shown.contains("▸ Changes"));
        assert!(shown.contains("Files"));
        assert!(shown.contains("Agents"));
        ui.focus = Focus::Agents;
        let active = painted(&ui, 120, 32)?;
        assert!(active.contains("▸ Agents [fixture]"));
        Ok(())
    }

    #[test]
    fn native_text_is_bounded_and_reset_empty_render() -> std::io::Result<()> {
        let mut ui = Native {
            input: "x".repeat(MAX_INPUT),
            ..Native::default()
        };
        ui.on_key_event(Key::new(KeyCode::A, KeyModifier::None), 'é');
        assert_eq!(ui.input.len(), MAX_INPUT);
        press(&mut ui, KeyCode::Backspace);
        assert_eq!(ui.input.len(), MAX_INPUT - 1);
        press(&mut ui, KeyCode::Enter);
        assert_eq!(ui.user_messages, 1);
        press(&mut ui, KeyCode::F7);
        assert!(transcript(&ui).contains("Safe fixture"));
        assert_eq!(ui.demo.cursor, 0);
        press(&mut ui, KeyCode::F8);
        assert_eq!(ui.user_messages, 0);
        press(&mut ui, KeyCode::F4);
        assert_eq!(ui.demo.cursor, 1);
        let shown = painted(&ui, 120, 32)?;
        assert!(shown.contains("src/retry.rs:12"));
        Ok(())
    }

    #[test]
    fn long_sessions_evict_whole_messages_and_bound_the_log() {
        let mut ui = Native::default();
        for index in 0..20 {
            ui.input = format!("message {index}");
            press(&mut ui, KeyCode::Enter);
        }
        assert_eq!(ui.user_messages, 16);
        let text = transcript(&ui);
        assert!(!text.contains("message 0"));
        assert!(text.contains("message 19"));
        assert!(ui.transcript.len() <= MAX_LOG);
    }

    #[test]
    fn small_terminal_sizes_are_bounded_without_panicking() -> std::io::Result<()> {
        let ui = Native::default();
        for (width, height) in [(1, 1), (49, 11), (50, 12), (80, 24), (120, 32), (200, 50)] {
            let text = painted(&ui, width, height)?;
            assert_eq!(
                text.lines().count(),
                usize::try_from(height).map_err(std::io::Error::other)?
            );
        }
        Ok(())
    }

    #[test]
    fn tokens_stay_distinguishable() {
        // The mapping is the falsifier for colour depth: if two Forge tokens
        // collapsed onto one ANSI16 colour, the UI could no longer tell a
        // caution from a failure.
        for (index, &(name, _, color)) in TOKENS.iter().enumerate() {
            for &(other_name, _, other) in TOKENS.iter().skip(index.saturating_add(1)) {
                assert_ne!(color, other, "tokens {name} and {other_name} collide");
            }
        }
    }
}
