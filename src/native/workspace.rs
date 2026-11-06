//! Board-first presentation. Fixture content is separate from real dispatch.

use std::cell::Cell;

use larry_contracts::ids::{CommandId, ContentDigest, RunId, TenantId, WorkspaceId};
use larry_contracts::state::RunState;
use larry_contracts::workspace::{
    ContinueCoordinator, ContinueOutcome, ContinueRequest, MissingMooAdapter, WorkBoard, WorkItem,
    WorkVersion, WorkspaceScope,
};

use super::render::draw_mascot;
use super::{
    App, BASE, CharAttribute, CharFlags, Character, Color, ERR, EventProcessStatus, Focus,
    InputApp, Key, KeyCode, KeyModifier, MI, MUTED, MouseEvent, Native, Row, Surface, WARN, Work,
    allocation_width, index_coordinate, part, terminal_dimension,
};

/// One fixed synthetic work item and its independently owned unsent notes.
struct Card {
    /// Fixture board lane, not observed live progress.
    lane: &'static str,
    /// Synthetic project label.
    project: &'static str,
    /// Human-readable fixture title.
    title: &'static str,
    /// Short fixture state description.
    summary: &'static str,
    /// Suggested inspection action.
    next: &'static str,
    /// Stable synthetic run identity.
    run: RunId,
    /// Retained idempotency identity for repeated continuation requests.
    command: CommandId,
    /// Disposable note editor and fixture transcript.
    session: Native,
    /// Last actual continuation outcome, separate from fixture lane status.
    dispatch: String,
}

/// One disposable view. No timer, thread, filesystem write or network client.
pub(super) struct Workspace {
    /// The four authored fixture cards; no live discovery or background updates.
    cards: Vec<Card>,
    /// Selected card index.
    selected: usize,
    /// Whether the selected card's session is open.
    entered: bool,
    /// Whether the explicit synthetic-data explanation is open.
    truth: bool,
    /// Bounded row offset into the explanation.
    truth_scroll: usize,
    /// Validates continuation against the fixture board and disconnected owner.
    coordinator: ContinueCoordinator,
    /// Last painted terminal dimensions for pointer hit testing.
    size: Cell<(i32, i32)>,
    /// Fixture construction failure displayed instead of a misleading board.
    setup_error: Option<String>,
    /// An input-driven focus marker. It never represents background progress
    /// or a live connection, and deliberately has no timer.
    motion: u8,
}

impl Default for Workspace {
    fn default() -> Self {
        // Synthetic identities are NEVER credentials; the missing adapter
        // refuses execution. Live identity must come from the authenticated host.
        let scope = WorkspaceScope::new(
            TenantId::from_bytes([1; 16]),
            WorkspaceId::from_bytes([2; 16]),
        );
        let fixtures = [
            (
                "Needs you",
                "LARRY",
                "Review retry boundary",
                "A decision is waiting.",
                "Inspect the proposed diff",
                RunState::AwaitingAuthority,
            ),
            (
                "In progress",
                "MOO",
                "Workspace handoff",
                "Owner checks in progress.",
                "Open its thread",
                RunState::Running,
            ),
            (
                "Up next",
                "LARRY",
                "Calmer project home",
                "Ready for a first pass.",
                "Continue the design",
                RunState::Draft,
            ),
            (
                "Recently done",
                "LARRY",
                "Bounded input",
                "Fixture checks passed.",
                "Inspect the result",
                RunState::Completed,
            ),
        ];
        let mut board = WorkBoard::new(scope);
        let mut setup_error = None;
        let cards = fixtures
            .into_iter()
            .zip([(3_u8, 20_u8), (4, 21), (5, 22), (6, 23)])
            .map(
                |((lane, project, title, summary, next, state), (run_byte, command_byte))| {
                    let run = RunId::from_bytes([run_byte; 16]);
                    let version = WorkVersion::new(1, ContentDigest::over(&[title.as_bytes()]), 1);
                    if let Err(error) = WorkItem::new(scope, run, title, version, state)
                        .and_then(|item| board.upsert(item))
                    {
                        setup_error = Some(error.to_string());
                    }
                    let mut session = Native::default();
                    session.transcript.clear();
                    session.demo.folders = true;
                    session.demo.agents = true;
                    session.push_row(Row::plain("AI · not connected", MI), false);
                    session.push_row(Row::plain("Your notes stay here, unsent.", MUTED), false);
                    session.push_row(Row::plain("Session only · lost on exit.", MUTED), false);
                    Card {
                        lane,
                        project,
                        title,
                        summary,
                        next,
                        run,
                        command: CommandId::from_bytes([command_byte; 16]),
                        session,
                        dispatch: "Not requested".into(),
                    }
                },
            )
            .collect();
        Self {
            cards,
            selected: 0,
            entered: false,
            truth: false,
            truth_scroll: 0,
            coordinator: ContinueCoordinator::new(board),
            size: Cell::new((120, 32)),
            setup_error,
            motion: 0,
        }
    }
}

// AppCUI arrives through the estate storefront in ANSI16 mode. "Quiet torque"
// translates Porsche's monochrome hierarchy and Atlassian's elevation roles
// into terminal facts: black is the shared default surface, gray is structure,
// blue names the one focused work item, and warning/error colours retain their
// operational meaning. There are no decorative region fills.
/// Shared neutral terminal background.
const SHELL: Color = Color::Black;
/// Navigation rail background.
const RAIL: Color = SHELL;
/// Unselected card background.
const PANEL: Color = SHELL;
/// The single selected card and rail highlight.
const FOCUS_PANEL: Color = Color::DarkBlue;
/// Disconnected-system panel background.
const SYSTEM_PANEL: Color = SHELL;
/// Primary foreground text.
const INK: Color = Color::White;
/// Secondary text and neutral borders.
const SOFT: Color = Color::Gray;
/// Focus accents, not execution status.
const FOCUS: Color = Color::Blue;
/// Total navigation rail width in terminal columns.
const RAIL_WIDTH: i32 = 27;
/// First clickable rail column.
const RAIL_ITEM_X: i32 = 1;
/// Clickable rail item width.
const RAIL_ITEM_WIDTH: i32 = 24;
/// First work item's rail row.
const RAIL_ITEM_Y: i32 = 11;
/// Number of rows occupied by each rail target.
const RAIL_ITEM_HEIGHT: i32 = 2;

/// Map a card index to its rail row, saturating outside coordinate range.
fn rail_item_y(index: usize) -> i32 {
    let index = index_coordinate(index);
    RAIL_ITEM_Y.saturating_add(index.saturating_mul(RAIL_ITEM_HEIGHT))
}

/// Paint a rectangle clipped to the owned surface, including extreme coordinates.
fn fill(surface: &mut Surface, x: i32, y: i32, width: i32, height: i32, background: Color) {
    if width <= 0 || height <= 0 {
        return;
    }
    let right = x
        .saturating_add(width)
        .min(terminal_dimension(surface.size().width));
    let bottom = y
        .saturating_add(height)
        .min(terminal_dimension(surface.size().height));
    for row in y.max(0)..bottom.max(0) {
        for column in x.max(0)..right.max(0) {
            surface.write_char(
                column,
                row,
                Character::new(
                    ' ',
                    background.contrast_color(),
                    background,
                    CharFlags::None,
                ),
            );
        }
    }
}

/// Small rendering DSL rather than an eight-argument utility: the grammar is
/// `surface, position, text, foreground, background, emphasis` at every call
/// site, which makes colour ownership visible in the scene code.
macro_rules! ink {
    ($surface:expr, $x:expr, $y:expr, $width:expr, $text:expr, $foreground:expr, $background:expr, $bold:expr $(,)?) => {{
        let (x, y, width): (i32, i32, i32) = ($x, $y, $width);
        let text = $text;
        let foreground = $foreground;
        let background = $background;
        if width > 0 && y >= 0 && x >= 0 {
            let columns = allocation_width(width);
            let clipped: String = text.chars().take(columns).collect();
            $surface.write_string(
                x,
                y,
                &clipped,
                CharAttribute::new(
                    foreground,
                    background,
                    if $bold {
                        CharFlags::Bold
                    } else {
                        CharFlags::None
                    },
                ),
                false,
            );
            if text.chars().count() > columns {
                $surface.write_char(
                    x.saturating_add(width).saturating_sub(1),
                    y,
                    Character::new('…', SOFT, background, CharFlags::None),
                );
            }
        }
    }};
}

/// Draw a horizontal separator in the requested terminal colors.
fn line(surface: &mut Surface, x: i32, y: i32, width: i32, color: Color, background: Color) {
    ink!(
        surface,
        x,
        y,
        width,
        &"─".repeat(allocation_width(width)),
        color,
        background,
        false,
    );
}

/// Draw a bordered fixture card with an explicit background and title.
macro_rules! shell_card {
    ($surface:expr, $x:expr, $y:expr, $width:expr, $height:expr, $title:expr, $background:expr, $border:expr $(,)?) => {{
        let (x, y, width, height): (i32, i32, i32, i32) = ($x, $y, $width, $height);
        let title = $title;
        let background = $background;
        let border = $border;
        if width >= 4 && height >= 3 {
            fill($surface, x, y, width, height, background);
            ink!(
                $surface,
                x,
                y,
                width,
                &format!("╭─ {title} "),
                border,
                background,
                true
            );
            ink!(
                $surface,
                x,
                y.saturating_add(height).saturating_sub(1),
                width,
                "╰",
                border,
                background,
                false
            );
            if width > 1 {
                ink!(
                    $surface,
                    x.saturating_add(1),
                    y.saturating_add(height).saturating_sub(1),
                    width.saturating_sub(1),
                    &"─".repeat(allocation_width(width.saturating_sub(1))),
                    border,
                    background,
                    false
                );
            }
            for row in y.saturating_add(1)..y.saturating_add(height).saturating_sub(1) {
                ink!($surface, x, row, 1, "│", border, background, false);
                ink!(
                    $surface,
                    x.saturating_add(width).saturating_sub(1),
                    row,
                    1,
                    "│",
                    border,
                    background,
                    false
                );
            }
        }
    }};
}

/// Wrap explanation text at character boundaries, splitting oversized words.
fn wrap(line: &str, width: usize) -> Vec<String> {
    let mut rows = vec![String::new()];
    for word in line.split_whitespace() {
        let last = rows.len().saturating_sub(1);
        if !rows[last].is_empty()
            && rows[last]
                .chars()
                .count()
                .saturating_add(1)
                .saturating_add(word.chars().count())
                > width
        {
            rows.push(String::new());
        }
        let last = rows.len().saturating_sub(1);
        if !rows[last].is_empty() {
            rows[last].push(' ');
        }
        for ch in word.chars() {
            if rows.last().is_some_and(|row| row.chars().count() >= width) {
                rows.push(String::new());
            }
            let last = rows.len().saturating_sub(1);
            rows[last].push(ch);
        }
    }
    rows
}

impl Workspace {
    /// Mark input-driven focus feedback without starting a timer.
    fn start_motion(&mut self) {
        // Event-driven visual feedback only. A continuous frame loop was
        // measured to exceed the terminal's 1 MiB idle-output bound in under
        // 30 seconds, so it is rejected for this lightweight surface.
        self.motion = 6;
    }

    /// Return the focus glyph; never a live activity indicator.
    fn motion_mark(&self) -> &'static str {
        if self.motion > 0 { "▌" } else { "·" }
    }

    /// Render navigation targets using the same geometry as pointer selection.
    fn sidebar(&self, surface: &mut Surface, height: i32) {
        if height < 12 {
            return;
        }
        fill(surface, 0, 2, RAIL_WIDTH, height.saturating_sub(2), RAIL);
        for y in 2..height {
            ink!(
                surface,
                RAIL_WIDTH.saturating_sub(1),
                y,
                1,
                "│",
                SOFT,
                RAIL,
                false
            );
        }
        ink!(surface, 2, 4, 23, "WORKSPACE", SOFT, RAIL, true);
        ink!(surface, 2, 6, 23, "▣  Home", INK, RAIL, true);
        line(surface, 2, 8, 23, SOFT, RAIL);
        ink!(surface, 2, 10, 23, "WORK IN VIEW", SOFT, RAIL, true);
        for (index, _) in self.cards.iter().enumerate() {
            let y = rail_item_y(index);
            if y.saturating_add(RAIL_ITEM_HEIGHT) > height.saturating_sub(3) {
                break;
            }
            let active = self.selected == index;
            let background = if active { FOCUS_PANEL } else { RAIL };
            if active {
                fill(
                    surface,
                    RAIL_ITEM_X,
                    y,
                    RAIL_ITEM_WIDTH,
                    RAIL_ITEM_HEIGHT,
                    background,
                );
            }
            ink!(
                surface,
                RAIL_ITEM_X.saturating_add(1),
                y,
                RAIL_ITEM_WIDTH.saturating_sub(2),
                &format!(
                    "{} {:02}  {}",
                    if active { "▌" } else { " " },
                    index.saturating_add(1),
                    match index {
                        0 => "Review retry",
                        1 => "Workspace handoff",
                        2 => "Calmer project home",
                        _ => "Bounded input",
                    }
                ),
                if active { INK } else { SOFT },
                background,
                active,
            );
            ink!(
                surface,
                RAIL_ITEM_X.saturating_add(5),
                y.saturating_add(1),
                RAIL_ITEM_WIDTH.saturating_sub(6),
                if active { "click / Enter to open" } else { "" },
                if active { FOCUS } else { SOFT },
                background,
                false,
            );
        }
        if height >= 28 {
            line(surface, 2, height.saturating_sub(5), 23, SOFT, RAIL);
            ink!(
                surface,
                2,
                height.saturating_sub(3),
                23,
                "⊙ DEMO / disconnected",
                WARN,
                RAIL,
                true,
            );
            ink!(
                surface,
                2,
                height.saturating_sub(2),
                23,
                "click a work item · t truth",
                SOFT,
                RAIL,
                false,
            );
        }
    }

    /// Request continuation through the disconnected adapter, failing closed.
    fn continue_selected(&mut self) {
        let card = &self.cards[self.selected];
        let request =
            ContinueRequest::new(card.command, self.coordinator.board().scope(), card.run, 1);
        let result = self
            .coordinator
            .continue_work(&mut MissingMooAdapter::default(), request);
        self.cards[self.selected].dispatch = match result {
            Ok(ContinueOutcome::AwaitingDependency) => "Awaiting Moo · nothing started".into(),
            Ok(ContinueOutcome::Queued(_)) => "Owner queued · not completed".into(),
            Ok(_) => "Not continued: unsupported owner outcome".into(),
            Err(error) => format!("Not continued: {error}"),
        };
    }

    /// Open a session and request continuation without releasing a fixture hold.
    fn enter(&mut self) {
        self.entered = true;
        self.start_motion();
        self.continue_selected();
    }

    /// Dispatch explicit UI commands or retain an unsent session-only note.
    fn submit(&mut self) {
        let raw = self.cards[self.selected].session.input.trim();
        // A command keeps one leading slash with its body trimmed; a note stays as typed.
        let text = match raw.strip_prefix('/') {
            Some(body) => format!("/{}", body.trim()),
            None => raw.to_owned(),
        };
        self.cards[self.selected].session.input.clone_from(&text);
        match text.as_str() {
            "/home" => {
                self.cards[self.selected].session.input.clear();
                self.entered = false;
            }
            "/truth" | "/help" => {
                self.cards[self.selected].session.input.clear();
                self.truth = true;
                self.truth_scroll = 0;
            }
            "/continue" => {
                self.cards[self.selected].session.input.clear();
                self.continue_selected();
            }
            "/fixture-continue" => {
                let ui = &mut self.cards[self.selected].session;
                ui.input.clear();
                ui.apply("continue");
            }
            "/folders" | "/files" => {
                let ui = &mut self.cards[self.selected].session;
                ui.input.clear();
                ui.demo.folders = true;
                ui.focus = Focus::Files;
            }
            "/agents" => {
                let ui = &mut self.cards[self.selected].session;
                ui.input.clear();
                ui.demo.agents = true;
                ui.focus = Focus::Agents;
            }
            _ if text.split_whitespace().next() == Some("/continue") => {
                let ui = &mut self.cards[self.selected].session;
                ui.input.clear();
                ui.push_notice("Use /continue without arguments", ERR);
            }
            _ if text.starts_with('/') => {
                let ui = &mut self.cards[self.selected].session;
                ui.submit();
                if text.split_whitespace().next() == Some("/inspect") {
                    ui.focus = Focus::Changes;
                }
            }
            _ if !text.is_empty() => {
                let ui = &mut self.cards[self.selected].session;
                ui.input.clear();
                ui.user_messages = ui.user_messages.saturating_add(1);
                ui.push_row(
                    Row::clipping(vec![part(format!("You · {text}"), BASE, CharFlags::None)]),
                    true,
                );
                ui.push_row(Row::plain("Unsent · no model connected", MUTED), false);
                while ui.user_messages > 16 {
                    if let Some(start) = ui.transcript.iter().position(|row| row.starts_message) {
                        // The oldest message ends where the next begins, or at
                        // the end of the transcript when none follows.
                        let end = match ui
                            .transcript
                            .iter()
                            .enumerate()
                            .skip(start.saturating_add(1))
                            .find(|&(_, row)| row.starts_message)
                        {
                            Some((next, _)) => next,
                            None => ui.transcript.len(),
                        };
                        ui.transcript.drain(start..end);
                        ui.user_messages = ui.user_messages.saturating_sub(1);
                    } else {
                        break;
                    }
                }
                ui.scroll = 0;
            }
            _ => {}
        }
        self.cards[self.selected].session.command_mode = false;
    }
}

// Declared after `ink!` and `shell_card!`: a `macro_rules!` macro is in scope
// only for modules that follow its definition.
mod input;
mod views;

#[cfg(test)]
mod tests;
